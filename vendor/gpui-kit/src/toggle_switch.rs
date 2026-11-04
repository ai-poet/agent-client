use gpui::{Context, Div, ElementId, Stateful, Window, div, prelude::*, px};

use crate::{activation::ActivationExt, theme::UiTheme};

/// The shared pill switch used by settings and automation forms.
pub fn toggle_switch<E>(
    id: impl Into<ElementId>,
    on: bool,
    disabled: bool,
    theme: UiTheme,
    cx: &mut Context<E>,
    activate: impl Fn(&mut E, &mut Window, &mut Context<E>) + 'static,
) -> Stateful<Div>
where
    E: 'static,
{
    let base = div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_color(theme.accent))
        .w(px(36.0))
        .h(px(20.0))
        .p(px(2.0))
        .flex_none()
        .rounded_full()
        .cursor_default()
        .when(disabled, |element| element.opacity(0.55))
        .bg(if on { theme.inverse } else { theme.inset })
        .border_1()
        .border_color(if on {
            theme.inverse
        } else {
            theme.border_strong
        })
        .flex()
        .items_center()
        .when(on, |element| element.justify_end())
        .child(div().w(px(14.0)).h(px(14.0)).rounded_full().bg(if on {
            theme.on_inverse
        } else {
            theme.text_tertiary
        }));

    if disabled {
        base
    } else {
        base.on_activation(cx, activate)
    }
}
