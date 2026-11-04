use gpui::{
    App, IntoElement, ParentElement, Rems, RenderOnce, SharedString, Styled, Window, div, px, rems,
};

use crate::theme::UiTheme;

fn scaled(value: f32) -> Rems {
    rems(value / 14.0)
}

/// A single-line tooltip surface; GPUI owns hover timing and placement.
#[derive(IntoElement)]
pub struct Tooltip {
    label: SharedString,
    theme: UiTheme,
    text_size: Rems,
    line_height: Rems,
}

impl Tooltip {
    pub fn new(label: impl Into<SharedString>, theme: UiTheme) -> Self {
        Self {
            label: label.into(),
            theme,
            text_size: scaled(12.5),
            line_height: scaled(15.0),
        }
    }

    pub fn text_metrics(mut self, text_size: Rems, line_height: Rems) -> Self {
        self.text_size = text_size;
        self.line_height = line_height;
        self
    }
}

impl RenderOnce for Tooltip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        div().pt(px(4.0)).pl(px(2.0)).child(
            div()
                .px(px(7.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.raised)
                .shadow_md()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(self.text_size)
                .line_height(self.line_height)
                .text_color(theme.text_secondary)
                .child(self.label),
        )
    }
}
