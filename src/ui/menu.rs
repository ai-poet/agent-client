//! Waku adapters for the reusable menu and popover components.

use gpui::{AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, Styled};

use crate::theme::Theme;

use super::ui_theme;

pub use client_ui::menu::{
    ConfirmEntry, ContextMenuHandle, DismissMenu, MenuAlign, MenuItem, SelectNextEntry,
    SelectPreviousEntry, init, popover, toggle_popover,
};

pub fn dropdown_menu<E>(
    trigger: E,
    id: impl Into<ElementId>,
    handle: &ContextMenuHandle,
    align: MenuAlign,
    items: impl Fn(&mut App) -> Vec<MenuItem> + 'static,
) -> AnyElement
where
    E: ParentElement + Styled + InteractiveElement + IntoElement + 'static,
{
    client_ui::menu::dropdown_menu_with_theme(trigger, id, handle, align, items, |cx| {
        ui_theme(Theme::current(cx))
    })
}

pub fn context_menu<E>(
    element: E,
    id: impl Into<ElementId>,
    handle: &ContextMenuHandle,
    items: impl Fn(&mut App) -> Vec<MenuItem> + 'static,
) -> AnyElement
where
    E: ParentElement + Styled + InteractiveElement + IntoElement + 'static,
{
    client_ui::menu::context_menu_with_theme(element, id, handle, items, |cx| {
        ui_theme(Theme::current(cx))
    })
}
