//! A turn split at the steering messages sent while it ran.
//!
//! Steering joins the running turn as another user message rather than
//! starting a new one, so a turn can hold several prompts. Each one begins a
//! segment of the turn's work with its own heading and its own clock:
//! "工作中 · 12s" under the latest, "已工作 1m 5s" folded under each before
//! it. Messages carry no marker for this; a steer is recognised by where it
//! sits — a user message of the turn that is not the prompt which opened it.

use uuid::Uuid;

use crate::model::{AgentSession, AgentTurn, MessageRole, TurnStatus};

/// One stretch of a turn's work, between two prompts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Segment {
    pub index: usize,
    /// The message index of the steer that opened it; `None` for the first
    /// segment, which the turn's own prompt (if any) opened.
    pub opener: Option<usize>,
    pub started_at: u64,
    /// `None` while it is the turn's live segment.
    pub ended_at: Option<u64>,
}

/// Message indices of the steering messages in a turn, in order.
///
/// The prompt that opened the turn is its first message when that is a user
/// message nothing of the turn rendered before. A turn the provider started
/// on its own — a goal carrying on — has no prompt: its first user message
/// arrives after work already shown, and is a steer like any later one.
pub fn steer_indexes(session: &AgentSession, turn_id: Uuid) -> Vec<usize> {
    let mut indices = session
        .messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.turn_id == Some(turn_id));
    let Some((first_index, first)) = indices.next() else {
        return Vec::new();
    };
    // A block anchored at `n` renders before message `n`.
    let work_before_first = session
        .transcript_blocks
        .iter()
        .any(|block| block.turn_id == Some(turn_id) && block.after_message <= first_index);
    let first_is_steer = first.role == MessageRole::User && work_before_first;
    std::iter::once((first_index, first))
        .filter(|_| first_is_steer)
        .chain(indices)
        .filter(|(_, message)| message.role == MessageRole::User)
        .map(|(index, _)| index)
        .collect()
}

/// The turn's segments, each bounded by the steer that opened it.
pub fn turn_segments(session: &AgentSession, turn: &AgentTurn) -> Vec<Segment> {
    let steers = steer_indexes(session, turn.id);
    let mut segments = Vec::with_capacity(steers.len() + 1);
    let mut started_at = turn.started_at;
    let mut opener = None;
    for (index, steer) in steers.iter().copied().enumerate() {
        let steered_at = session.messages[steer].created_at.max(started_at);
        segments.push(Segment {
            index,
            opener,
            started_at,
            ended_at: Some(steered_at),
        });
        started_at = steered_at;
        opener = Some(steer);
    }
    segments.push(Segment {
        index: steers.len(),
        opener,
        started_at,
        ended_at: turn.completed_at,
    });
    segments
}

/// The segment a row at `position` belongs to: a message's index, or a
/// block's anchor. Work rendered after a steer belongs to the segment it
/// opened.
pub fn segment_of(steers: &[usize], position: usize) -> usize {
    steers.iter().filter(|steer| **steer < position).count()
}

/// Whether a segment's work starts expanded, and whether the person may fold
/// it. The segment being worked on right now stays open and cannot fold; a
/// finished turn folds its work away; a failed or stopped one keeps it open,
/// since that work is the explanation.
pub fn segment_default_open(status: TurnStatus, is_last: bool) -> (bool, bool) {
    match status {
        TurnStatus::Running if is_last => (true, false),
        TurnStatus::Running | TurnStatus::Completed => (false, true),
        TurnStatus::Failed | TurnStatus::Interrupted => (true, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActivityItem, ActivityKind, Message, ProviderKind, TranscriptBlock};

    fn block(session: &mut AgentSession, turn_id: Uuid) {
        session.transcript_blocks.push(TranscriptBlock {
            after_message: session.messages.len(),
            turn_id: Some(turn_id),
            activities: vec![ActivityItem::new(
                None,
                ActivityKind::Tool,
                "work",
                None,
                true,
            )],
        });
    }

    fn message(session: &mut AgentSession, role: MessageRole, turn_id: Uuid, at: u64) -> usize {
        let mut message = Message::new_for_turn(role, "text", turn_id);
        message.created_at = at;
        session.messages.push(message);
        session.messages.len() - 1
    }

    #[test]
    fn a_steer_splits_the_turn_and_the_prompt_does_not() {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Native);
        let turn_id = session.begin_turn("prompt");
        session.turns[0].started_at = 100;
        block(&mut session, turn_id);
        let steer = message(&mut session, MessageRole::User, turn_id, 130);
        block(&mut session, turn_id);
        message(&mut session, MessageRole::Assistant, turn_id, 150);

        assert_eq!(steer_indexes(&session, turn_id), vec![steer]);
        let segments = turn_segments(&session, &session.turns[0]);
        assert_eq!(segments.len(), 2);
        assert_eq!(
            (segments[0].started_at, segments[0].ended_at),
            (100, Some(130))
        );
        assert_eq!(segments[1].opener, Some(steer));
        assert_eq!(segments[1].ended_at, None, "the live segment is still open");

        // The first block renders before the steer, the second after it.
        let steers = steer_indexes(&session, turn_id);
        assert_eq!(
            segment_of(&steers, session.transcript_blocks[0].after_message),
            0
        );
        assert_eq!(
            segment_of(&steers, session.transcript_blocks[1].after_message),
            1
        );
    }

    #[test]
    fn a_turn_the_provider_started_has_no_prompt_to_skip() {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Codex);
        let turn_id = session.begin_provider_turn();
        block(&mut session, turn_id);
        let steer = message(&mut session, MessageRole::User, turn_id, 10);
        assert_eq!(steer_indexes(&session, turn_id), vec![steer]);
    }

    #[test]
    fn a_turn_without_steers_is_one_segment() {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Native);
        let turn_id = session.begin_turn("prompt");
        block(&mut session, turn_id);
        assert!(steer_indexes(&session, turn_id).is_empty());
        assert_eq!(turn_segments(&session, &session.turns[0]).len(), 1);
    }

    #[test]
    fn live_work_stays_open_and_abnormal_endings_keep_theirs_open() {
        assert_eq!(
            segment_default_open(TurnStatus::Running, true),
            (true, false)
        );
        assert_eq!(
            segment_default_open(TurnStatus::Running, false),
            (false, true)
        );
        assert_eq!(
            segment_default_open(TurnStatus::Completed, true),
            (false, true)
        );
        assert_eq!(segment_default_open(TurnStatus::Failed, true), (true, true));
        assert_eq!(
            segment_default_open(TurnStatus::Interrupted, false),
            (true, true)
        );
    }
}
