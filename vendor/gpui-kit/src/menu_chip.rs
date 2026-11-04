use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement, Interactivity, ParentElement, Pixels,
    Rems, RenderOnce, SharedString, Stateful, StyleRefinement, Styled, Window, div, prelude::*, px,
    rems,
};

use crate::{icon::icon, theme::UiTheme};

fn scaled(value: f32) -> Rems {
    rems(value / 14.0)
}

/// A compact chip used as a dropdown-menu trigger.
#[derive(IntoElement)]
pub struct MenuChip {
    base: Stateful<Div>,
    icon: Option<(&'static str, gpui::Hsla)>,
    label: SharedString,
    caret: bool,
    outlined: bool,
    selected: bool,
    disabled: bool,
    height: Option<Pixels>,
    background: Option<gpui::Hsla>,
    theme: Option<UiTheme>,
    text_size: Rems,
    line_height: Rems,
}

impl MenuChip {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            icon: None,
            label: SharedString::default(),
            caret: true,
            outlined: false,
            selected: false,
            disabled: false,
            height: None,
            background: None,
            theme: None,
            text_size: scaled(13.0),
            line_height: scaled(16.0),
        }
    }

    pub fn height(mut self, height: Pixels) -> Self {
        self.height = Some(height);
        self
    }

    pub fn background(mut self, background: gpui::Hsla) -> Self {
        self.background = Some(background);
        self
    }

    pub fn icon(mut self, path: &'static str, color: gpui::Hsla) -> Self {
        self.icon = Some((path, color));
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }

    pub fn outlined(mut self, outlined: bool) -> Self {
        self.outlined = outlined;
        self
    }

    pub fn caret(mut self, caret: bool) -> Self {
        self.caret = caret;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn theme(mut self, theme: UiTheme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn text_metrics(mut self, text_size: Rems, line_height: Rems) -> Self {
        self.text_size = text_size;
        self.line_height = line_height;
        self
    }
}

impl Styled for MenuChip {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for MenuChip {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl ParentElement for MenuChip {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for MenuChip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme.unwrap_or_else(UiTheme::fixture);
        self.base
            .h(self
                .height
                .unwrap_or(if self.outlined { px(30.0) } else { px(26.0) }))
            .px(if self.outlined { px(10.0) } else { px(7.0) })
            .rounded(if self.outlined { px(7.0) } else { px(6.0) })
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(self.text_size)
            .line_height(self.line_height)
            .cursor_default()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .when(self.outlined, |element| {
                element
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(self.background.unwrap_or(theme.raised))
            })
            .when(self.selected, |element| element.bg(theme.overlay))
            .when(!self.disabled, |element| {
                element.hover(|element| element.bg(theme.overlay))
            })
            .when(self.disabled, |element| element.opacity(0.7))
            .when_some(self.icon, |element, (path, color)| {
                element.child(icon(path, scaled(12.0), color))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_secondary)
                    .child(self.label),
            )
            .when(self.caret, |element| {
                element.child(icon(
                    "icons/chevron-down.svg",
                    scaled(10.5),
                    theme.text_ghost,
                ))
            })
    }
}
