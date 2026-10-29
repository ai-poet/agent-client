//! A built-in agent request being retried: what the working row says while
//! it waits, and taking back an answer that had started streaming when its
//! connection broke.
//!
//! Fork addition. The engine sends a failed request again on its own
//! (`claurst_api::retry_policy`) and reports each attempt as
//! [`DriverEvent::ModelRetry`]. When part of the failed answer had already
//! streamed, the engine threw it away — it never reached the conversation —
//! and the transcript has to drop it too, or the retried answer would appear
//! after the first half of the one that failed. Everything since the last
//! point the engine is known to have settled (the turn's start, a finished
//! request, a tool call, a compaction) is what goes; the person's own
//! messages and the notices in between stay.

use super::*;
use crate::model::RetryCause;

/// What "Continue" sends. English for the model, whatever the interface
/// language; the transcript shows `error_banner.continue_message` instead.
const CONTINUE_PROMPT: &str = "The previous turn stopped because of an error before it finished. \
     Continue from where it stopped. Do not repeat work that is already done; \
     if you are unsure what state things are in, check before changing anything.";

/// Whether a turn got any work done — a tool call ran — so continuing it
/// keeps something a retry would throw away.
pub(super) fn turn_did_work(session: &AgentSession, turn_id: Uuid) -> bool {
    session
        .transcript_blocks
        .iter()
        .filter(|block| block.turn_id == Some(turn_id))
        .flat_map(|block| block.activities.iter())
        .any(|activity| activity.reasoning.is_none())
}

/// Where the active turn's settled output ended, the last time the engine
/// was known to be between requests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct OutputBoundary {
    messages: usize,
    /// The last message's text length, when it was an assistant reply — a
    /// continuation can stream into the same reply.
    last_message_len: Option<usize>,
    blocks: usize,
    /// Activities in the last block.
    last_block_activities: usize,
    /// The last activity's reasoning length, when it is reasoning.
    last_reasoning_len: Option<usize>,
}

impl OutputBoundary {
    pub(super) fn capture(session: &AgentSession) -> Self {
        let last_message_len = session
            .messages
            .last()
            .filter(|message| message.role == MessageRole::Assistant)
            .map(|message| message.content.len());
        let last_block = session.transcript_blocks.last();
        Self {
            messages: session.messages.len(),
            last_message_len,
            blocks: session.transcript_blocks.len(),
            last_block_activities: last_block.map_or(0, |block| block.activities.len()),
            last_reasoning_len: last_block
                .and_then(|block| block.activities.last())
                .and_then(|activity| activity.reasoning.as_ref())
                .map(|reasoning| reasoning.content.len()),
        }
    }
}

/// Remove from the active turn the assistant text and reasoning that
/// streamed after `boundary`. Returns whether anything was removed.
pub(super) fn take_back_streamed_output(
    session: &mut AgentSession,
    boundary: &OutputBoundary,
) -> bool {
    let turn = session.active_turn_id();
    let mut changed = false;

    // Text streamed into the reply that was open at the boundary.
    if let Some(len) = boundary.last_message_len
        && let Some(message) = boundary
            .messages
            .checked_sub(1)
            .and_then(|index| session.messages.get_mut(index))
        && message.role == MessageRole::Assistant
        && message.content.len() > len
        && message.content.is_char_boundary(len)
    {
        message.content.truncate(len);
        changed = true;
    }

    // Replies of this turn opened since. The person's messages and the
    // notices stay; the blocks anchored after a removed reply move up.
    let start = boundary.messages.min(session.messages.len());
    let mut removed = Vec::new();
    let mut index = session.messages.len();
    while index > start {
        index -= 1;
        let message = &session.messages[index];
        if message.role == MessageRole::Assistant && message.turn_id == turn {
            session.messages.remove(index);
            removed.push(index);
        }
    }
    if !removed.is_empty() {
        changed = true;
        for block in &mut session.transcript_blocks {
            let shift = removed
                .iter()
                .filter(|&&index| index < block.after_message)
                .count();
            block.after_message -= shift;
        }
    }
    for message in &mut session.messages {
        message.streaming = false;
    }

    // Reasoning: what grew in the block that was last at the boundary, and
    // every reasoning row of this turn after it.
    if let Some(block) = boundary
        .blocks
        .checked_sub(1)
        .and_then(|index| session.transcript_blocks.get_mut(index))
    {
        if let Some(len) = boundary.last_reasoning_len
            && let Some(reasoning) = boundary
                .last_block_activities
                .checked_sub(1)
                .and_then(|index| block.activities.get_mut(index))
                .and_then(|activity| activity.reasoning.as_mut())
            && reasoning.content.len() > len
            && reasoning.content.is_char_boundary(len)
        {
            reasoning.content.truncate(len);
            changed = true;
        }
        let before = block.activities.len();
        let mut position = 0;
        block.activities.retain(|activity| {
            let keep = position < boundary.last_block_activities || activity.reasoning.is_none();
            position += 1;
            keep
        });
        changed |= block.activities.len() != before;
    }
    let first_new = boundary.blocks.min(session.transcript_blocks.len());
    let mut kept = Vec::with_capacity(session.transcript_blocks.len());
    for (index, mut block) in std::mem::take(&mut session.transcript_blocks)
        .into_iter()
        .enumerate()
    {
        if index >= first_new && block.turn_id == turn {
            let before = block.activities.len();
            block
                .activities
                .retain(|activity| activity.reasoning.is_none());
            changed |= block.activities.len() != before;
            if block.activities.is_empty() {
                continue;
            }
        }
        kept.push(block);
    }
    session.transcript_blocks = kept;
    changed
}

/// The retry the working row (or, between turns, the error banner) waits on.
#[derive(Clone, Debug)]
pub(super) struct RetryStatus {
    attempt: u32,
    max_attempts: Option<u32>,
    resume_at: Instant,
    /// Why it is being retried, read once from the engine's reason.
    cause: RetryCause,
    pub(super) between_turns: bool,
}

impl RetryStatus {
    fn seconds_left(&self) -> u64 {
        self.resume_at
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
            .ceil() as u64
    }

    /// Whether its wait is still running.
    pub(super) fn pending(&self) -> bool {
        self.resume_at > Instant::now()
    }

    /// "Rate limited · retrying in 8 s (2/10)…" for the working row while
    /// the wait runs, then "Sending again (2/10)…" while the request is out
    /// — not a countdown stuck at 0 s. An empty answer is asked again once,
    /// so it carries no count. The row redraws on the pulse clock already
    /// running for its dots; the switch costs no timer of its own.
    pub(super) fn working_label(&self) -> String {
        let waiting = self.pending();
        if self.cause == RetryCause::Empty {
            return if waiting {
                tr!("turn_status.empty_retry", seconds = self.seconds_left())
            } else {
                tr!("turn_status.empty_sending")
            };
        }
        match (waiting, self.max_attempts) {
            (true, Some(max)) => tr!(
                "turn_status.retry_waiting",
                cause = self.cause.label(),
                seconds = self.seconds_left(),
                attempt = self.attempt,
                max = max
            ),
            (true, None) => tr!(
                "turn_status.retry_waiting_unbounded",
                cause = self.cause.label(),
                seconds = self.seconds_left(),
                attempt = self.attempt
            ),
            (false, Some(max)) => tr!(
                "turn_status.retry_sending",
                attempt = self.attempt,
                max = max
            ),
            (false, None) => tr!(
                "turn_status.retry_sending_unbounded",
                attempt = self.attempt
            ),
        }
    }

    /// "The goal resumes on its own in 30 s (1/5)" for the error banner.
    pub(super) fn goal_label(&self) -> String {
        tr!(
            "error_banner.goal_auto_resume",
            seconds = self.seconds_left(),
            attempt = self.attempt,
            max = self.max_attempts.unwrap_or(self.attempt)
        )
    }
}

impl Waku {
    /// A built-in agent request is about to be sent again. Takes back what
    /// streamed of the failed answer, and keeps the retry for the status.
    pub(super) fn handle_model_retry(
        &mut self,
        session_id: Uuid,
        runtime: &mut SessionRuntime,
        retry: crate::model::ModelRetry,
    ) {
        let cause = RetryCause::from_reason(&retry.reason);
        if retry.discarded_output && self.accepts_turn_output(session_id) {
            let previous_kinds = self.snapshot_selected_transcript_rows(session_id);
            let boundary = runtime.output_boundary.clone();
            if let Some(session) = self.state.session_mut(session_id) {
                if let Some(boundary) = boundary {
                    take_back_streamed_output(session, &boundary);
                }
                session.push_message(
                    MessageRole::System,
                    match retry.max_attempts {
                        Some(max) => tr!(
                            "session.stream_discarded",
                            lead = cause.discarded_lead(),
                            attempt = retry.attempt,
                            max = max
                        ),
                        None => tr!(
                            "session.stream_discarded_unbounded",
                            lead = cause.discarded_lead(),
                            attempt = retry.attempt
                        ),
                    },
                );
                session.updated_at = unix_time();
            }
            runtime.stream_phase = None;
            self.state.mark_session_dirty(session_id);
            if let Some(previous_kinds) = previous_kinds.as_deref() {
                self.splice_active_transcript_rows_after_visibility_change(previous_kinds);
            }
        }
        runtime.model_retry = Some(RetryStatus {
            attempt: retry.attempt,
            max_attempts: retry.max_attempts,
            resume_at: Instant::now() + Duration::from_millis(retry.delay_ms),
            cause,
            between_turns: retry.between_turns,
        });
        self.note_output_boundary(session_id, runtime);
    }

    /// The engine is between requests: what the turn shows now is settled.
    pub(super) fn note_output_boundary(&self, session_id: Uuid, runtime: &mut SessionRuntime) {
        runtime.output_boundary = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(OutputBoundary::capture);
    }

    /// Pick a failed turn up where it stopped, in a new turn that keeps
    /// the conversation — the work its tool rounds finished is in it. Retry
    /// rewinds and asks the opening prompt again, which throws that work
    /// away; after a long turn that is rarely what the person wants.
    pub(super) fn continue_failed_turn(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        self.submit_submission_for_session(
            session_id,
            ComposerSubmission {
                prompt: CONTINUE_PROMPT.to_owned(),
                display_content: Some(tr!("error_banner.continue_message")),
                attachments: Vec::new(),
            },
            cx,
        );
    }

    /// A turn that failed because the conversation no longer fits: compact
    /// it (the built-in agent's own `/compact`), and when the turn had got
    /// work done, queue "Continue" behind it — the queue drains only after
    /// the compaction succeeds. Held for a route lookup, it only compacts.
    pub(super) fn compact_failed_turn(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let did_work = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| Some((session, session.turns.last()?.id)))
            .is_some_and(|(session, turn_id)| turn_did_work(session, turn_id));
        self.submit_submission_for_session(
            session_id,
            ComposerSubmission {
                prompt: "/compact".to_owned(),
                display_content: None,
                attachments: Vec::new(),
            },
            cx,
        );
        let compacting = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .is_some_and(AgentSession::is_busy);
        if did_work && compacting {
            self.enqueue_follow_up_submission(
                session_id,
                ComposerSubmission {
                    prompt: CONTINUE_PROMPT.to_owned(),
                    display_content: Some(tr!("error_banner.continue_message")),
                    attachments: Vec::new(),
                },
                cx,
            );
        }
    }

    /// The goal's automatic resume waiting after a failed turn, for the
    /// error banner — while its goal is still being pursued.
    pub(super) fn pending_goal_resume(&self, session: &AgentSession) -> Option<String> {
        let status = self.runtimes.get(&session.id)?.model_retry.as_ref()?;
        let active = session
            .thread_goal
            .as_ref()
            .is_some_and(|goal| goal.status == crate::model::ThreadGoalStatus::Active);
        (status.between_turns && status.pending() && active).then(|| status.goal_label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retry(cause: RetryCause, max_attempts: Option<u32>, wait: Duration) -> RetryStatus {
        RetryStatus {
            attempt: 2,
            max_attempts,
            resume_at: Instant::now() + wait,
            cause,
            between_turns: false,
        }
    }

    #[test]
    fn a_retry_reads_as_sending_once_its_wait_is_over() {
        let waiting = retry(RetryCause::RateLimited, Some(10), Duration::from_secs(30));
        assert_eq!(
            waiting.working_label(),
            tr!(
                "turn_status.retry_waiting",
                cause = RetryCause::RateLimited.label(),
                seconds = 30,
                attempt = 2,
                max = 10
            )
        );
        let sent = retry(RetryCause::RateLimited, Some(10), Duration::ZERO);
        assert_eq!(
            sent.working_label(),
            tr!("turn_status.retry_sending", attempt = 2, max = 10)
        );
        let unbounded = retry(RetryCause::Busy, None, Duration::ZERO);
        assert_eq!(
            unbounded.working_label(),
            tr!("turn_status.retry_sending_unbounded", attempt = 2)
        );
    }

    #[test]
    fn an_empty_answer_is_asked_again_without_a_count() {
        let waiting = retry(RetryCause::Empty, Some(1), Duration::from_secs(2));
        assert_eq!(
            waiting.working_label(),
            tr!("turn_status.empty_retry", seconds = 2)
        );
        let sent = retry(RetryCause::Empty, Some(1), Duration::ZERO);
        assert_eq!(sent.working_label(), tr!("turn_status.empty_sending"));
        assert!(!sent.working_label().contains("1/1"));
    }

    fn session_with_turn() -> (AgentSession, Uuid) {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Native);
        let turn = session.begin_provider_turn();
        (session, turn)
    }

    fn reasoning(content: &str) -> ActivityItem {
        ActivityItem::from_reasoning(
            ReasoningBlock {
                content: content.to_owned(),
                started_at_ms: 0,
                finished_at_ms: 0,
            },
            false,
        )
    }

    #[test]
    fn a_retried_answer_takes_back_what_streamed_of_the_failed_one() {
        let (mut session, turn) = session_with_turn();
        session.push_message(MessageRole::User, "fix the bug");
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 1,
            turn_id: Some(turn),
            activities: vec![reasoning("settled thought")],
        });
        let boundary = OutputBoundary::capture(&session);

        // The failed attempt: a reasoning row, half a reply, then a steer.
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 1,
            turn_id: Some(turn),
            activities: vec![reasoning("lost thought")],
        });
        let mut reply = Message::new_for_turn(MessageRole::Assistant, "Half an ans", turn);
        reply.streaming = true;
        session.messages.push(reply);
        session.push_message(MessageRole::User, "also check the tests");

        assert!(take_back_streamed_output(&mut session, &boundary));
        let texts: Vec<&str> = session
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(texts, vec!["fix the bug", "also check the tests"]);
        assert_eq!(session.transcript_blocks.len(), 1);
        assert_eq!(
            session.transcript_blocks[0].activities[0]
                .reasoning
                .as_ref()
                .map(|r| r.content.as_str()),
            Some("settled thought")
        );
        // Nothing left to take back.
        let settled = OutputBoundary::capture(&session);
        assert!(!take_back_streamed_output(&mut session, &settled));
    }

    #[test]
    fn text_and_reasoning_grown_past_the_boundary_are_cut_back() {
        let (mut session, turn) = session_with_turn();
        session.messages.push(Message::new_for_turn(
            MessageRole::Assistant,
            "Part one. ",
            turn,
        ));
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 0,
            turn_id: Some(turn),
            activities: vec![reasoning("thinking")],
        });
        let boundary = OutputBoundary::capture(&session);

        session.messages[0].content.push_str("Part tw");
        session.transcript_blocks[0].activities[0]
            .reasoning
            .as_mut()
            .expect("reasoning")
            .content
            .push_str(" more");

        assert!(take_back_streamed_output(&mut session, &boundary));
        assert_eq!(session.messages[0].content, "Part one. ");
        assert_eq!(
            session.transcript_blocks[0].activities[0]
                .reasoning
                .as_ref()
                .map(|r| r.content.as_str()),
            Some("thinking")
        );
    }

    #[test]
    fn tool_rows_and_other_turns_are_never_touched() {
        let (mut session, turn) = session_with_turn();
        let boundary = OutputBoundary::capture(&session);
        let tool = ActivityItem::new(None, ActivityKind::Tool, "Read", None, true);
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 0,
            turn_id: Some(turn),
            activities: vec![tool.clone(), reasoning("x")],
        });
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 0,
            turn_id: Some(Uuid::new_v4()),
            activities: vec![reasoning("another turn")],
        });
        assert!(take_back_streamed_output(&mut session, &boundary));
        assert_eq!(session.transcript_blocks.len(), 2);
        assert_eq!(session.transcript_blocks[0].activities.len(), 1);
        assert!(
            session.transcript_blocks[0].activities[0]
                .reasoning
                .is_none()
        );
        assert_eq!(session.transcript_blocks[1].activities.len(), 1);
    }
}
