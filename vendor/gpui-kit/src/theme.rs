use gpui::{Hsla, hsla, rgb};

/// Semantic values consumed by reusable styled components.
///
/// The application converts its active theme into this snapshot. This crate
/// deliberately owns no global theme, platform material or persistence state.
#[derive(Clone, Copy, Debug)]
pub struct UiTheme {
    pub is_dark: bool,
    pub overlay: Hsla,
    pub overlay_strong: Hsla,
    pub raised: Hsla,
    pub inset: Hsla,
    pub border_strong: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub text_ghost: Hsla,
    pub accent: Hsla,
    pub gauge: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
    pub inverse: Hsla,
    pub on_inverse: Hsla,
}

impl UiTheme {
    /// A standalone fixture theme for component tests and examples.
    pub fn fixture() -> Self {
        Self {
            is_dark: true,
            overlay: hsla(220.0 / 360.0, 0.10, 0.90, 0.05),
            overlay_strong: hsla(220.0 / 360.0, 0.10, 0.90, 0.09),
            raised: rgb(0x232323).into(),
            inset: rgb(0x151515).into(),
            border_strong: hsla(220.0 / 360.0, 0.10, 0.90, 0.14),
            border: hsla(220.0 / 360.0, 0.10, 0.90, 0.07),
            text: rgb(0xE2E2E2).into(),
            text_secondary: rgb(0xA3A3A3).into(),
            text_tertiary: rgb(0x7D7D7D).into(),
            text_ghost: rgb(0x575757).into(),
            accent: rgb(0xE2795B).into(),
            gauge: rgb(0x3B82F6).into(),
            warning: rgb(0xE0B36A).into(),
            success: rgb(0x62C987).into(),
            danger: rgb(0xE2726A).into(),
            inverse: rgb(0xE7E9EC).into(),
            on_inverse: rgb(0x17181C).into(),
        }
    }
}
