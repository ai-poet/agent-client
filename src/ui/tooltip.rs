//! Waku's tooltip view adapter.
//!
//! GPUI owns tooltip behavior; `client-ui` owns the reusable surface.

use gpui::{AnyView, App, AppContext, IntoElement, Render, SharedString, Window};

use crate::theme::{Theme, sp};

use super::ui_theme;

/// A single-line hint.
pub struct Tooltip {
    label: SharedString,
}

impl Tooltip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }

    /// Build the view GPUI's `.tooltip(..)` expects.
    pub fn build(self, _window: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| self).into()
    }

    /// Shorthand for the overwhelmingly common case:
    /// `.tooltip(Tooltip::text("Copy message"))`.
    pub fn text(
        label: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let label = label.into();
        move |window, cx| Tooltip::new(label.clone()).build(window, cx)
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        client_ui::tooltip::Tooltip::new(self.label.clone(), ui_theme(Theme::current(cx)))
            .text_metrics(sp(12.5), sp(15.0))
    }
}
