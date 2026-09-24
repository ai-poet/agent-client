//! An agent's todo list, drawn the same wherever it appears: expanded under
//! the tool row that wrote it, and in the status capsule's progress section.

use gpui::{Div, FontWeight, ParentElement, Styled, div, px};

use crate::theme::Theme;
use crate::todo::{TodoItem, TodoStatus};

/// One row per item, in the agent's own order: ○ waiting, → being worked on,
/// ✓ done and ✕ dropped, the last two struck through so the eye lands on
/// what is left.
pub(super) fn render_todo_list(items: &[TodoItem], text_size: f32, theme: &Theme) -> Div {
    let mut list = div().w_full().min_w_0().flex().flex_col().gap(px(3.0));
    for item in items {
        list = list.child(render_todo_row(item, text_size, theme));
    }
    list
}

fn render_todo_row(item: &TodoItem, text_size: f32, theme: &Theme) -> Div {
    let (glyph, glyph_color) = match item.status {
        TodoStatus::Pending => ("○", theme.text_ghost),
        TodoStatus::InProgress => ("→", theme.accent),
        TodoStatus::Completed => ("✓", theme.success),
        TodoStatus::Cancelled => ("✕", theme.text_ghost),
    };
    let mut label = div()
        .flex_1()
        .min_w_0()
        .text_size(px(text_size))
        .line_height(px(text_size * 1.45))
        .child(item.label().to_owned());
    label = match item.status {
        TodoStatus::InProgress => label.text_color(theme.text).font_weight(FontWeight::MEDIUM),
        TodoStatus::Pending => label.text_color(theme.text_secondary),
        TodoStatus::Completed | TodoStatus::Cancelled => {
            label.text_color(theme.text_tertiary).line_through()
        }
    };
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_start()
        .gap(px(8.0))
        .child(
            div()
                .flex_none()
                .w(px(14.0))
                .text_size(px(text_size))
                .line_height(px(text_size * 1.45))
                .text_color(glyph_color)
                .child(glyph),
        )
        .child(label)
}
