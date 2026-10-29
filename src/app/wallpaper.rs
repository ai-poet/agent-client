//! New-task wallpaper: managed files, bounded decoded artwork and transitions.
//!
//! Adapted from Zeron's new-thread background settings and effects (MIT,
//! Copyright (c) 2026 Wing). Decoding and raster effects never run on the UI
//! thread; render only receives an already prepared `RenderImage`.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::{Bounds, Context, FocusHandle, Hsla, KeyDownEvent, Pixels, Point, RenderImage, Rgba};
use image::{Rgba as Pixel, RgbaImage};
use uuid::Uuid;

use super::Waku;
use crate::persistence::{
    WallpaperAdjustment, WallpaperEffect, WallpaperSettings, wallpaper_directory,
};
use crate::theme::Theme;

const SOURCE_CACHE_LIMIT: usize = 4;
const CROSSFADE_DURATION: Duration = Duration::from_millis(220);

#[derive(Default)]
pub(super) struct WallpaperState {
    generation: u64,
    pub loading: bool,
    pub error: Option<String>,
    readiness: Readiness,
    pub adjustment_dialog: Option<WallpaperAdjustmentDialog>,
    pub adjustment_preview_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    tint: Option<Hsla>,
    was_empty: bool,
    departing_started: Option<Instant>,
}

pub(super) struct WallpaperAdjustmentDialog {
    pub draft: WallpaperAdjustment,
    pub focus: FocusHandle,
    drag: Option<Point<Pixels>>,
}

impl WallpaperState {
    /// `None` means the hero is gone; otherwise this is its departure progress.
    fn session_dissolve(&mut self, empty: bool, reduced_motion: bool, now: Instant) -> Option<f32> {
        if empty {
            self.was_empty = true;
            self.departing_started = None;
            return Some(0.0);
        }
        if reduced_motion {
            self.was_empty = false;
            self.departing_started = None;
            return None;
        }
        if self.was_empty {
            self.departing_started = Some(now);
        }
        self.was_empty = false;
        let started = self.departing_started?;
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / CROSSFADE_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        if progress >= 1.0 {
            self.departing_started = None;
            None
        } else {
            Some(progress)
        }
    }
}

#[derive(Default)]
struct Readiness {
    current: Option<Arc<RenderImage>>,
    previous: Option<Arc<RenderImage>>,
    current_path: Option<PathBuf>,
    previous_path: Option<PathBuf>,
    current_adjustment: WallpaperAdjustment,
    previous_adjustment: WallpaperAdjustment,
    started: Option<Instant>,
}

pub(super) struct ArtworkFrame {
    pub current: Option<Arc<RenderImage>>,
    pub previous: Option<Arc<RenderImage>>,
    pub current_adjustment: WallpaperAdjustment,
    pub previous_adjustment: WallpaperAdjustment,
    pub mix: f32,
    pub active: bool,
}

impl Readiness {
    fn adopt(
        &mut self,
        image: Option<Arc<RenderImage>>,
        path: Option<PathBuf>,
        adjustment: WallpaperAdjustment,
        reduced_motion: bool,
    ) {
        self.update_adjustment(adjustment, path.as_deref());
        let changed =
            self.current.as_ref().map(|image| image.id) != image.as_ref().map(|image| image.id);
        if !changed {
            self.current_path = path;
            self.current_adjustment = adjustment.normalized();
            return;
        }

        self.previous = self.current.take();
        self.previous_path = self.current_path.take();
        self.previous_adjustment = self.current_adjustment;
        self.current = image;
        self.current_path = path;
        self.current_adjustment = adjustment.normalized();
        if reduced_motion {
            self.previous = None;
            self.previous_path = None;
            self.started = None;
        } else {
            self.started = Some(Instant::now());
        }
    }

    fn update_adjustment(&mut self, adjustment: WallpaperAdjustment, path: Option<&Path>) {
        if path.is_some() && path == self.current_path.as_deref() {
            self.current_adjustment = adjustment.normalized();
        }
        if path.is_some() && path == self.previous_path.as_deref() {
            self.previous_adjustment = adjustment.normalized();
        }
    }

    fn frame(&mut self, reduced_motion: bool, now: Instant) -> ArtworkFrame {
        let progress = self.started.map_or(1.0, |started| {
            (now.saturating_duration_since(started).as_secs_f32()
                / CROSSFADE_DURATION.as_secs_f32())
            .clamp(0.0, 1.0)
        });
        if reduced_motion || progress >= 1.0 {
            self.previous = None;
            self.previous_path = None;
            self.started = None;
        }
        ArtworkFrame {
            current: self.current.clone(),
            previous: self.previous.clone(),
            current_adjustment: self.current_adjustment,
            previous_adjustment: self.previous_adjustment,
            mix: self.started.map_or(1.0, |started| {
                (now.saturating_duration_since(started).as_secs_f32()
                    / CROSSFADE_DURATION.as_secs_f32())
                .clamp(0.0, 1.0)
            }),
            active: self.started.is_some(),
        }
    }
}

struct PreparedArtwork {
    image: Arc<RenderImage>,
    color: Option<Hsla>,
}

struct WallpaperSource {
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
    colors: Box<[[u8; 4]]>,
    effects: Mutex<Vec<((WallpaperEffect, bool), Arc<RenderImage>)>>,
}

static SOURCE_CACHE: OnceLock<Mutex<Vec<(PathBuf, Arc<WallpaperSource>)>>> = OnceLock::new();

fn effect_key(effect: WallpaperEffect, light: bool) -> (WallpaperEffect, bool) {
    (
        effect,
        light && !matches!(effect, WallpaperEffect::None | WallpaperEffect::Dither),
    )
}

fn cached_source(path: &Path) -> Option<Arc<WallpaperSource>> {
    SOURCE_CACHE
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .ok()?
        .iter()
        .find(|(cached, _)| cached == path)
        .map(|(_, source)| source.clone())
}

fn install_source(path: &Path, source: Arc<WallpaperSource>) {
    let Ok(mut cache) = SOURCE_CACHE.get_or_init(|| Mutex::new(Vec::new())).lock() else {
        return;
    };
    cache.retain(|(cached, _)| cached != path);
    cache.push((path.to_path_buf(), source));
    if cache.len() > SOURCE_CACHE_LIMIT {
        cache.remove(0);
    }
}

fn cached_artwork(
    path: &Path,
    effect: WallpaperEffect,
    light: bool,
) -> Option<(Arc<RenderImage>, Option<Hsla>)> {
    let source = cached_source(path)?;
    let key = effect_key(effect, light);
    let image = source
        .effects
        .lock()
        .ok()?
        .iter()
        .find(|(cached, _)| *cached == key)
        .map(|(_, image)| image.clone())?;
    Some((image, source.dominant_color()))
}

fn prepare_artwork(
    path: &Path,
    effect: WallpaperEffect,
    light: bool,
) -> Result<PreparedArtwork, String> {
    let source = match cached_source(path) {
        Some(source) => source,
        None => {
            let bytes =
                std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
            let source = decode_source(&bytes)?;
            install_source(path, source.clone());
            source
        }
    };
    let key = effect_key(effect, light);
    if let Some(image) = source.effects.lock().ok().and_then(|effects| {
        effects
            .iter()
            .find(|(cached, _)| *cached == key)
            .map(|(_, image)| image.clone())
    }) {
        return Ok(PreparedArtwork {
            image,
            color: source.dominant_color(),
        });
    }
    let image = source.render_image(effect, light);
    if let Ok(mut effects) = source.effects.lock() {
        effects.retain(|(cached, _)| *cached != key);
        effects.push((key, image.clone()));
    }
    Ok(PreparedArtwork {
        image,
        color: source.dominant_color(),
    })
}

fn decode_source(bytes: &[u8]) -> Result<Arc<WallpaperSource>, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("unsupported or damaged image: {error}"))?;
    let proxy = image.thumbnail(2048, 2048);
    let gray = proxy.to_luma8();
    Ok(Arc::new(WallpaperSource {
        width: gray.width(),
        height: gray.height(),
        pixels: gray.into_raw().into_boxed_slice(),
        colors: proxy.to_rgba8().pixels().map(|pixel| pixel.0).collect(),
        effects: Mutex::new(Vec::new()),
    }))
}

impl WallpaperSource {
    fn render_image(&self, effect: WallpaperEffect, light: bool) -> Arc<RenderImage> {
        let pixels = match effect {
            WallpaperEffect::None => RgbaImage::from_fn(self.width, self.height, |x, y| {
                let [r, g, b, a] = self.colors[(y * self.width + x) as usize];
                Pixel([b, g, r, a])
            }),
            WallpaperEffect::Dither => self.dither_pixels(),
            WallpaperEffect::Ascii => self.ascii_pixels(light),
            WallpaperEffect::Halftone => self.halftone_pixels(light),
            WallpaperEffect::Scanlines => self.scanline_pixels(light),
        };
        Arc::new(RenderImage::new([image::Frame::new(pixels)]))
    }

    fn scanline_pixels(&self, light: bool) -> RgbaImage {
        RgbaImage::from_fn(self.width, self.height, |x, y| {
            let [r, g, b, a] = self.colors[(y * self.width + x) as usize];
            let gain = if y % 3 == 0 { 0.52 } else { 1.0 };
            let channel = |value: u8| {
                if light {
                    (value as f32 + (255.0 - value as f32) * (1.0 - gain)) as u8
                } else {
                    (value as f32 * gain) as u8
                }
            };
            Pixel([channel(b), channel(g), channel(r), a])
        })
    }

    fn ascii_pixels(&self, light: bool) -> RgbaImage {
        // Artwork pixels, not thousands of independently shaped text runs.
        const GLYPHS: [[u8; 7]; 10] = [
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 4, 0],
            [0, 4, 0, 0, 4, 0, 0],
            [0, 0, 14, 0, 0, 0, 0],
            [0, 0, 14, 0, 14, 0, 0],
            [0, 4, 4, 31, 4, 4, 0],
            [0, 21, 14, 31, 14, 21, 0],
            [10, 10, 31, 10, 31, 10, 10],
            [17, 2, 4, 4, 8, 16, 17],
            [14, 17, 23, 21, 23, 16, 14],
        ];
        RgbaImage::from_fn(self.width, self.height, |x, y| {
            let sx = (x / 6 * 6 + 3).min(self.width - 1);
            let sy = (y / 8 * 8 + 4).min(self.height - 1);
            let sample = (sy * self.width + sx) as usize;
            let ink_density = if light {
                255 - self.pixels[sample]
            } else {
                self.pixels[sample]
            };
            let index = ((ink_density as f32 / 255.0).sqrt() * 9.0) as usize;
            let ink =
                x % 6 < 5 && y % 8 < 7 && GLYPHS[index][y as usize % 8] & (1 << (4 - x % 6)) != 0;
            let [r, g, b, a] = self.colors[(y * self.width + x) as usize];
            let [cr, cg, cb, _] = self.colors[sample];
            let mix = |base: u8, glyph: u8| {
                let paper = if light { 255.0 } else { 0.0 };
                (base as f32 * 0.60
                    + if ink {
                        glyph as f32 * 0.40
                    } else {
                        paper * 0.40
                    }) as u8
            };
            Pixel([mix(b, cb), mix(g, cg), mix(r, cr), a])
        })
    }

    fn halftone_pixels(&self, light: bool) -> RgbaImage {
        let paper = if light { 255 } else { 0 };
        let (width, height) = (self.width, self.height);
        let mut pixels = RgbaImage::from_pixel(width, height, Pixel([paper, paper, paper, 255]));
        for y in (0..height).step_by(4) {
            for x in (0..width).step_by(4) {
                let luma = self.sample_cover(x as f32, y as f32);
                let luma = if light { 255 - luma } else { luma };
                let radius = 2.0 * (0.3 + 0.7 * (luma as f32 / 255.0).sqrt());
                let [r, g, b, a] = self.colors[self.cover_index(x as f32 + 2.0, y as f32 + 2.0)];
                for dy in 0..4.min(height - y) {
                    for dx in 0..4.min(width - x) {
                        let distance =
                            ((dx as f32 - 1.5).powi(2) + (dy as f32 - 1.5).powi(2)).sqrt();
                        let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0) * a as f32 / 255.0;
                        let [sr, sg, sb, sa] = self.colors[((y + dy) * width + x + dx) as usize];
                        let blend = |source: u8, dot: u8| {
                            (source as f32 * 0.60
                                + (dot as f32 * coverage + paper as f32 * (1.0 - coverage)) * 0.40)
                                as u8
                        };
                        pixels.put_pixel(
                            x + dx,
                            y + dy,
                            Pixel([blend(sb, b), blend(sg, g), blend(sr, r), sa]),
                        );
                    }
                }
            }
        }
        pixels
    }

    fn dither_pixels(&self) -> RgbaImage {
        const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        let (width, height) = (self.width, self.height);
        let mut pixels = RgbaImage::new(width, height);
        for y in (0..height).step_by(2) {
            for x in (0..width).step_by(2) {
                let index = self.cover_index((x + 1) as f32, (y + 1) as f32);
                let [r, g, b, a] = dither_color(
                    self.colors[index],
                    BAYER[y as usize / 2 % 4][x as usize / 2 % 4],
                );
                for dy in 0..2.min(height - y) {
                    for dx in 0..2.min(width - x) {
                        pixels.put_pixel(x + dx, y + dy, Pixel([b, g, r, a]));
                    }
                }
            }
        }
        pixels
    }

    fn sample_cover(&self, x: f32, y: f32) -> u8 {
        self.pixels[self.cover_index(x, y)]
    }

    fn cover_index(&self, x: f32, y: f32) -> usize {
        let scale =
            (self.width as f32 / self.width as f32).max(self.height as f32 / self.height as f32);
        let visible_width = self.width as f32 / scale;
        let visible_height = self.height as f32 / scale;
        let source_x = ((self.width as f32 - visible_width) * 0.5 + x / scale)
            .clamp(0.0, self.width as f32 - 1.0) as u32;
        let source_y = ((self.height as f32 - visible_height) * 0.5 + y / scale)
            .clamp(0.0, self.height as f32 - 1.0) as u32;
        (source_y * self.width + source_x) as usize
    }

    /// Quantized dominant colour, favouring chromatic regions over neutrals.
    fn dominant_color(&self) -> Option<Hsla> {
        let stride = (self.colors.len() / 4096).max(1);
        let mut bins = vec![(0.0_f64, [0.0_f64; 3]); 4096];
        for [r, g, b, a] in self.colors.iter().step_by(stride).copied() {
            if a < 128 {
                continue;
            }
            let high = r.max(g).max(b) as f64;
            let low = r.min(g).min(b) as f64;
            let saturation = (high - low) / high.max(1.0);
            let weight = (0.2 + saturation * saturation) * a as f64 / 255.0;
            let index = ((r as usize >> 4) << 8) | ((g as usize >> 4) << 4) | (b as usize >> 4);
            let (count, channels) = &mut bins[index];
            *count += weight;
            for (sum, channel) in channels.iter_mut().zip([r, g, b]) {
                *sum += channel as f64 * weight;
            }
        }
        let (count, channels) = bins.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
        (count > 0.0).then(|| {
            Rgba {
                r: (channels[0] / count).round() as f32 / 255.0,
                g: (channels[1] / count).round() as f32 / 255.0,
                b: (channels[2] / count).round() as f32 / 255.0,
                a: 1.0,
            }
            .into()
        })
    }
}

fn dither_color([r, g, b, a]: [u8; 4], threshold: u8) -> [u8; 4] {
    let peak = r.max(g).max(b) as f32;
    let bright = peak / 255.0 > (threshold as f32 + 0.5) / 16.0;
    let gain = if bright { 255.0 / peak.max(1.0) } else { 0.08 };
    [
        (r as f32 * gain).round() as u8,
        (g as f32 * gain).round() as u8,
        (b as f32 * gain).round() as u8,
        a,
    ]
}

fn install_wallpaper_file(
    source: &Path,
    effect: WallpaperEffect,
    theme_colors: bool,
) -> Result<WallpaperSettings, String> {
    let bytes = std::fs::read(source).map_err(|error| format!("{}: {error}", source.display()))?;
    image::load_from_memory(&bytes)
        .map_err(|error| format!("unsupported or damaged image: {error}"))?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), "png" | "jpg" | "jpeg"))
        .unwrap_or_else(|| "png".to_owned());
    let directory = wallpaper_directory();
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let destination = directory.join(format!("wallpaper-{}.{}", Uuid::new_v4(), extension));
    let temporary = destination.with_extension(format!("{extension}.tmp"));
    std::fs::write(&temporary, &bytes)
        .and_then(|_| std::fs::rename(&temporary, &destination))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("{}: {error}", destination.display())
        })?;
    Ok(WallpaperSettings {
        path: destination,
        name: source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("wallpaper")
            .to_owned(),
        adjustment: WallpaperAdjustment::default(),
        effect,
        theme_colors,
    })
}

fn remove_managed_file(path: &Path) {
    if path.parent() == Some(wallpaper_directory().as_path()) {
        let _ = std::fs::remove_file(path);
    }
}

impl Waku {
    pub(super) fn prepare_wallpaper(&mut self, cx: &mut Context<Self>) {
        self.wallpaper.generation = self.wallpaper.generation.wrapping_add(1).max(1);
        let generation = self.wallpaper.generation;
        let reduced_motion = cx.reduce_motion();
        let Some(settings) = self.state.wallpaper.clone() else {
            self.wallpaper.loading = false;
            self.wallpaper.error = None;
            self.wallpaper.tint = None;
            crate::theme::set_wallpaper_tint(None, cx);
            self.wallpaper.readiness.adopt(
                None,
                None,
                WallpaperAdjustment::default(),
                reduced_motion,
            );
            return;
        };
        let light = !Theme::current(cx).is_dark;
        if let Some((image, color)) = cached_artwork(&settings.path, settings.effect, light) {
            self.wallpaper.loading = false;
            self.wallpaper.error = None;
            self.wallpaper.tint = color;
            crate::theme::set_wallpaper_tint(settings.theme_colors.then_some(color).flatten(), cx);
            self.wallpaper.readiness.adopt(
                Some(image),
                Some(settings.path),
                settings.adjustment,
                reduced_motion,
            );
            return;
        }

        self.wallpaper.loading = true;
        self.wallpaper.error = None;
        let path = settings.path.clone();
        let effect = settings.effect;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let worker_path = path.clone();
            let result = executor
                .spawn(async move { prepare_artwork(&worker_path, effect, light) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.wallpaper.generation {
                    return;
                }
                this.wallpaper.loading = false;
                match result {
                    Ok(prepared) => {
                        this.wallpaper.tint = prepared.color;
                        let tint = this
                            .state
                            .wallpaper
                            .as_ref()
                            .filter(|settings| settings.theme_colors)
                            .and_then(|_| prepared.color);
                        crate::theme::set_wallpaper_tint(tint, cx);
                        this.wallpaper.readiness.adopt(
                            Some(prepared.image),
                            Some(path),
                            settings.adjustment,
                            reduced_motion,
                        );
                    }
                    Err(error) => this.wallpaper.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn pick_wallpaper(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!("wallpaper.choose").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.first()
            {
                let _ = this.update(cx, |this, cx| this.install_wallpaper(path.clone(), cx));
            }
        })
        .detach();
    }

    fn install_wallpaper(&mut self, source: PathBuf, cx: &mut Context<Self>) {
        let (effect, theme_colors) = self
            .state
            .wallpaper
            .as_ref()
            .map(|settings| (settings.effect, settings.theme_colors))
            .unwrap_or_default();
        self.wallpaper.loading = true;
        self.wallpaper.error = None;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = executor
                .spawn(async move { install_wallpaper_file(&source, effect, theme_colors) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.wallpaper.loading = false;
                match result {
                    Ok(settings) => {
                        let new_path = settings.path.clone();
                        let previous = this.state.wallpaper.replace(settings);
                        if let Err(error) = this.store.save(&mut this.state) {
                            this.state.wallpaper = previous;
                            remove_managed_file(&new_path);
                            this.wallpaper.error = Some(error.to_string());
                        } else {
                            if let Some(previous) = previous {
                                remove_managed_file(&previous.path);
                            }
                            this.prepare_wallpaper(cx);
                        }
                    }
                    Err(error) => this.wallpaper.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn remove_wallpaper(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.state.wallpaper.take() else {
            return;
        };
        if let Err(error) = self.store.save(&mut self.state) {
            self.state.wallpaper = Some(previous);
            self.wallpaper.error = Some(error.to_string());
            cx.notify();
            return;
        }
        remove_managed_file(&previous.path);
        self.prepare_wallpaper(cx);
        cx.notify();
    }

    pub(super) fn set_wallpaper_effect(&mut self, effect: WallpaperEffect, cx: &mut Context<Self>) {
        let Some(settings) = self.state.wallpaper.as_mut() else {
            return;
        };
        if settings.effect == effect {
            return;
        }
        settings.effect = effect;
        self.save();
        self.prepare_wallpaper(cx);
        cx.notify();
    }

    pub(super) fn set_wallpaper_theme_colors(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(settings) = self.state.wallpaper.as_mut() else {
            return;
        };
        settings.theme_colors = enabled;
        self.save();
        crate::theme::set_wallpaper_tint(enabled.then_some(self.wallpaper.tint).flatten(), cx);
        cx.notify();
    }

    pub(super) fn open_wallpaper_adjustment(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.state.wallpaper.as_ref() else {
            return;
        };
        self.wallpaper.adjustment_dialog = Some(WallpaperAdjustmentDialog {
            draft: settings.adjustment.normalized(),
            focus: cx.focus_handle(),
            drag: None,
        });
        cx.notify();
    }

    pub(super) fn cancel_wallpaper_adjustment(&mut self, cx: &mut Context<Self>) {
        self.wallpaper.adjustment_dialog = None;
        cx.notify();
    }

    pub(super) fn reset_wallpaper_adjustment(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.wallpaper.adjustment_dialog.as_mut() {
            dialog.draft = WallpaperAdjustment::default();
        }
        cx.notify();
    }

    pub(super) fn zoom_wallpaper_adjustment(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(dialog) = self.wallpaper.adjustment_dialog.as_mut() {
            dialog.draft.zoom = (dialog.draft.zoom + delta)
                .clamp(WallpaperAdjustment::MIN_ZOOM, WallpaperAdjustment::MAX_ZOOM);
        }
        cx.notify();
    }

    pub(super) fn begin_wallpaper_adjustment_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.wallpaper.adjustment_dialog.as_mut() else {
            return;
        };
        window.focus(&dialog.focus, cx);
        dialog.drag = Some(position);
        cx.stop_propagation();
    }

    pub(super) fn move_wallpaper_adjustment_drag(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let source = self.wallpaper_preview();
        let bounds = self.wallpaper.adjustment_preview_bounds.get();
        let Some(dialog) = self.wallpaper.adjustment_dialog.as_mut() else {
            return;
        };
        let Some(previous) = dialog.drag else {
            return;
        };
        let delta = gpui::point(position.x - previous.x, position.y - previous.y);
        dialog.drag = Some(position);
        if let (Some(source), Some(bounds)) = (source, bounds) {
            dialog.draft = super::wallpaper_view::pan_wallpaper_adjustment(
                &source,
                bounds,
                dialog.draft,
                delta,
            );
        }
        cx.notify();
    }

    pub(super) fn end_wallpaper_adjustment_drag(&mut self) {
        if let Some(dialog) = self.wallpaper.adjustment_dialog.as_mut() {
            dialog.drag = None;
        }
    }

    pub(super) fn key_wallpaper_adjustment(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.cancel_wallpaper_adjustment(cx);
                cx.stop_propagation();
                return;
            }
            "enter" => {
                self.apply_wallpaper_adjustment(cx);
                cx.stop_propagation();
                return;
            }
            "+" | "=" => {
                self.zoom_wallpaper_adjustment(0.1, cx);
                cx.stop_propagation();
                return;
            }
            "-" => {
                self.zoom_wallpaper_adjustment(-0.1, cx);
                cx.stop_propagation();
                return;
            }
            _ => {}
        }
        let Some(dialog) = self.wallpaper.adjustment_dialog.as_mut() else {
            return;
        };
        let step = if event.keystroke.modifiers.shift {
            0.1
        } else {
            0.02
        };
        match key {
            "left" => dialog.draft.focal_x -= step,
            "right" => dialog.draft.focal_x += step,
            "up" => dialog.draft.focal_y -= step,
            "down" => dialog.draft.focal_y += step,
            _ => return,
        }
        dialog.draft = dialog.draft.normalized();
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn apply_wallpaper_adjustment(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.wallpaper.adjustment_dialog.take() else {
            return;
        };
        let adjustment = dialog.draft.normalized();
        let Some(settings) = self.state.wallpaper.as_mut() else {
            return;
        };
        settings.adjustment = adjustment;
        let path = settings.path.clone();
        self.wallpaper
            .readiness
            .update_adjustment(adjustment, Some(&path));
        self.save();
        cx.notify();
    }

    pub(super) fn wallpaper_frame(&mut self, reduced_motion: bool) -> ArtworkFrame {
        self.wallpaper
            .readiness
            .frame(reduced_motion, Instant::now())
    }

    pub(super) fn wallpaper_session_dissolve(
        &mut self,
        empty: bool,
        reduced_motion: bool,
    ) -> Option<f32> {
        self.wallpaper
            .session_dissolve(empty, reduced_motion, Instant::now())
    }

    pub(super) fn wallpaper_preview(&self) -> Option<Arc<RenderImage>> {
        self.wallpaper.readiness.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_cache_only_separates_themes_for_effects_that_need_it() {
        assert_eq!(effect_key(WallpaperEffect::None, true).1, false);
        assert_eq!(effect_key(WallpaperEffect::Dither, true).1, false);
        assert_eq!(effect_key(WallpaperEffect::Ascii, true).1, true);
        assert_eq!(effect_key(WallpaperEffect::Ascii, false).1, false);
    }

    #[test]
    fn every_effect_rasterizes_a_small_source() {
        let source = Arc::new(WallpaperSource {
            width: 8,
            height: 8,
            pixels: (0..64)
                .map(|value| value * 4)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            colors: (0..64)
                .map(|value| [value * 4, 80, 160, 255])
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            effects: Mutex::new(Vec::new()),
        });
        for effect in WallpaperEffect::ALL {
            let image = source.render_image(effect, false);
            assert_eq!(image.size(0).width.0, 8);
            assert_eq!(image.size(0).height.0, 8);
        }
    }
}
