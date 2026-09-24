//! The conversation, and the three operations Waku needs on it.
//!
//! The engine takes `&mut Vec<Message>` and hands it back mutated, which means
//! **Waku owns the transcript**, not the agent. Rewind and branch are then
//! vector operations rather than a protocol negotiation, and resuming is a
//! deserialize.
//!
//! The one subtlety is what counts as a turn. In the Anthropic message format
//! a tool result is also a `user` message, and the engine adds user messages
//! of its own mid-turn — a steering message, a goal's "keep going", the
//! recovery note after an output limit, a compaction summary. Waku counts a
//! turn per prompt the person sent, so the prompts that start one are marked
//! (in `Message.uuid`, which is stored and never sent to the model) and only
//! marked messages count. Cutting anywhere else would land in the middle of
//! a tool exchange and leave a `tool_use` with no matching `tool_result` — a
//! history the API rejects.
//!
//! A compaction summary stands in for the turns it replaced and is marked
//! with how many there were, so the count Waku keeps stays right. Rewinding
//! into one is refused: the turns it replaced are no longer there to cut.

use claurst_core::types::{ContentBlock, Message, MessageContent, Role};
use claurst_query::COMPACT_SUMMARY_OPEN;

/// `Message.uuid` prefix on a prompt that started a turn.
const TURN_MARK: &str = "waku:turn:";
/// `Message.uuid` prefix on a compaction summary; the rest is how many turns
/// it replaced.
const COMPACT_MARK: &str = "waku:compact:";

/// Serialize for Waku's session store.
pub fn serialize(messages: &[Message]) -> Vec<u8> {
    serde_json::to_vec(messages).unwrap_or_else(|_| b"[]".to_vec())
}

/// Read back what [`serialize`] wrote. An unreadable or empty payload restores
/// an empty conversation rather than failing the session: losing history is
/// bad, but refusing to open the session loses the user their project too.
///
/// A transcript written before turns were marked is marked here, once, by the
/// rule that used to be applied on every count.
pub fn deserialize(bytes: &[u8]) -> Vec<Message> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut messages: Vec<Message> = serde_json::from_slice(bytes).unwrap_or_else(|error| {
        tracing::warn!(%error, "agent: unreadable session history, starting empty");
        Vec::new()
    });
    adopt_legacy(&mut messages);
    messages
}

/// Mark `message` as the prompt that starts a turn. Returns the mark, which
/// finds the message again after the engine has rearranged the history.
pub fn mark_turn(message: &mut Message) -> String {
    let mark = format!("{TURN_MARK}{}", uuid::Uuid::new_v4());
    message.uuid = Some(mark.clone());
    mark
}

/// Mark the turns of a transcript that predates marking: every user message
/// without a tool result, which is what used to be counted. A compaction
/// summary among them counts as the one turn it used to.
fn adopt_legacy(messages: &mut [Message]) {
    if messages.iter().any(|message| mark_of(message).is_some()) {
        return;
    }
    for message in messages.iter_mut() {
        if message.role != Role::User || carries_tool_result(message) {
            continue;
        }
        if is_compaction_summary(message) {
            message.uuid = Some(format!("{COMPACT_MARK}1"));
        } else {
            mark_turn(message);
        }
    }
}

/// After a turn in which the engine compacted, mark its summary with the
/// number of turns it replaced: whatever the conversation weighed before the
/// turn, less what is still there to count.
pub fn settle_compaction(weight_before: usize, messages: &mut [Message]) {
    let Some(index) = messages
        .iter()
        .position(|message| mark_of(message).is_none() && is_compaction_summary(message))
    else {
        return;
    };
    let remaining: usize = messages.iter().map(weight).sum();
    let replaced = weight_before.saturating_sub(remaining);
    messages[index].uuid = Some(format!("{COMPACT_MARK}{replaced}"));
}

fn mark_of(message: &Message) -> Option<&str> {
    message
        .uuid
        .as_deref()
        .filter(|uuid| uuid.starts_with(TURN_MARK) || uuid.starts_with(COMPACT_MARK))
}

/// How many turns a message stands for.
fn weight(message: &Message) -> usize {
    match message.uuid.as_deref() {
        Some(uuid) if uuid.starts_with(TURN_MARK) => 1,
        Some(uuid) => uuid
            .strip_prefix(COMPACT_MARK)
            .and_then(|count| count.parse().ok())
            .unwrap_or(0),
        None => 0,
    }
}

fn is_compaction_summary(message: &Message) -> bool {
    message.role == Role::User && message.get_all_text().contains(COMPACT_SUMMARY_OPEN)
}

/// Indices of the prompts that started a turn.
fn turn_starts(messages: &[Message]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message
                .uuid
                .as_deref()
                .is_some_and(|uuid| uuid.starts_with(TURN_MARK))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Where the prompt carrying `mark` sits now, or `None` once a compaction has
/// folded it into a summary.
pub fn position_of(messages: &[Message], mark: &str) -> Option<usize> {
    messages
        .iter()
        .position(|message| message.uuid.as_deref() == Some(mark))
}

fn carries_tool_result(message: &Message) -> bool {
    match &message.content {
        MessageContent::Text(_) => false,
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolResult { .. })),
    }
}

/// Number of turns in the conversation, counting the ones a compaction
/// summary replaced.
pub fn turn_count(messages: &[Message]) -> usize {
    messages.iter().map(weight).sum()
}

/// Rewinding would have to cut inside a compaction summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewindPastCompaction;

impl std::fmt::Display for RewindPastCompaction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "the conversation before this point was compacted and can no longer be rewound to",
        )
    }
}

impl std::error::Error for RewindPastCompaction {}

/// Drop the last `turns` turns and everything that followed them.
///
/// Returns the number actually removed, which is smaller than `turns` when the
/// conversation is shorter than the request. Removing zero turns is a no-op
/// rather than an error — the UI gates this, and a bypassed gate should not
/// destroy a conversation. Reaching back past a compaction summary that
/// replaced turns is an error and changes nothing.
pub fn rollback(messages: &mut Vec<Message>, turns: usize) -> Result<usize, RewindPastCompaction> {
    if turns == 0 {
        return Ok(0);
    }
    let mut removed = 0;
    let mut cut = None;
    for (index, message) in messages.iter().enumerate().rev() {
        if removed == turns {
            break;
        }
        match weight(message) {
            0 => {}
            1 if message
                .uuid
                .as_deref()
                .is_some_and(|uuid| uuid.starts_with(TURN_MARK)) =>
            {
                removed += 1;
                cut = Some(index);
            }
            _ => return Err(RewindPastCompaction),
        }
    }
    if let Some(cut) = cut {
        messages.truncate(cut);
    }
    Ok(removed)
}

/// A copy of the conversation with the last `turns_to_remove` turns dropped,
/// for seeding a branch. The original is untouched.
pub fn fork(
    messages: &[Message],
    turns_to_remove: usize,
) -> Result<Vec<Message>, RewindPastCompaction> {
    let mut branched = messages.to_vec();
    rollback(&mut branched, turns_to_remove)?;
    Ok(branched)
}

/// One user turn as a reader would see it: what they wrote, and what the
/// assistant said back — tool calls, tool results and thinking left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranscriptTurn {
    pub user: String,
    pub assistant: String,
}

/// The conversation as display turns, for importing a stored transcript
/// into a client that keeps its own transcript format.
pub fn turns_for_display(messages: &[Message]) -> Vec<TranscriptTurn> {
    let starts = turn_starts(messages);
    starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = starts.get(index + 1).copied().unwrap_or(messages.len());
            let assistant = messages[start + 1..end]
                .iter()
                .filter(|message| message.role == Role::Assistant)
                .map(visible_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            TranscriptTurn {
                user: visible_text(&messages[start]),
                assistant,
            }
        })
        .collect()
}

/// The text a person would read in a message: text blocks only.
pub fn visible_text(message: &Message) -> String {
    match &message.content {
        MessageContent::Text(text) => text.trim().to_owned(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned(),
    }
}

/// Whether the messages appended from `start` onwards contain anything the
/// user would recognise as an answer: assistant text, or reasoning.
///
/// A turn that ends cleanly and adds none of that has failed at something,
/// however cheerful its stop reason. Tool calls alone do not count — a turn
/// that only ran tools and then stopped left the user with nothing either.
pub fn produced_visible_output(messages: &[Message], start: usize) -> bool {
    messages
        .iter()
        .skip(start)
        .filter(|message| message.role == Role::Assistant)
        .any(|message| {
            !visible_text(message).is_empty()
                || match &message.content {
                    MessageContent::Blocks(blocks) => blocks
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Thinking { .. })),
                    MessageContent::Text(_) => false,
                }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_that_only_ran_tools_counts_as_having_said_nothing() {
        // The shape a swallowed upstream failure takes, and the shape of a
        // turn that stopped mid-work: neither left the user an answer.
        let messages = vec![Message::user("go"), tool_use("call-1"), tool_result("call-1")];
        assert!(!produced_visible_output(&messages, 1));
    }

    #[test]
    fn assistant_text_and_reasoning_both_count() {
        let text = vec![Message::user("go"), Message::assistant("here you go")];
        assert!(produced_visible_output(&text, 1));

        let thinking = vec![
            Message::user("go"),
            Message::assistant_blocks(vec![ContentBlock::Thinking {
                thinking: "hmm".into(),
                signature: String::new(),
            }]),
        ];
        assert!(produced_visible_output(&thinking, 1));
    }

    #[test]
    fn only_this_turns_messages_are_looked_at() {
        let messages = vec![
            Message::assistant("an answer from the turn before"),
            Message::user("go"),
            tool_use("call-1"),
        ];
        assert!(!produced_visible_output(&messages, 2));
    }

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

    fn prompt(text: &str) -> Message {
        let mut message = Message::user(text);
        mark_turn(&mut message);
        message
    }

    fn summary(replaced_text: &str) -> Message {
        Message::user(format!(
            "This session is being continued.\n\n<compact-summary>\n{replaced_text}\n</compact-summary>"
        ))
    }

    /// One user turn that ran a tool: user → tool_use → tool_result → answer.
    fn conversation() -> Vec<Message> {
        vec![
            prompt("first"),
            tool_use("t1"),
            tool_result("t1"),
            Message::assistant("done one"),
            prompt("second"),
            Message::assistant("done two"),
        ]
    }

    #[test]
    fn a_tool_result_is_not_a_turn_boundary() {
        assert_eq!(turn_count(&conversation()), 2);
    }

    /// A steering message, a goal's "keep going" and the engine's recovery
    /// note are user messages too, but nobody pressed send for them: Waku's
    /// turn count does not include them, so neither may this one.
    #[test]
    fn messages_the_engine_added_mid_turn_are_not_turns() {
        let mut messages = conversation();
        messages.insert(4, Message::user("also check the tests"));
        messages.push(Message::user("Continue working toward the goal."));
        messages.push(Message::assistant("continuing"));
        assert_eq!(turn_count(&messages), 2);

        // Rewinding one turn removes the second prompt and everything after
        // it, and keeps the steer that belonged to the first turn.
        assert_eq!(rollback(&mut messages, 1), Ok(1));
        assert_eq!(messages.len(), 5);
        assert_eq!(visible_text(&messages[4]), "also check the tests");
    }

    #[test]
    fn a_transcript_from_before_marking_is_marked_by_the_old_rule_on_load() {
        let unmarked = vec![
            Message::user("first"),
            tool_use("t1"),
            tool_result("t1"),
            Message::assistant("done one"),
            Message::user("second"),
        ];
        let restored = deserialize(&serialize(&unmarked));
        assert_eq!(turn_count(&restored), 2);
        assert!(restored[0].uuid.as_deref().unwrap().starts_with(TURN_MARK));
        assert!(restored[2].uuid.is_none(), "a tool result is never a turn");

        // Already-marked transcripts are left exactly as they were.
        let marked = conversation();
        let again = deserialize(&serialize(&marked));
        assert_eq!(again[0].uuid, marked[0].uuid);
    }

    #[test]
    fn a_compaction_summary_counts_the_turns_it_replaced() {
        // Three turns, then the engine folded the first two into a summary
        // and kept the third verbatim.
        let before = vec![
            prompt("one"),
            Message::assistant("a"),
            prompt("two"),
            Message::assistant("b"),
            prompt("three"),
            Message::assistant("c"),
        ];
        let weight_before = turn_count(&before);
        let mut after = vec![summary("one and two")];
        after.extend_from_slice(&before[4..]);

        settle_compaction(weight_before, &mut after);
        assert_eq!(turn_count(&after), 3);
        assert_eq!(after[0].uuid.as_deref(), Some("waku:compact:2"));

        // The kept turn can still be rewound; reaching into the summary cannot.
        let mut rewound = after.clone();
        assert_eq!(rollback(&mut rewound, 1), Ok(1));
        assert_eq!(rewound.len(), 1);
        let mut too_far = after.clone();
        assert_eq!(rollback(&mut too_far, 2), Err(RewindPastCompaction));
        assert_eq!(
            too_far.len(),
            after.len(),
            "a refused rewind changes nothing"
        );
        assert!(fork(&after, 3).is_err());
    }

    #[test]
    fn a_prompt_is_found_again_until_a_compaction_folds_it_away() {
        let mut message = Message::user("go");
        let mark = mark_turn(&mut message);
        let messages = vec![Message::assistant("earlier"), message];
        assert_eq!(position_of(&messages, &mark), Some(1));
        assert_eq!(position_of(&[summary("go")], &mark), None);
    }

    #[test]
    fn rewinding_one_turn_keeps_the_whole_previous_exchange() {
        let mut messages = conversation();
        assert_eq!(rollback(&mut messages, 1), Ok(1));
        assert_eq!(messages.len(), 4);
        // The tool_use and its matching tool_result both survived.
        assert!(matches!(messages[1].role, Role::Assistant));
        assert!(carries_tool_result(&messages[2]));
    }

    #[test]
    fn rewinding_past_the_start_empties_the_conversation_without_panicking() {
        let mut messages = conversation();
        assert_eq!(rollback(&mut messages, 99), Ok(2));
        assert!(messages.is_empty());
    }

    #[test]
    fn rewinding_zero_turns_changes_nothing() {
        let mut messages = conversation();
        assert_eq!(rollback(&mut messages, 0), Ok(0));
        assert_eq!(messages.len(), 6);
    }

    #[test]
    fn a_branch_does_not_disturb_the_conversation_it_came_from() {
        let messages = conversation();
        let branched = fork(&messages, 1).unwrap();
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

    #[test]
    fn display_turns_pair_each_prompt_with_the_answer_and_hide_the_tools() {
        let turns = turns_for_display(&conversation());
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user, "first");
        assert_eq!(turns[0].assistant, "done one");
        assert_eq!(turns[1].user, "second");
        assert_eq!(turns[1].assistant, "done two");
    }

    #[test]
    fn a_turn_still_running_has_an_empty_answer_rather_than_none() {
        let turns = turns_for_display(&[prompt("hello")]);
        assert_eq!(
            turns,
            vec![TranscriptTurn {
                user: "hello".into(),
                assistant: String::new(),
            }]
        );
    }
}
