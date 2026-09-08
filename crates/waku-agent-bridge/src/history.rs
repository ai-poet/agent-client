//! The conversation, and the three operations Waku needs on it.
//!
//! The engine takes `&mut Vec<Message>` and hands it back mutated, which means
//! **Waku owns the transcript**, not the agent. Rewind and branch are then
//! vector operations rather than a protocol negotiation, and resuming is a
//! deserialize.
//!
//! The one subtlety is what counts as a turn. In the Anthropic message format
//! a tool result is also a `user` message, so counting `Role::User` would make
//! "rewind one turn" land in the middle of a tool exchange and leave a
//! `tool_use` with no matching `tool_result` — a history the API rejects. A
//! turn boundary here is a user message that carries no tool result.

use claurst_core::types::{ContentBlock, Message, MessageContent, Role};

/// Serialize for Waku's session store.
pub fn serialize(messages: &[Message]) -> Vec<u8> {
    serde_json::to_vec(messages).unwrap_or_else(|_| b"[]".to_vec())
}

/// Read back what [`serialize`] wrote. An unreadable or empty payload restores
/// an empty conversation rather than failing the session: losing history is
/// bad, but refusing to open the session loses the user their project too.
pub fn deserialize(bytes: &[u8]) -> Vec<Message> {
    if bytes.is_empty() {
        return Vec::new();
    }
    serde_json::from_slice(bytes).unwrap_or_else(|error| {
        tracing::warn!(%error, "agent: unreadable session history, starting empty");
        Vec::new()
    })
}

/// Indices where a user-authored turn begins.
fn turn_starts(messages: &[Message]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == Role::User && !carries_tool_result(message))
        .map(|(index, _)| index)
        .collect()
}

fn carries_tool_result(message: &Message) -> bool {
    match &message.content {
        MessageContent::Text(_) => false,
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolResult { .. })),
    }
}

/// Number of user-authored turns in the conversation.
pub fn turn_count(messages: &[Message]) -> usize {
    turn_starts(messages).len()
}

/// Drop the last `turns` user turns and everything that followed them.
///
/// Returns the number actually removed, which is smaller than `turns` when the
/// conversation is shorter than the request. Removing zero turns is a no-op
/// rather than an error — the UI gates this, and a bypassed gate should not
/// destroy a conversation.
pub fn rollback(messages: &mut Vec<Message>, turns: usize) -> usize {
    if turns == 0 {
        return 0;
    }
    let starts = turn_starts(messages);
    if starts.is_empty() {
        return 0;
    }
    let removed = turns.min(starts.len());
    let cut = starts[starts.len() - removed];
    messages.truncate(cut);
    removed
}

/// A copy of the conversation with the last `turns_to_remove` turns dropped,
/// for seeding a branch. The original is untouched.
pub fn fork(messages: &[Message], turns_to_remove: usize) -> Vec<Message> {
    let mut branched = messages.to_vec();
    rollback(&mut branched, turns_to_remove);
    branched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_use(id: &str) -> Message {
        Message::assistant_blocks(vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "Bash".into(),
            input: serde_json::json!({}),
            thought_signature: None,
        }])
    }

    fn tool_result(id: &str) -> Message {
        Message::user_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: claurst_core::types::ToolResultContent::Text("ok".into()),
            is_error: None,
        }])
    }

    /// One user turn that ran a tool: user → tool_use → tool_result → answer.
    fn conversation() -> Vec<Message> {
        vec![
            Message::user("first"),
            tool_use("t1"),
            tool_result("t1"),
            Message::assistant("done one"),
            Message::user("second"),
            Message::assistant("done two"),
        ]
    }

    #[test]
    fn a_tool_result_is_not_a_turn_boundary() {
        assert_eq!(turn_count(&conversation()), 2);
    }

    #[test]
    fn rewinding_one_turn_keeps_the_whole_previous_exchange() {
        let mut messages = conversation();
        assert_eq!(rollback(&mut messages, 1), 1);
        assert_eq!(messages.len(), 4);
        // The tool_use and its matching tool_result both survived.
        assert!(matches!(messages[1].role, Role::Assistant));
        assert!(carries_tool_result(&messages[2]));
    }

    #[test]
    fn rewinding_past_the_start_empties_the_conversation_without_panicking() {
        let mut messages = conversation();
        assert_eq!(rollback(&mut messages, 99), 2);
        assert!(messages.is_empty());
    }

    #[test]
    fn rewinding_zero_turns_changes_nothing() {
        let mut messages = conversation();
        assert_eq!(rollback(&mut messages, 0), 0);
        assert_eq!(messages.len(), 6);
    }

    #[test]
    fn a_branch_does_not_disturb_the_conversation_it_came_from() {
        let messages = conversation();
        let branched = fork(&messages, 1);
        assert_eq!(branched.len(), 4);
        assert_eq!(messages.len(), 6);
    }

    #[test]
    fn history_round_trips_through_the_session_store() {
        let messages = conversation();
        let restored = deserialize(&serialize(&messages));
        assert_eq!(restored.len(), messages.len());
        assert_eq!(turn_count(&restored), 2);
    }

    #[test]
    fn an_unreadable_payload_restores_an_empty_conversation() {
        assert!(deserialize(b"{not json").is_empty());
        assert!(deserialize(b"").is_empty());
    }
}
