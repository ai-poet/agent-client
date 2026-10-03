//! What the live turn is doing, said beside how long it has run.
//!
//! Fork addition, after dsh-claude-style's turn-status line (MIT, © Nwflower):
//! the working row reads "Working for 1m 5s · 2.3k tokens · Thinking…" instead
//! of the bare time, so a long silence says whether the model is reasoning,
//! a tool is running, or the turn is waiting on the person.
//!
//! The action is derived from what the transcript already holds — the stream
//! phase and the turn's unfinished calls — so it works for every provider.
//! The token count is the turn's *output* tokens, read from the running total
//! the agent reports (`TokenUsageUpdated`; today only the built-in agent does)
//! minus the total when the turn's first report arrived. Nothing is
//! estimated: a provider that reports no usage shows no count.

use super::*;

/// The one thing the live turn is doing right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TurnAction {
    /// An approval or a question is waiting on the person.
    WaitingForYou,
    Thinking,
    /// Calls that have started and not finished.
    RunningTools(usize),
    Writing,
    /// Between steps: the request is out and nothing has streamed back yet.
    WaitingForModel,
}

impl TurnAction {
    fn label(self) -> String {
        match self {
            TurnAction::WaitingForYou => tr!("turn_status.waiting_for_you"),
            TurnAction::Thinking => tr!("turn_status.thinking"),
            TurnAction::RunningTools(1) => tr!("turn_status.running_tool"),
            TurnAction::RunningTools(count) => tr!("turn_status.running_tools", count = count),
            TurnAction::Writing => tr!("turn_status.writing"),
            TurnAction::WaitingForModel => tr!("turn_status.waiting_for_model"),
        }
    }
}

/// Precedence: the person first (nothing moves until they answer), then the
/// stream's own phase, then calls in flight.
pub(super) fn turn_action(
    phase: Option<StreamPhase>,
    running_tools: usize,
    waiting_on_user: bool,
) -> TurnAction {
    if waiting_on_user {
        return TurnAction::WaitingForYou;
    }
    match phase {
        Some(StreamPhase::Reasoning) => TurnAction::Thinking,
        _ if running_tools > 0 => TurnAction::RunningTools(running_tools),
        Some(StreamPhase::Text) => TurnAction::Writing,
        Some(StreamPhase::Activity) | None => TurnAction::WaitingForModel,
    }
}

/// Output tokens since the baseline, read from the agent's running total.
/// A total below the baseline means the runtime restarted and its count
/// began again; then the whole new total belongs to this turn.
pub(super) fn turn_output_tokens(total: u64, baseline: u64) -> u64 {
    if total >= baseline {
        total - baseline
    } else {
        total
    }
}

impl Waku {
    /// Called before a `TokenUsageUpdated` is applied: the first report of a
    /// turn pins the total it started from.
    pub(super) fn note_turn_output_baseline(&mut self, session_id: Uuid) {
        let Some(session) = self.state.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let Some(turn_id) = session.active_turn_id() else {
            return;
        };
        if self
            .turn_output_baselines
            .get(&session_id)
            .is_some_and(|(pinned, _)| *pinned == turn_id)
        {
            return;
        }
        let total = session
            .context_usage
            .as_ref()
            .and_then(|usage| usage.session)
            .map_or(0, |totals| totals.output);
        self.turn_output_baselines
            .insert(session_id, (turn_id, total));
    }

    /// " · 2.3k tokens · Thinking…" for the selected session's live turn.
    pub(super) fn live_turn_status_suffix(&self) -> String {
        let Some(session) = self.selected_session() else {
            return String::new();
        };
        let Some(turn_id) = session.active_turn_id() else {
            return String::new();
        };
        let runtime = self.selected_runtime();
        let waiting_on_user = session.status == SessionStatus::Waiting
            || runtime.is_some_and(|runtime| {
                !runtime.pending_permissions.is_empty() || runtime.pending_user_input.is_some()
            });
        let running_tools = session
            .transcript_blocks
            .iter()
            .rev()
            .take_while(|block| block.turn_id == Some(turn_id))
            .flat_map(|block| block.activities.iter())
            .filter(|activity| {
                activity.reasoning.is_none()
                    && !activity.complete
                    && !activity.failed
                    && !activity.stopped
            })
            .count();
        let action = turn_action(
            runtime.and_then(|runtime| runtime.stream_phase),
            running_tools,
            waiting_on_user,
        );

        let mut suffix = String::new();
        let tokens = self
            .turn_output_baselines
            .get(&session.id)
            .filter(|(pinned, _)| *pinned == turn_id)
            .zip(
                session
                    .context_usage
                    .as_ref()
                    .and_then(|usage| usage.session),
            )
            .map(|((_, baseline), totals)| turn_output_tokens(totals.output, *baseline))
            .unwrap_or(0);
        if tokens > 0 {
            suffix.push_str(" · ");
            suffix.push_str(&tr!(
                "turn_status.tokens",
                count = crate::usage::format_tokens(tokens)
            ));
        }
        suffix.push_str(" · ");
        suffix.push_str(&action.label());
        suffix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_person_outranks_everything() {
        assert_eq!(
            turn_action(Some(StreamPhase::Reasoning), 2, true),
            TurnAction::WaitingForYou
        );
    }

    #[test]
    fn reasoning_reads_as_thinking_even_beside_running_calls() {
        assert_eq!(
            turn_action(Some(StreamPhase::Reasoning), 1, false),
            TurnAction::Thinking
        );
    }

    #[test]
    fn calls_in_flight_outrank_the_last_phase() {
        assert_eq!(
            turn_action(Some(StreamPhase::Activity), 3, false),
            TurnAction::RunningTools(3)
        );
        assert_eq!(
            turn_action(Some(StreamPhase::Text), 1, false),
            TurnAction::RunningTools(1)
        );
    }

    #[test]
    fn text_streaming_is_writing_and_silence_is_waiting() {
        assert_eq!(
            turn_action(Some(StreamPhase::Text), 0, false),
            TurnAction::Writing
        );
        assert_eq!(
            turn_action(Some(StreamPhase::Activity), 0, false),
            TurnAction::WaitingForModel
        );
        assert_eq!(turn_action(None, 0, false), TurnAction::WaitingForModel);
    }

    #[test]
    fn output_tokens_are_the_growth_since_the_baseline() {
        assert_eq!(turn_output_tokens(5_400, 3_000), 2_400);
        assert_eq!(turn_output_tokens(3_000, 3_000), 0);
        // The runtime restarted and counts from zero again.
        assert_eq!(turn_output_tokens(800, 3_000), 800);
    }
}
