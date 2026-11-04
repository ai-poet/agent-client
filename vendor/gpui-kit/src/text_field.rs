use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement, Interactivity, ParentElement, Rems,
    RenderOnce, Stateful, StyleRefinement, Styled, Window, div, prelude::*, px, rems,
};

use crate::{icon::icon, theme::UiTheme};

/// The visual shell for a one-line input. Editing state stays in the caller.
#[derive(IntoElement)]
pub struct TextField {
    base: Stateful<Div>,
    content: AnyElement,
    focused: bool,
    theme: UiTheme,
    icon: Option<(&'static str, Rems)>,
    text_size: Rems,
    line_height: Rems,
}

impl TextField {
    pub fn new(
        id: impl Into<ElementId>,
        content: impl IntoElement,
        focused: bool,
        theme: UiTheme,
    ) -> Self {
        Self::from_base(div().id(id), content, focused, theme)
    }

    pub fn from_base(
        base: Stateful<Div>,
        content: impl IntoElement,
        focused: bool,
        theme: UiTheme,
    ) -> Self {
        Self {
            base,
            content: content.into_any_element(),
            focused,
            theme,
            icon: None,
            text_size: rems(12.5 / 14.0),
            line_height: rems(16.0 / 14.0),
        }
    }

    pub fn icon(mut self, path: &'static str, size: Rems) -> Self {
        self.icon = Some((path, size));
        self
    }

    pub fn text_metrics(mut self, text_size: Rems, line_height: Rems) -> Self {
        self.text_size = text_size;
        self.line_height = line_height;
        self
    }
}

impl Styled for TextField {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for TextField {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl ParentElement for TextField {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for TextField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        self.base
            .h(px(28.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(if self.focused {
                theme.accent
            } else {
                theme.border_strong
            })
            .bg(theme.inset)
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(self.text_size)
            .line_height(self.line_height)
            .when_some(self.icon, |element, (path, size)| {
                element.child(icon(path, size, theme.text_tertiary))
            })
            .child(div().min_w_0().flex_1().child(self.content))
    }
}
