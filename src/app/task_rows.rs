//! Sidebar task rows: the failure badge and the remove button.
//!
//! Fork addition. Upstream marks a failed task with a bare red `×` that does
//! nothing when clicked — it reads as a close button and answers like a
//! label, and the failure's cause is nowhere near it. Here the mark is a
//! circled `×` that carries the failure's own words as a tooltip and, when
//! clicked, opens the task at the point where they were said. Removal gets
//! a control of its own: a `×` that appears on hover and on keyboard focus,
//! and asks before it acts, because a task's transcript does not come back.

use crate::ui::ActivationExt as _;

use super::sidebar::localized_session_title;
use super::*;

/// How much of the failure to put in a tooltip.
const FAILURE_SUMMARY_CHARS: usize = 120;

/// The failed turn's last words from the assistant — where the drivers put
/// a provider error or an exit reason — flattened to one line and cut to
/// tooltip length. `None` for a task that has not failed or said nothing.
pub(super) fn failure_summary(session: &AgentSession) -> Option<String> {
    if session.status != SessionStatus::Failed {
        return None;
    }
    let text = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| message.display_content.as_deref().unwrap_or(&message.content))?;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(&collapsed, FAILURE_SUMMARY_CHARS))
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit).collect();
    cut.push('…');
    cut
}

impl Waku {
    /// The failure mark for a task row: tooltip with the cause, click to
    /// open the task where the cause was reported.
    pub(super) fn render_task_failure_badge(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if session.status != SessionStatus::Failed {
            return None;
        }
        let theme = Theme::current(cx);
        let session_id = session.id;
        let summary = failure_summary(session).unwrap_or_else(|| tr!("session.failed"));
        Some(
            div()
                .id(SharedString::from(format!("task-failed-{session_id}")))
                .flex_none()
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(4.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .hover(|style| style.bg(theme.overlay))
                .tooltip(Tooltip::text(SharedString::from(summary)))
                .child(icon("icons/circle-x.svg", 12.0, theme.danger))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.select_session(session_id, cx);
                    this.scroll_transcript_to_bottom(cx);
                })),
        )
    }

    /// The remove control for a task row. Hidden until the row is hovered
    /// or the control is focused, so the list stays quiet; keyboard users
    /// reach it by tabbing, mouse users by hovering. Asks first.
    pub(super) fn render_task_remove_button(
        &self,
        session: &AgentSession,
        group_name: SharedString,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let session_id = session.id;
        let title = localized_session_title(session);
        div()
            .id(SharedString::from(format!("task-remove-{session_id}")))
            .tab_index(0)
            .flex_none()
            .w_0()
            .h(px(18.0))
            .overflow_hidden()
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .opacity(0.0)
            .group_hover(group_name, |style| style.w(px(18.0)).opacity(1.0))
            .focus_visible(|style| {
                style
                    .w(px(18.0))
                    .opacity(1.0)
                    .border_1()
                    .border_color(theme.accent)
            })
            .hover(|style| style.bg(theme.overlay))
            .tooltip(Tooltip::text(tr_cow!("session.remove_task")))
            .child(icon("icons/x.svg", 11.0, theme.text_tertiary))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_activation(cx, move |this, _, cx| {
                this.confirm_remove_task(session_id, title.clone(), cx);
            })
    }

    fn confirm_remove_task(&mut self, session_id: Uuid, title: String, cx: &mut Context<Self>) {
        let detail = format!("{title}\n{}", tr!("session.remove_confirm_detail"));
        self.request_confirm(
            tr!("session.remove_confirm_title"),
            Some(detail),
            tr!("common.remove"),
            true,
            cx,
            move |this, _, cx| this.remove_session(session_id, cx),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_session_with(messages: &[(MessageRole, &str)]) -> AgentSession {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Claude);
        session.status = SessionStatus::Failed;
        for (role, content) in messages {
            session.push_message(*role, (*content).to_owned());
        }
        session
    }

    #[test]
    fn failure_summary_takes_the_last_assistant_words_flattened() {
        let session = failed_session_with(&[
            (MessageRole::User, "do the thing"),
            (MessageRole::Assistant, "Error:   provider\nexited  before   a response"),
        ]);
        assert_eq!(
            failure_summary(&session).as_deref(),
            Some("Error: provider exited before a response")
        );
    }

    #[test]
    fn failure_summary_is_cut_to_tooltip_length() {
        let long = "x".repeat(300);
        let session = failed_session_with(&[(MessageRole::Assistant, &long)]);
        let summary = failure_summary(&session).unwrap();
        assert_eq!(summary.chars().count(), FAILURE_SUMMARY_CHARS + 1);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn failure_summary_needs_a_failed_task_with_something_said() {
        let mut idle = failed_session_with(&[(MessageRole::Assistant, "fine")]);
        idle.status = SessionStatus::Idle;
        assert!(failure_summary(&idle).is_none());
        let silent = failed_session_with(&[(MessageRole::User, "hello")]);
        assert!(failure_summary(&silent).is_none());
    }
}
