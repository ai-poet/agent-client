//! A small modal confirmation for the destructive actions this fork adds —
//! removing a task from its row, clearing a custom endpoint.
//!
//! Fork addition. Upstream has no generic confirm: its dialogs are bespoke
//! (goal, commit). This one is deliberately plain: a title, an optional
//! detail line, Cancel, and one confirm button whose label the caller
//! supplies. Enter confirms, Escape cancels, and focus goes to the confirm
//! button on open and back to where it was on close, so a keyboard user is
//! never stranded behind the scrim.

use gpui::{KeyBinding, actions};

use crate::ui::ActivationExt as _;

use super::*;

actions!(waku_confirm_dialog, [AcceptConfirmDialog, DismissConfirmDialog]);

const DIALOG_CONTEXT: &str = "ConfirmDialog";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", AcceptConfirmDialog, Some(DIALOG_CONTEXT)),
        KeyBinding::new("escape", DismissConfirmDialog, Some(DIALOG_CONTEXT)),
    ]);
}

/// What runs when the user confirms.
pub(super) type ConfirmAction = Box<dyn FnOnce(&mut Waku, &mut Window, &mut Context<Waku>)>;

pub(super) struct ConfirmDialogState {
    title: String,
    detail: Option<String>,
    confirm_label: String,
    /// Paints the confirm button in the danger color.
    destructive: bool,
    on_confirm: Option<ConfirmAction>,
    confirm_focus: FocusHandle,
    cancel_focus: FocusHandle,
    /// Where focus was before the dialog took it; restored on close.
    previous_focus: Option<FocusHandle>,
    /// The first frame after opening moves focus into the dialog. Done from
    /// render because the request path may not carry a `Window`.
    focus_pending: bool,
}

impl Waku {
    /// Open the confirmation. `on_confirm` runs only if the user confirms;
    /// cancelling drops it.
    pub(super) fn request_confirm(
        &mut self,
        title: String,
        detail: Option<String>,
        confirm_label: String,
        destructive: bool,
        cx: &mut Context<Self>,
        on_confirm: impl FnOnce(&mut Waku, &mut Window, &mut Context<Waku>) + 'static,
    ) {
        self.confirm_dialog = Some(ConfirmDialogState {
            title,
            detail,
            confirm_label,
            destructive,
            on_confirm: Some(Box::new(on_confirm)),
            confirm_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            previous_focus: None,
            focus_pending: true,
        });
        cx.notify();
    }

    fn accept_confirm_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut dialog) = self.confirm_dialog.take() else {
            return;
        };
        self.restore_focus_after_confirm(dialog.previous_focus.take(), window, cx);
        if let Some(action) = dialog.on_confirm.take() {
            action(self, window, cx);
        }
        cx.notify();
    }

    fn dismiss_confirm_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut dialog) = self.confirm_dialog.take() else {
            return;
        };
        self.restore_focus_after_confirm(dialog.previous_focus.take(), window, cx);
        cx.notify();
    }

    fn restore_focus_after_confirm(
        &self,
        previous: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match previous {
            Some(handle) => window.focus(&handle, cx),
            None => {
                let composer = self.composer_focus(cx);
                window.focus(&composer, cx);
            }
        }
    }

    pub(super) fn render_confirm_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::current(cx);
        let dialog = self.confirm_dialog.as_mut()?;
        if dialog.focus_pending {
            dialog.focus_pending = false;
            dialog.previous_focus = window.focused(cx);
            window.focus(&dialog.confirm_focus, cx);
        }
        let title = dialog.title.clone();
        let detail = dialog.detail.clone();
        let confirm_label = dialog.confirm_label.clone();
        let destructive = dialog.destructive;
        let confirm_focus = dialog.confirm_focus.clone();
        let cancel_focus = dialog.cancel_focus.clone();

        let cancel = div()
            .id("confirm-dialog-cancel")
            .track_focus(&cancel_focus)
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(30.0))
            .px(px(14.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .hover(|style| style.bg(theme.overlay))
            .child(tr!("common.cancel"))
            .on_activation(cx, |this, window, cx| this.dismiss_confirm_dialog(window, cx));

        let confirm = div()
            .id("confirm-dialog-confirm")
            .track_focus(&confirm_focus)
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .h(px(30.0))
            .px(px(14.0))
            .rounded(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .font_weight(FontWeight::MEDIUM)
            .bg(if destructive {
                theme.danger
            } else {
                theme.inverse
            })
            .text_color(if destructive {
                gpui::white()
            } else {
                theme.on_inverse
            })
            .hover(|style| style.opacity(0.9))
            .child(confirm_label)
            .on_activation(cx, |this, window, cx| this.accept_confirm_dialog(window, cx));

        let card = div()
            .id("confirm-dialog-card")
            .key_context(DIALOG_CONTEXT)
            .on_action(cx.listener(|this, _: &AcceptConfirmDialog, window, cx| {
                this.accept_confirm_dialog(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DismissConfirmDialog, window, cx| {
                this.dismiss_confirm_dialog(window, cx);
            }))
            .tab_group()
            .tab_stop(false)
            .w_full()
            .max_w(px(380.0))
            .rounded(px(16.0))
            .bg(theme.composer)
            .shadow_xl()
            .p(px(18.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(sp(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(title),
            )
            .when_some(detail, |card, detail| {
                card.child(
                    div()
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(theme.text_secondary)
                        .child(detail),
                )
            })
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(cancel)
                    .child(confirm),
            );

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        let layer = div()
            .id("confirm-dialog-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(scrim)
            .p(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.dismiss_confirm_dialog(window, cx)),
            )
            .child(card);
        // Above every other modal: a confirmation is the last word.
        Some(gpui::deferred(layer).with_priority(5).into_any_element())
    }
}
