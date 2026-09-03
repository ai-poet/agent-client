//! "Edit and send again" for a user message when rewinding is not on offer.
//!
//! Fork addition. Upstream's rewind button edits a user message in place
//! and resubmits it from that point, but only when it can restore the
//! turn's git checkpoint and roll the provider back — so in a folder that
//! is not a git repository, or for a provider without rollback, the
//! affordance is simply absent and the message reads as final. This is the
//! fallback: put the message (attachments included) back into the composer,
//! focus it, and say that sending starts a new turn. The two never appear
//! together on one message: rewind when it can, this otherwise.

use super::*;

/// A user message the composer can take back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResendAction {
    pub session_id: Uuid,
    pub message_id: Uuid,
}

/// The gate, kept pure for the tests: a user message, a settled session,
/// and no rewind on offer for it.
pub(super) fn resend_eligible(
    role: MessageRole,
    status: SessionStatus,
    rewind_available: bool,
) -> bool {
    role == MessageRole::User
        && matches!(status, SessionStatus::Idle | SessionStatus::Failed)
        && !rewind_available
}

impl Waku {
    pub(super) fn resend_action_for_message(&self, message_index: usize) -> Option<ResendAction> {
        let session = self.selected_session()?;
        let message = session.messages.get(message_index)?;
        let rewind_available = self.user_message_action_for_message(message_index).is_some();
        resend_eligible(message.role, session.status, rewind_available).then_some(ResendAction {
            session_id: session.id,
            message_id: message.id,
        })
    }

    /// Put the message back into the composer and hand it focus.
    pub(super) fn resend_user_message(
        &mut self,
        action: ResendAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == action.session_id)
            .and_then(|session| {
                session
                    .messages
                    .iter()
                    .find(|message| message.id == action.message_id)
            })
        else {
            return;
        };
        let submission = ComposerSubmission {
            prompt: message.content.clone(),
            display_content: message.display_content.clone(),
            attachments: message.attachments.clone(),
        };
        self.restore_composer_submission(submission, cx);
        let focus = self.composer_focus(cx);
        window.focus(&focus, cx);
        self.show_toast(tr!("session.resend_as_new_message"));
        cx.notify();
    }
}

/// The footer button beside a user message — the pencil that stands where
/// the rewind icon would otherwise be.
pub(super) fn footer_button(
    action: ResendAction,
    color: gpui::Hsla,
    theme: &Theme,
    waku: gpui::WeakEntity<Waku>,
) -> Stateful<Div> {
    let click_waku = waku.clone();
    let key_waku = waku;
    div()
        .id(SharedString::from(format!(
            "user-message-resend-{}",
            action.message_id
        )))
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .w(px(27.0))
        .h(px(27.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .hover(|element| element.bg(theme.overlay_strong))
        .child(icon("icons/pencil.svg", 14.0, color))
        .tooltip(Tooltip::text(tr_cow!("session.edit_and_resend")))
        .on_click(move |_, window, cx| {
            let _ = click_waku.update(cx, |this, cx| {
                this.resend_user_message(action, window, cx);
            });
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if !event.keystroke.modifiers.modified()
                && matches!(event.keystroke.key.as_str(), "enter" | "space")
            {
                let _ = key_waku.update(cx, |this, cx| {
                    this.resend_user_message(action, window, cx);
                });
                cx.stop_propagation();
            }
        })
}

/// The context-menu entry.
pub(super) fn menu_item(action: ResendAction, waku: gpui::WeakEntity<Waku>) -> MenuItem {
    MenuItem::new(tr!("session.edit_and_resend"), move |window, cx| {
        let _ = waku.update(cx, |this, cx| {
            this.resend_user_message(action, window, cx);
        });
    })
    .icon("icons/pencil.svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_is_offered_only_where_rewind_is_not() {
        assert!(resend_eligible(MessageRole::User, SessionStatus::Idle, false));
        assert!(resend_eligible(MessageRole::User, SessionStatus::Failed, false));
        // Rewind wins where it exists — one affordance per message.
        assert!(!resend_eligible(MessageRole::User, SessionStatus::Idle, true));
        // Never for the assistant's messages, never mid-turn.
        assert!(!resend_eligible(MessageRole::Assistant, SessionStatus::Idle, false));
        assert!(!resend_eligible(MessageRole::User, SessionStatus::Working, false));
        assert!(!resend_eligible(MessageRole::User, SessionStatus::Waiting, false));
        assert!(!resend_eligible(MessageRole::User, SessionStatus::Connecting, false));
    }
}
