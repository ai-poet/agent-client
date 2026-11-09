use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement, Interactivity, ParentElement, RenderOnce,
    Stateful, StyleRefinement, Styled, Window, div, prelude::*,
};

use crate::input::TextInput;
use crate::theme::{Theme, sp};

use super::ui_theme;

/// Waku adapter for the reusable one-line input shell. The text editor entity
/// stays in the app; only content and visual focus cross the component seam.
#[derive(IntoElement)]
pub struct TextField {
    base: Stateful<Div>,
    input: gpui::Entity<TextInput>,
    icon: Option<(&'static str, f32)>,
}

impl TextField {
    pub fn new(id: impl Into<ElementId>, input: gpui::Entity<TextInput>) -> Self {
        Self {
            base: div().id(id),
            input,
            icon: None,
        }
    }

    /// Leading icon, tinted tertiary like the address bar's lock.
    pub fn icon(mut self, path: &'static str, size: f32) -> Self {
        self.icon = Some((path, size));
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self.input.read(cx).is_visually_focused(window);
        let field = client_ui::TextField::from_base(
            self.base,
            self.input,
            focused,
            ui_theme(Theme::current(cx)),
        )
        .text_metrics(sp(12.5), sp(16.0));
        match self.icon {
            Some((path, size)) => field.icon(path, sp(size)),
            None => field,
        }
    }
}
