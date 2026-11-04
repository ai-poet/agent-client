//! Backdrop-blurred floating surfaces.
//!
//! The reusable scene-layer element lives in `waku-ui`; Waku keeps only its
//! theme-specific translucent fill policy here.

use gpui::Hsla;

use crate::theme::Theme;

use super::ui_theme;

pub use waku_ui::frost::{MENU_BLUR, frosted};

/// A floating surface keeps the theme's hue while letting blurred content
/// show through. Opaque platforms keep the exact original color.
pub fn background(theme: &Theme) -> Hsla {
    waku_ui::frost::background(&ui_theme(*theme))
}
