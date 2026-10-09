// Fork addition (Waku): reminders that change from step to step travel on the
// request's last message, never in the system prompt.
//
// The system prompt opens every request. The engine used to append the todo
// nudge ("You have N incomplete tasks…", from step 3 on, its count moving with
// every finished item) and the goal's progress to it, so the prompt changed
// whenever either did — and everything after it, the whole conversation,
// missed the prompt cache: GPT's prefix cache and Claude's breakpoints alike.
// Sent as a `<system-reminder>` on the last message of the one request, the
// change costs only that message. Nothing here is written to the history.

use claurst_core::types::{ContentBlock, Message, MessageContent, Role};
use claurst_tools::ToolContext;

use crate::QueryConfig;

/// This step's reminder: the todo nudge from step 3 on, and the active goal's
/// progress in goal mode. `None` when there is nothing to remind.
pub(crate) fn step_reminder(
    turn: u32,
    config: &QueryConfig,
    tool_ctx: &ToolContext,
) -> Option<String> {
    let mut parts = Vec::new();
    if turn > 2 {
        let nudge = super::tools::build_todo_nudge(&tool_ctx.session_id);
        if !nudge.is_empty() {
            parts.push(nudge);
        }
    }
    if matches!(
        config.continuation,
        crate::continuation::ContinuationMode::Goal
    ) {
        // Synchronous store access, no lock held across an `.await`.
        if let Some(goal) = claurst_core::GoalStore::open_default()
            .and_then(|store| store.get_active_goal(&tool_ctx.session_id))
        {
            parts.push(claurst_core::goal_system_prompt_addendum(&goal));
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// A copy of `messages` whose last message carries `reminder` as a trailing
/// `<system-reminder>` text block — or, when the assistant spoke last, with a
/// user message of its own after it.
pub(crate) fn with_step_reminder(messages: &[Message], reminder: &str) -> Vec<Message> {
    let mut copy = messages.to_vec();
    let block = ContentBlock::Text {
        text: format!("<system-reminder>\n{reminder}\n</system-reminder>"),
    };
    match copy.last_mut() {
        Some(last) if last.role == Role::User => {
            let content = std::mem::replace(&mut last.content, MessageContent::Blocks(Vec::new()));
            let mut blocks = match content {
                MessageContent::Blocks(blocks) => blocks,
                MessageContent::Text(text) => vec![ContentBlock::Text { text }],
            };
            blocks.push(block);
            last.content = MessageContent::Blocks(blocks);
        }
        _ => copy.push(Message::user_blocks(vec![block])),
    }
    copy
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::types::ToolResultContent;

    fn texts(message: &Message) -> Vec<String> {
        match &message.content {
            MessageContent::Text(text) => vec![text.clone()],
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => text.clone(),
                    ContentBlock::ToolResult { tool_use_id, .. } => format!("result:{tool_use_id}"),
                    _ => "other".to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_reminder_follows_the_tool_results_and_the_history_is_untouched() {
        let history = vec![
            Message::user("do it"),
            Message::assistant("calling"),
            Message::user_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: ToolResultContent::Text("ok".into()),
                is_error: None,
            }]),
        ];
        let sent = with_step_reminder(&history, "You have 2 incomplete tasks.");
        assert_eq!(sent.len(), 3);
        assert_eq!(
            texts(&sent[2]),
            [
                "result:call_1".to_owned(),
                "<system-reminder>\nYou have 2 incomplete tasks.\n</system-reminder>".to_owned()
            ]
        );
        // Everything before the last message is the history as it was.
        assert_eq!(texts(&sent[0]), texts(&history[0]));
        assert_eq!(texts(&history[2]), ["result:call_1".to_owned()]);
    }

    #[test]
    fn a_plain_prompt_keeps_its_text_first() {
        let sent = with_step_reminder(&[Message::user("hi")], "r");
        assert_eq!(
            texts(&sent[0]),
            [
                "hi".to_owned(),
                "<system-reminder>\nr\n</system-reminder>".to_owned()
            ]
        );
    }

    #[test]
    fn after_the_assistant_it_is_a_message_of_its_own() {
        let sent = with_step_reminder(&[Message::user("hi"), Message::assistant("done")], "r");
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].role, Role::User);
    }
}
