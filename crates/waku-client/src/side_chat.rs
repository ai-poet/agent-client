//! What a side question is asked with: a digest of the task so far.
//!
//! Fork addition (`/btw`, ⌘;). The question is answered by a model that has
//! no tools and is not the task's agent, so it reads the task the way the
//! person has seen it — Waku's own transcript, which every provider fills —
//! cut to a budget: newest first, each message capped, each piece of work
//! one line, reasoning and tool output left out. Pure, so every rule is
//! tested on plain values.

use waku_protocol::SideChatMessage;
use waku_protocol::model::{ActivityKind, AgentSession, MessageRole, TranscriptBlock};

/// The digest's budget, in bytes.
pub const DIGEST_BUDGET: usize = 48 * 1024;
/// One message's share of it, head and tail kept.
const MESSAGE_CAP: usize = 6 * 1024;
/// Earlier exchanges of the side thread sent along with a new question.
const THREAD_EXCHANGES: usize = 10;
/// Their budget, in bytes.
const THREAD_BUDGET: usize = 64 * 1024;

/// How the answering model is told what it is doing.
pub const SIDE_SYSTEM: &str = "You are answering a side question about a coding task that \
another agent is working on. You only see a digest of that task's transcript: you cannot \
use tools, read files or act, and nothing you say reaches the agent doing the task. Answer \
from what the transcript shows, briefly, in the language the question is asked in. Say so \
when the transcript does not show something; never claim to have changed anything.";

/// The task's transcript cut to a budget.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Digest {
    pub text: String,
    /// Messages and pieces of work left out at the start for the budget.
    pub omitted: usize,
}

/// One exchange of a side thread, as the desktop keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideExchange {
    pub question: String,
    /// `None` while unanswered, or when the answer failed.
    pub answer: Option<String>,
}

/// Head and tail of `text` within `cap` bytes, cut on character boundaries.
fn cap_text(text: &str, cap: usize) -> String {
    let text = text.trim();
    if text.len() <= cap {
        return text.to_owned();
    }
    let half = cap / 2;
    let mut head = half;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - half;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}\n[…]\n{}", &text[..head], &text[tail..])
}

fn activity_lines(block: &TranscriptBlock) -> Vec<String> {
    block
        .activities
        .iter()
        .filter(|activity| activity.kind != ActivityKind::Reasoning)
        .map(|activity| {
            let mut line = format!("- [{:?}] {}", activity.kind, activity.title.trim());
            if activity.failed {
                line.push_str(" (failed)");
            } else if !activity.complete {
                line.push_str(" (stopped or running)");
            }
            for change in &activity.file_changes {
                line.push_str(&format!(
                    "; {} +{} -{}",
                    change.path,
                    change.additions.unwrap_or(0),
                    change.deletions.unwrap_or(0)
                ));
            }
            cap_text(&line, 400)
        })
        .collect()
}

/// The digest of `session`'s transcript within `budget` bytes.
pub fn build_digest(session: &AgentSession, budget: usize) -> Digest {
    // Oldest first: the work recorded before each message, then the message.
    let mut entries: Vec<String> = Vec::new();
    for index in 0..=session.messages.len() {
        for block in session
            .transcript_blocks
            .iter()
            .filter(|block| block.after_message == index)
        {
            entries.extend(activity_lines(block));
        }
        if let Some(message) = session.messages.get(index) {
            let speaker = match message.role {
                MessageRole::User => "Person",
                MessageRole::Assistant => "Agent",
                MessageRole::System => "System",
            };
            let text = message.display_content.as_deref().unwrap_or(&message.content);
            let mut entry = format!("[{speaker}]\n{}", cap_text(text, MESSAGE_CAP));
            for attachment in &message.attachments {
                entry.push_str(&format!("\n(attached: {})", attachment.name));
            }
            entries.push(entry);
        }
    }

    let mut footer = String::new();
    if let Some(goal) = &session.thread_goal {
        footer.push_str(&format!("\nGoal: {}\n", goal.objective.trim()));
    }
    if !session.todos.is_empty() {
        footer.push_str("\nThe agent's todo list:\n");
        for todo in &session.todos {
            footer.push_str(&format!("- [{:?}] {}\n", todo.status, todo.content.trim()));
        }
    }

    let mut used = footer.len();
    let mut kept: Vec<&String> = Vec::new();
    for entry in entries.iter().rev() {
        if used + entry.len() + 2 > budget && !kept.is_empty() {
            break;
        }
        used += entry.len() + 2;
        kept.push(entry);
    }
    let omitted = entries.len() - kept.len();
    let mut text = String::new();
    if omitted > 0 {
        text.push_str(&format!("[{omitted} earlier messages or steps left out]\n\n"));
    }
    for entry in kept.iter().rev() {
        text.push_str(entry);
        text.push_str("\n\n");
    }
    text.push_str(&footer);
    Digest {
        text: text.trim_end().to_owned(),
        omitted,
    }
}

/// The messages a side question is sent as: the digest and the first
/// question together, then the earlier exchanges (the newest
/// [`THREAD_EXCHANGES`] that fit), then the new question.
pub fn side_messages(digest: &Digest, earlier: &[SideExchange], question: &str) -> Vec<SideChatMessage> {
    let mut kept: Vec<&SideExchange> = Vec::new();
    let mut used = 0;
    for exchange in earlier
        .iter()
        .rev()
        .filter(|exchange| exchange.answer.is_some())
        .take(THREAD_EXCHANGES)
    {
        let size = exchange.question.len() + exchange.answer.as_deref().map_or(0, str::len);
        if used + size > THREAD_BUDGET {
            break;
        }
        used += size;
        kept.push(exchange);
    }
    kept.reverse();

    let opening = |question: &str| {
        format!(
            "Here is the task's transcript so far, oldest first:\n<transcript>\n{}\n</transcript>\n\nMy question: {}",
            digest.text,
            question.trim()
        )
    };
    let mut messages = Vec::new();
    for exchange in kept {
        let text = if messages.is_empty() {
            opening(&exchange.question)
        } else {
            exchange.question.trim().to_owned()
        };
        messages.push(SideChatMessage { assistant: false, text });
        messages.push(SideChatMessage {
            assistant: true,
            text: exchange.answer.clone().unwrap_or_default(),
        });
    }
    let text = if messages.is_empty() {
        opening(question)
    } else {
        question.trim().to_owned()
    };
    messages.push(SideChatMessage { assistant: false, text });
    messages
}

/// The question of a `/btw <question>` composer line, or `None` for any
/// other line (`/btwx` included).
pub fn btw_question(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("/btw")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use waku_protocol::model::{ActivityItem, Message, ProviderKind};

    fn session_with(messages: Vec<Message>, blocks: Vec<TranscriptBlock>) -> AgentSession {
        let mut session = AgentSession::new(Uuid::nil(), ProviderKind::Native);
        session.messages = messages;
        session.transcript_blocks = blocks;
        session
    }

    fn activity(kind: ActivityKind, title: &str) -> ActivityItem {
        ActivityItem::new(None, kind, title, None, true)
    }

    #[test]
    fn the_digest_interleaves_work_and_messages_in_order() {
        let session = session_with(
            vec![
                Message::new(MessageRole::User, "fix the bug"),
                Message::new(MessageRole::Assistant, "done"),
            ],
            vec![TranscriptBlock {
                after_message: 1,
                turn_id: None,
                activities: vec![
                    activity(ActivityKind::Reasoning, "thinking hard"),
                    activity(ActivityKind::Command, "cargo test"),
                ],
            }],
        );
        let digest = build_digest(&session, DIGEST_BUDGET);
        let person = digest.text.find("fix the bug").unwrap();
        let command = digest.text.find("cargo test").unwrap();
        let agent = digest.text.find("done").unwrap();
        assert!(person < command && command < agent, "{}", digest.text);
        assert!(!digest.text.contains("thinking hard"));
        assert_eq!(digest.omitted, 0);
    }

    #[test]
    fn the_budget_keeps_the_newest_and_says_what_it_left_out() {
        let messages = (0..50)
            .map(|n| Message::new(MessageRole::User, format!("message {n} {}", "x".repeat(200))))
            .collect();
        let digest = build_digest(&session_with(messages, Vec::new()), 2_000);
        assert!(digest.omitted > 0);
        assert!(digest.text.contains("message 49"));
        assert!(!digest.text.contains("message 0 "));
        assert!(digest.text.starts_with('['));
        assert!(digest.text.len() < 2_600);
    }

    #[test]
    fn a_long_message_keeps_its_head_and_tail() {
        let long = format!("start{}end", "é".repeat(10_000));
        let capped = cap_text(&long, 100);
        assert!(capped.starts_with("start"));
        assert!(capped.ends_with("end"));
        assert!(capped.len() < 200);
    }

    #[test]
    fn the_thread_opens_with_the_digest_then_replays_answered_exchanges() {
        let digest = Digest {
            text: "T".into(),
            omitted: 0,
        };
        let first = side_messages(&digest, &[], "why?");
        assert_eq!(first.len(), 1);
        assert!(first[0].text.contains("<transcript>\nT\n</transcript>"));
        assert!(first[0].text.ends_with("My question: why?"));

        let earlier = [
            SideExchange { question: "q1".into(), answer: Some("a1".into()) },
            SideExchange { question: "lost".into(), answer: None },
        ];
        let next = side_messages(&digest, &earlier, "q2");
        assert_eq!(next.len(), 3);
        assert!(next[0].text.contains("My question: q1"));
        assert!(next[1].assistant);
        assert_eq!(next[2].text, "q2");
    }

    #[test]
    fn btw_takes_its_question_and_nothing_else() {
        assert_eq!(btw_question("/btw what is this?"), Some("what is this?"));
        assert_eq!(btw_question("  /btw"), Some(""));
        assert_eq!(btw_question("/btwx hi"), None);
        assert_eq!(btw_question("hello /btw"), None);
    }
}
