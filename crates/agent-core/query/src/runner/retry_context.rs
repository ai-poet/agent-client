// Fork addition (Waku): one logical model request's stable inputs, and the
// visible fragment a failed attempt leaves for the next one.
//
// A retry must not silently become another request. Queue drains, todo/goal
// reminders and the system prompt stay where the first attempt put them. Visible
// text is different: it is evidence of work already done, so it remains in the
// conversation with an explicit interruption boundary instead of disappearing.

use agent_core::effort::EffortLevel;
use agent_core::types::{Message, Role};
use agent_core_api::SystemPrompt;

/// `Message.uuid` is persisted but never sent to a provider. It lets the local
/// transcript distinguish an interrupted fragment from a completed answer.
pub(crate) const INTERRUPTED_RESPONSE_MARK: &str = "waku:interrupted:";

/// What changes between attempts of one logical request.
#[derive(Default)]
pub(crate) struct RetryContext {
    replaying: bool,
    system: Option<SystemPrompt>,
    reminder: Option<String>,
    effort: Option<EffortLevel>,
}

impl RetryContext {
    /// Whether this iteration replays a request. Queue drains and history
    /// rewrites belong only to a new logical request, not to this state.
    pub(crate) fn replaying(&self) -> bool {
        self.replaying
    }

    /// Keep the current inputs for another attempt. The first attempt has
    /// already assembled them by the time a retry can be requested.
    pub(crate) fn begin_retry(&mut self) {
        self.replaying = true;
    }

    /// A response settled, or compaction deliberately changed the context:
    /// the next request assembles fresh inputs and may accept new steering.
    pub(crate) fn finish_request(&mut self) {
        self.replaying = false;
        self.system = None;
        self.reminder = None;
        self.effort = None;
    }

    /// Reuse the first attempt's system prompt through every retry.
    pub(crate) fn system(&mut self, build: impl FnOnce() -> SystemPrompt) -> SystemPrompt {
        if self.replaying {
            if let Some(system) = &self.system {
                return system.clone();
            }
        }
        let system = build();
        self.system = Some(system.clone());
        system
    }

    /// Reuse the first attempt's reminder even if a background task or the
    /// goal store changes while the retry is waiting.
    pub(crate) fn reminder(&mut self, build: impl FnOnce() -> Option<String>) -> Option<String> {
        if self.replaying {
            return self.reminder.clone();
        }
        let reminder = build();
        self.reminder = reminder.clone();
        reminder
    }

    /// Keep an inline ultracode/persona choice too. The continuation note is a
    /// new user message, so re-deriving this from the updated history could
    /// silently lower the next attempt's reasoning effort.
    pub(crate) fn effort(
        &mut self,
        build: impl FnOnce() -> Option<EffortLevel>,
    ) -> Option<EffortLevel> {
        if self.replaying {
            return self.effort;
        }
        let effort = build();
        self.effort = effort;
        effort
    }
}

/// Keep what a failed attempt made visible without treating incomplete tool
/// calls as executable history. `text` contains only visible text, never raw
/// thinking blocks or partial tool-call JSON.
pub(crate) fn preserve_interrupted_response(
    messages: &mut Vec<Message>,
    text: String,
    had_content: bool,
) {
    if !had_content {
        return;
    }
    if !text.is_empty() {
        let mut partial = Message::assistant(text);
        partial.uuid = Some(format!(
            "{INTERRUPTED_RESPONSE_MARK}{}",
            uuid::Uuid::new_v4()
        ));
        messages.push(partial);
    }
    let note = if messages.last().is_some_and(|message| {
        message.role == Role::Assistant
            && message
                .uuid
                .as_deref()
                .is_some_and(|uuid| uuid.starts_with(INTERRUPTED_RESPONSE_MARK))
    }) {
        "The immediately preceding assistant message is an interrupted fragment, not a complete \
         answer. It is preserved so you know what was already generated. Continue the original \
         task from that point without repeating the fragment or discarding earlier context. Any \
         incomplete tool calls were not executed; check the current state before making new calls."
    } else {
        "The previous model attempt was interrupted before producing visible text. Any tool calls \
         it started were not executed. Continue the original task from the existing context; \
         check the current state before making new calls."
    };
    messages.push(Message::user(note));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retry_reuses_the_system_and_reminder_until_it_settles() {
        let mut context = RetryContext::default();
        let first_system = context.system(|| SystemPrompt::Text("first".into()));
        let first_reminder = context.reminder(|| Some("two todos".into()));
        let first_effort = context.effort(|| Some(EffortLevel::Ultracode));
        context.begin_retry();
        assert_eq!(
            serde_json::to_value(context.system(|| SystemPrompt::Text("changed".into()))).unwrap(),
            serde_json::to_value(first_system).unwrap()
        );
        assert_eq!(context.reminder(|| Some("one todo".into())), first_reminder);
        assert_eq!(context.effort(|| Some(EffortLevel::Low)), first_effort);
        assert!(context.replaying());

        context.finish_request();
        assert_eq!(
            serde_json::to_value(context.system(|| SystemPrompt::Text("next".into()))).unwrap(),
            serde_json::to_value(SystemPrompt::Text("next".into())).unwrap()
        );
        assert_eq!(
            context.reminder(|| Some("one todo".into())),
            Some("one todo".into())
        );
        assert_eq!(
            context.effort(|| Some(EffortLevel::Low)),
            Some(EffortLevel::Low)
        );
    }

    #[test]
    fn an_interrupted_fragment_is_marked_and_followed_by_a_continuation_note() {
        let mut messages = vec![Message::user("start")];
        preserve_interrupted_response(&mut messages, "Half an answer".into(), true);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].get_all_text(), "Half an answer");
        assert!(messages[1]
            .uuid
            .as_deref()
            .is_some_and(|uuid| uuid.starts_with(INTERRUPTED_RESPONSE_MARK)));
        assert_eq!(messages[2].role, Role::User);
        assert!(messages[2].get_all_text().contains("without repeating"));
    }

    #[test]
    fn no_content_means_no_history_and_tool_only_content_means_only_a_note() {
        let mut empty = vec![Message::user("start")];
        preserve_interrupted_response(&mut empty, String::new(), false);
        assert_eq!(empty.len(), 1);

        preserve_interrupted_response(&mut empty, String::new(), true);
        assert_eq!(empty.len(), 2);
        assert_eq!(empty[1].role, Role::User);
        assert!(empty[1].get_all_text().contains("not executed"));
    }
}
