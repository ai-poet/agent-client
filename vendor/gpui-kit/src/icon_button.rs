use gpui::{Div, ElementId, Rems, Stateful, div, prelude::*, px};

use crate::{icon::icon, theme::UiTheme};

/// A compact ghost icon button with Waku's interaction treatment.
pub fn icon_button(
    id: impl Into<ElementId>,
    path: &'static str,
    icon_size: Rems,
    theme: UiTheme,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(22.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .hover(|element| element.bg(theme.overlay))
        .active(|element| element.bg(theme.overlay_strong))
        .child(icon(path, icon_size, theme.text_tertiary))
}
