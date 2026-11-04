//! Waku adapter for the reusable overlay scrollbar.

use std::rc::Rc;

use gpui::IntoElement;

use crate::theme::Theme;

use super::ui_theme;

pub use waku_ui::scrollbar::{Scrollable, ScrollbarState};

/// Render the scrollbar with Waku's currently active semantic colors.
pub fn vertical<S>(surface: &S, state: &Rc<ScrollbarState>) -> impl IntoElement + use<S>
where
    S: Scrollable + Clone + 'static,
{
    waku_ui::scrollbar::vertical(surface, state, |cx| ui_theme(Theme::current(cx)))
}
