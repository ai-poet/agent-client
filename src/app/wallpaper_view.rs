//! Responsive wallpaper painting and the composer-shaped alpha mask.
//!
//! Adapted from Zeron's `new_thread_background_mask.rs` (MIT). Source-space
//! rasterization is independent from this paint-time crop and feather.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{ImageAlphaMask, Point, RenderImage, size};

use super::*;
use crate::persistence::WallpaperAdjustment;

type SurfaceBounds = Rc<Cell<Option<Bounds<Pixels>>>>;
const COMPOSER_RADIUS: f32 = 26.0;
const CUTOUT_REVEAL_OPACITY: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
struct FittedGeometry {
    bounds: Bounds<Pixels>,
    overflow_x: f32,
    overflow_y: f32,
}

fn fitted_geometry(
    source_width: f32,
    source_height: f32,
    bounds: Bounds<Pixels>,
    adjustment: WallpaperAdjustment,
) -> Option<FittedGeometry> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 0.0
        || height <= 0.0
        || !width.is_finite()
        || !height.is_finite()
        || source_width <= 0.0
        || source_height <= 0.0
        || !source_width.is_finite()
        || !source_height.is_finite()
    {
        return None;
    }
    let adjustment = adjustment.normalized();
    let cover = (width / source_width).max(height / source_height);
    let fitted_width = (source_width * cover * adjustment.zoom).max(width);
    let fitted_height = (source_height * cover * adjustment.zoom).max(height);
    let overflow_x = (fitted_width - width).max(0.0);
    let overflow_y = (fitted_height - height).max(0.0);
    Some(FittedGeometry {
        bounds: Bounds::new(
            point(
                bounds.left() - px(overflow_x * adjustment.focal_x),
                bounds.top() - px(overflow_y * adjustment.focal_y),
            ),
            size(px(fitted_width), px(fitted_height)),
        ),
        overflow_x,
        overflow_y,
    })
}

fn source_geometry(
    source: &RenderImage,
    bounds: Bounds<Pixels>,
    adjustment: WallpaperAdjustment,
) -> Option<FittedGeometry> {
    let source_size = source.size(0);
    fitted_geometry(
        source_size.width.0 as f32,
        source_size.height.0 as f32,
        bounds,
        adjustment,
    )
}

/// Convert a preview drag into persistent, viewport-independent framing.
pub(super) fn pan_wallpaper_adjustment(
    source: &RenderImage,
    bounds: Bounds<Pixels>,
    adjustment: WallpaperAdjustment,
    delta: Point<Pixels>,
) -> WallpaperAdjustment {
    let adjustment = adjustment.normalized();
    let Some(geometry) = source_geometry(source, bounds, adjustment) else {
        return adjustment;
    };
    let mut next = adjustment;
    if geometry.overflow_x > 0.0 {
        next.focal_x -= f32::from(delta.x) / geometry.overflow_x;
    }
    if geometry.overflow_y > 0.0 {
        next.focal_y -= f32::from(delta.y) / geometry.overflow_y;
    }
    next.normalized()
}

fn mask(bounds: Bounds<Pixels>, composer: Option<Bounds<Pixels>>, cutout: bool) -> ImageAlphaMask {
    let height = f32::from(bounds.size.height);
    let has_cutout = cutout && composer.is_some();
    let cleared = composer.map(|composer| {
        Bounds::new(
            composer.origin,
            size(
                composer.size.width,
                composer.bottom().max(bounds.bottom()) - composer.top(),
            ),
        )
    });
    ImageAlphaMask {
        bounds: if has_cutout {
            cleared.unwrap_or(bounds)
        } else {
            Bounds::new(point(bounds.left(), bounds.bottom() + px(1.0)), bounds.size)
        },
        radius: if has_cutout {
            px(COMPOSER_RADIUS)
        } else {
            px(0.0)
        },
        feather: if has_cutout {
            px((height * 0.52).clamp(120.0, 280.0))
        } else {
            px(1.0)
        },
        clearance: if has_cutout { px(8.0) } else { px(0.0) },
        bottom_fade: Some((bounds.bottom(), px(height.max(1.0)))),
    }
}

fn paint_adjusted_with_mask(
    source: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
    adjustment: WallpaperAdjustment,
    mask: Option<ImageAlphaMask>,
    window: &mut Window,
) {
    let Some(fitted) = source_geometry(&source, bounds, adjustment) else {
        return;
    };
    let _ = window.paint_image_fitted_masked(
        bounds,
        fitted.bounds,
        Default::default(),
        source,
        0,
        false,
        mask,
    );
}

/// Appearance preview and the main hero deliberately share the exact crop.
pub(super) fn paint_wallpaper_preview(
    source: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
    adjustment: WallpaperAdjustment,
    window: &mut Window,
) {
    paint_adjusted_with_mask(source, bounds, adjustment, None, window);
}

fn artwork_pass(
    source: Arc<RenderImage>,
    adjustment: WallpaperAdjustment,
    composer_bounds: SurfaceBounds,
    cutout: bool,
) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            paint_adjusted_with_mask(
                source.clone(),
                bounds,
                adjustment,
                Some(mask(bounds, composer_bounds.get(), cutout)),
                window,
            );
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

fn artwork_layer(
    source: Arc<RenderImage>,
    adjustment: WallpaperAdjustment,
    composer_bounds: SurfaceBounds,
    opacity: f32,
) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .opacity(opacity)
        .child(
            div()
                .absolute()
                .inset_0()
                .opacity(CUTOUT_REVEAL_OPACITY)
                .child(artwork_pass(
                    source.clone(),
                    adjustment,
                    composer_bounds.clone(),
                    false,
                )),
        )
        .child(artwork_pass(source, adjustment, composer_bounds, true))
        .into_any_element()
}

impl Waku {
    pub(super) fn render_wallpaper_hero(
        &mut self,
        empty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let reduced_motion = cx.reduce_motion();
        let dissolve = self.wallpaper_session_dissolve(empty, reduced_motion)?;
        let frame = self.wallpaper_frame(reduced_motion);
        if !reduced_motion && (frame.active || !empty) {
            window.request_animation_frame();
        }
        let opacity = (1.0 - dissolve)
            * if crate::platform::window_glass_enabled() {
                0.84
            } else {
                1.0
            };
        let composer_bounds = if self.selected_project().is_some() {
            self.composer_autocomplete.card_bounds_cell()
        } else {
            Rc::new(Cell::new(None))
        };
        let mut layers = Vec::new();
        if let Some(source) = frame.previous {
            layers.push(artwork_layer(
                source,
                frame.previous_adjustment,
                composer_bounds.clone(),
                (1.0 - frame.mix) * opacity,
            ));
        }
        if let Some(source) = frame.current {
            layers.push(artwork_layer(
                source,
                frame.current_adjustment,
                composer_bounds,
                frame.mix * opacity,
            ));
        }
        if layers.is_empty() {
            return None;
        }
        let hero_height = (f32::from(window.viewport_size().height).max(0.0) * 0.72).min(760.0);
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(hero_height))
                .overflow_hidden()
                .children(layers)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop_honors_focal_point_and_zoom() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(100.0), px(100.0)));
        let fitted = fitted_geometry(
            200.0,
            100.0,
            bounds,
            WallpaperAdjustment {
                focal_x: 1.0,
                focal_y: 0.5,
                zoom: 2.0,
            },
        )
        .expect("valid geometry");
        assert_eq!(fitted.bounds.size.width, px(400.0));
        assert_eq!(fitted.bounds.size.height, px(200.0));
        assert_eq!(fitted.bounds.left(), px(-290.0));
        assert_eq!(fitted.bounds.top(), px(-30.0));
    }

    #[test]
    fn invalid_source_or_viewport_has_no_geometry() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(100.0)));
        assert!(fitted_geometry(0.0, 100.0, bounds, Default::default()).is_none());
        assert!(fitted_geometry(100.0, 100.0, Bounds::default(), Default::default()).is_none());
    }
}
