use gpui::{
    App, Global, Hsla, Rems, Rgba, Window, WindowAppearance, hsla, rems, rgb, transparent_black,
};

pub use waku_client::theme::ThemePreference;

/// Scaled pixels: a dimension authored at the default 14px UI font size,
/// expressed in rems so the UI font size setting scales it. The window's rem
/// size *is* the UI font size, so at the default setting this resolves to
/// exactly the authored pixel value.
///
/// Chrome text sizes and their line heights go through here. Content surfaces
/// that already derive from a font-size setting — markdown metrics, the file
/// editor, diff rows, tool-output mono — stay in `px` so they never scale
/// twice.
pub fn sp(value: f32) -> Rems {
    rems(value / waku_client::persistence::DEFAULT_UI_FONT_SIZE)
}

fn resolves_to_dark(preference: ThemePreference, system_appearance: WindowAppearance) -> bool {
    match preference {
        ThemePreference::System => matches!(
            system_appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
        ThemePreference::Light => false,
        ThemePreference::Dark => true,
    }
}

fn native_override(preference: ThemePreference) -> Option<bool> {
    match preference {
        ThemePreference::System => None,
        ThemePreference::Light => Some(false),
        ThemePreference::Dark => Some(true),
    }
}

/// Waku's visual language, take two: neutral graphite surfaces in the spirit
/// of Cursor — color is reserved for meaning. On platforms with a native
/// backdrop, the window and sidebar use contrast-checked translucent tints;
/// selected, hovered, and pressed rows remain a 6% neutral layer.
#[derive(Clone, Copy)]
pub struct Theme {
    pub is_dark: bool,
    pub canvas: Hsla,
    pub sidebar: Hsla,
    pub sidebar_item_background: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub composer: Hsla,
    pub inset: Hsla,
    /// Terminal screen surface: paper-white in light mode, near-black in dark.
    pub terminal: Hsla,
    pub overlay: Hsla,
    pub overlay_strong: Hsla,

    pub border: Hsla,
    pub border_strong: Hsla,
    pub sidebar_border: Hsla,

    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub text_ghost: Hsla,

    /// Brand coral. Logo, caret, live-activity pulses — nothing structural.
    pub accent: Hsla,
    pub resize_handle: Hsla,
    /// Meter fills in the usage panel. Quota-meter blue by convention;
    /// warning/danger take over as a lane fills.
    pub gauge: Hsla,

    /// Text-selection wash. Painted *under* the glyphs, so it stays
    /// translucent and deliberately reads as the familiar browser blue rather
    /// than as brand color.
    pub selection: Hsla,
    /// Inline `code` foreground and its rounded wash.
    pub code_text: Hsla,
    pub code_wash: Hsla,

    /// Light fill for primary buttons (send, allow), dark glyph on top.
    pub inverse: Hsla,
    pub on_inverse: Hsla,

    pub warning: Hsla,
    pub success: Hsla,
    pub favorite: Hsla,
    pub danger: Hsla,
    pub danger_soft: Hsla,
    /// Optional wallpaper-derived surface tint. Brand and semantic colors
    /// deliberately stay outside this channel.
    wallpaper_tint: Option<Hsla>,
}

impl Theme {
    pub fn current(cx: &App) -> Self {
        if cx.has_global::<ActiveWakuTheme>() {
            cx.global::<ActiveWakuTheme>().0
        } else {
            Self::dark()
        }
    }

    /// Theme-owned tint over the native desktop material. The contrast guard
    /// follows Zeron's glass recipe: an unknown bright/dark desktop may make
    /// the glass denser, never the text fainter.
    pub fn window_surface(&self) -> Hsla {
        let surface = self.wallpaper_tinted(self.surface);
        if !crate::platform::window_glass_enabled() {
            return surface;
        }
        self.glass_tint(surface, 0.80)
    }

    /// The sidebar is a sibling of the main surface, not another translucent
    /// quad on top of it, so their alpha never multiplies.
    pub fn sidebar_surface(&self) -> Hsla {
        let tint = if self.sidebar.is_transparent() {
            self.canvas
        } else {
            self.sidebar
        };
        let tint = self.wallpaper_tinted(tint);
        if !crate::platform::window_glass_enabled() {
            return tint;
        }
        self.glass_tint(tint, 0.72)
    }

    /// Floating input surfaces blur the scene inside the window on every
    /// desktop renderer, independently of the native window material.
    pub fn composer_surface(&self) -> Hsla {
        let composer = self.wallpaper_tinted(self.composer);
        if !cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            return composer;
        }
        let base = if self.is_dark { 0.72 } else { 0.80 };
        composer.opacity(self.contrast_checked_tint_alpha(composer, base, self.adverse_backdrop()))
    }

    pub fn composer_border(&self) -> Hsla {
        if cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            if self.is_dark {
                hsla(210.0 / 360.0, 0.18, 0.78, 0.09)
            } else {
                hsla(210.0 / 360.0, 0.18, 0.32, 0.10)
            }
        } else {
            self.border
        }
    }

    fn glass_tint(&self, tint: Hsla, base: f32) -> Hsla {
        tint.opacity(self.contrast_checked_tint_alpha(tint, base, self.adverse_backdrop()))
    }

    fn contrast_checked_tint_alpha(&self, tint: Hsla, base: f32, backdrop: Hsla) -> f32 {
        for step in 0..=20 {
            let alpha = base + (1.0 - base) * step as f32 / 20.0;
            let composite = flatten(tint.opacity(alpha), backdrop);
            if painted_contrast(self.text, composite) >= 4.5
                && painted_contrast(self.text_secondary, composite) >= 3.0
            {
                return alpha;
            }
        }
        1.0
    }

    fn adverse_backdrop(&self) -> Hsla {
        rgb(if self.is_dark { 0xFFFFFF } else { 0x000000 }).into()
    }

    fn wallpaper_tinted(&self, surface: Hsla) -> Hsla {
        let Some(color) = self.wallpaper_tint else {
            return surface;
        };
        let neutral = rgb(if self.is_dark { 0x000000 } else { 0xFFFFFF }).into();
        let tint = mix(color, neutral, if self.is_dark { 0.88 } else { 0.94 });
        mix(surface, tint, 0.4)
    }

    pub fn dark() -> Self {
        Self {
            is_dark: true,
            canvas: rgb(0x1A1A1A).into(),
            sidebar: if cfg!(target_os = "macos") {
                transparent_black()
            } else {
                rgb(0x181818).into()
            },
            sidebar_item_background: hsla(0.0, 0.0, 0.941, 0.06),
            surface: rgb(0x1A1A1A).into(),
            raised: rgb(0x232323).into(),
            composer: rgb(0x212121).into(),
            inset: rgb(0x151515).into(),
            terminal: rgb(0x151515).into(),
            overlay: hsla(220.0 / 360.0, 0.10, 0.90, 0.05),
            overlay_strong: hsla(220.0 / 360.0, 0.10, 0.90, 0.09),

            border: hsla(220.0 / 360.0, 0.10, 0.90, 0.07),
            border_strong: hsla(220.0 / 360.0, 0.10, 0.90, 0.14),
            sidebar_border: hsla(126.93 / 360.0, 0.000_000_1, 0.16077, 1.0),

            text: rgb(0xE2E2E2).into(),
            text_secondary: rgb(0xA3A3A3).into(),
            text_tertiary: rgb(0x7D7D7D).into(),
            text_ghost: rgb(0x575757).into(),

            accent: rgb(0xE2795B).into(),
            resize_handle: rgb(0x3B82F6).into(),
            gauge: rgb(0x3B82F6).into(),

            selection: hsla(211.0 / 360.0, 1.0, 0.50, 0.55),
            code_text: rgb(0xE0A882).into(),
            code_wash: hsla(220.0 / 360.0, 0.10, 0.90, 0.08),

            inverse: rgb(0xE7E9EC).into(),
            on_inverse: rgb(0x17181C).into(),

            warning: rgb(0xE0B36A).into(),
            success: rgb(0x62C987).into(),
            favorite: rgb(0xEAB308).into(),
            danger: rgb(0xE2726A).into(),
            danger_soft: hsla(4.0 / 360.0, 0.55, 0.63, 0.10),
            wallpaper_tint: None,
        }
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,
            canvas: rgb(0xF6F5F6).into(),
            sidebar: if cfg!(target_os = "macos") {
                transparent_black()
            } else {
                rgb(0xF3F3F3).into()
            },
            sidebar_item_background: hsla(0.0, 0.0, 0.078, 0.06),
            surface: rgb(0xF6F5F6).into(),
            raised: rgb(0xECECEC).into(),
            composer: rgb(0xFFFFFF).into(),
            inset: rgb(0xE6E6E6).into(),
            terminal: rgb(0xFFFFFF).into(),
            overlay: hsla(220.0 / 360.0, 0.10, 0.12, 0.05),
            overlay_strong: hsla(220.0 / 360.0, 0.10, 0.12, 0.09),

            border: hsla(220.0 / 360.0, 0.10, 0.12, 0.08),
            border_strong: hsla(220.0 / 360.0, 0.10, 0.12, 0.15),
            sidebar_border: hsla(0.0, 0.0, 0.078, 0.12),

            text: rgb(0x242424).into(),
            text_secondary: rgb(0x666666).into(),
            text_tertiary: rgb(0x858585).into(),
            text_ghost: rgb(0xA4A4A4).into(),

            accent: rgb(0xC85F44).into(),
            resize_handle: rgb(0x2563EB).into(),
            gauge: rgb(0x2563EB).into(),

            selection: hsla(211.0 / 360.0, 1.0, 0.50, 0.35),
            code_text: rgb(0x9A5528).into(),
            code_wash: hsla(220.0 / 360.0, 0.10, 0.12, 0.07),

            inverse: rgb(0x202227).into(),
            on_inverse: rgb(0xF8F8F9).into(),

            warning: rgb(0xA66B20).into(),
            success: rgb(0x2F8F52).into(),
            favorite: rgb(0xCA8A04).into(),
            danger: rgb(0xC64A42).into(),
            danger_soft: hsla(4.0 / 360.0, 0.55, 0.52, 0.10),
            wallpaper_tint: None,
        }
    }
}

fn mix(a: Hsla, b: Hsla, amount: f32) -> Hsla {
    let amount = amount.clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * amount;
    hsla(
        lerp(a.h, b.h),
        lerp(a.s, b.s),
        lerp(a.l, b.l),
        lerp(a.a, b.a),
    )
}

/// Composite a possibly translucent foreground over an opaque background.
/// Adapted from Zeron's theme contrast helpers (MIT).
fn flatten(fg: Hsla, bg: Hsla) -> Hsla {
    let alpha = fg.a.clamp(0.0, 1.0);
    let fg = fg.to_rgb();
    let bg = bg.to_rgb();
    Rgba {
        r: fg.r * alpha + bg.r * (1.0 - alpha),
        g: fg.g * alpha + bg.g * (1.0 - alpha),
        b: fg.b * alpha + bg.b * (1.0 - alpha),
        a: 1.0,
    }
    .into()
}

fn relative_luminance(color: Hsla) -> f32 {
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    let color = color.to_rgb();
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn painted_contrast(foreground: Hsla, background: Hsla) -> f32 {
    let foreground = flatten(foreground, background);
    let (a, b) = (
        relative_luminance(foreground),
        relative_luminance(background),
    );
    let (high, low) = if a >= b { (a, b) } else { (b, a) };
    (high + 0.05) / (low + 0.05)
}

#[derive(Clone, Copy)]
struct ActiveWakuTheme(Theme);

impl Global for ActiveWakuTheme {}

/// Publish the resolved palette. [`Theme::current`] reads it back from the
/// global, which is how every view gets its colors.
fn set_active_theme(mut theme: Theme, cx: &mut App) {
    if cx.has_global::<ActiveWakuTheme>() {
        theme.wallpaper_tint = cx.global::<ActiveWakuTheme>().0.wallpaper_tint;
    }
    cx.set_global(ActiveWakuTheme(theme));
}

/// Update only the wallpaper-derived surface channel. Changing the base theme
/// preserves it; disabling wallpaper colors clears it without a theme reset.
pub(crate) fn set_wallpaper_tint(tint: Option<Hsla>, cx: &mut App) {
    if cx.has_global::<ActiveWakuTheme>() {
        let mut theme = cx.global::<ActiveWakuTheme>().0;
        theme.wallpaper_tint = tint;
        cx.set_global(ActiveWakuTheme(theme));
    }
}

/// Resolve and publish the startup palette, before any window exists.
pub fn init(cx: &mut App) {
    let system_appearance = cx.window_appearance();
    let theme = if resolves_to_dark(ThemePreference::System, system_appearance) {
        Theme::dark()
    } else {
        Theme::light()
    };
    set_active_theme(theme, cx);
}

pub fn apply_theme_preference(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    crate::platform::set_window_appearance(window, native_override(preference));
    let is_dark = resolves_to_dark(preference, cx.window_appearance());
    set_active_theme(
        if is_dark {
            Theme::dark()
        } else {
            Theme::light()
        },
        cx,
    );
    // AppKit/GPUI may replace the native effect view while changing appearance.
    // Re-request the material before configuring the replacement.
    window.set_background_appearance(crate::platform::preferred_window_background());
    crate::platform::configure_window_material(window, is_dark);
    window.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_surfaces_keep_shell_text_legible_on_an_adverse_backdrop() {
        for theme in [Theme::dark(), Theme::light()] {
            let backdrop = theme.adverse_backdrop();
            for tint in [theme.window_surface(), theme.sidebar_surface()] {
                let composite = flatten(tint, backdrop);
                assert!(painted_contrast(theme.text, composite) >= 4.5);
                assert!(painted_contrast(theme.text_secondary, composite) >= 3.0);
            }
        }
    }

    #[test]
    fn flattening_an_opaque_color_preserves_it() {
        let theme = Theme::dark();
        let flattened = flatten(theme.surface, rgb(0xFFFFFF).into());
        assert_eq!(flattened, theme.surface);
    }
}
