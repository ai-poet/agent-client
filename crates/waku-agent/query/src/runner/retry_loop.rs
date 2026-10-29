// Fork addition (Waku): the query loop's side of `claurst_api::retry_policy`
// — how many attempts the request now going out has left, whether an answer
// that had started streaming may be thrown away and asked again, and the
// cancellable wait before the next attempt.

use std::time::Duration;

use claurst_api::retry_policy::{
    backoff_delay, FailureClass, ModelFailure, RetryBudget, RetryReason,
    UNBOUNDED_SERVER_ERROR_LIMIT,
};
use tokio_util::sync::CancellationToken;

/// What to do after a failed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    /// Wait `delay`, then send the request again. `attempt` counts this
    /// request's retries so far (1-based); `max` is `None` without a limit.
    Retry {
        attempt: u32,
        max: Option<u32>,
        delay: Duration,
    },
    /// The conversation no longer fits: compact it, then send again.
    Compact,
    /// Stop and report the failure.
    GiveUp,
}

/// Per-prompt retry bookkeeping.
///
/// A request's retries are counted until it succeeds, and start again for
/// the next one — a two-hundred-step task gets the full budget on every
/// step, not two retries for the whole run. Throwing away an answer that had
/// started streaming is counted per prompt as well, because each one costs
/// the user a visible restart.
#[derive(Debug)]
pub(crate) struct RetryState {
    budget: RetryBudget,
    request_retries: u32,
    recoveries: u32,
    server_errors_in_row: u32,
    empty_retries: u32,
    overflow_compactions: u32,
}

impl RetryState {
    pub(crate) fn new(budget: RetryBudget) -> Self {
        Self {
            budget,
            request_retries: 0,
            recoveries: 0,
            server_errors_in_row: 0,
            empty_retries: 0,
            overflow_compactions: 0,
        }
    }

    /// Decide on a failed attempt. `visible_output` says whether the user had
    /// already seen part of its answer, which the retry will take back.
    pub(crate) fn on_failure(
        &mut self,
        failure: &ModelFailure,
        visible_output: bool,
    ) -> RetryDecision {
        match failure.class {
            FailureClass::Fatal => RetryDecision::GiveUp,
            FailureClass::ContextOverflow => {
                // Once per request: a compacted conversation that still does
                // not fit will not fit after a second pass either.
                if self.overflow_compactions == 0 {
                    self.overflow_compactions += 1;
                    RetryDecision::Compact
                } else {
                    RetryDecision::GiveUp
                }
            }
            FailureClass::Retryable => {
                if visible_output {
                    if self
                        .budget
                        .recoveries_per_prompt()
                        .is_some_and(|max| self.recoveries >= max)
                    {
                        return RetryDecision::GiveUp;
                    }
                }
                let max = self.budget.retries_per_request();
                if max.is_some_and(|max| self.request_retries >= max) {
                    return RetryDecision::GiveUp;
                }
                if self.budget == RetryBudget::Unbounded {
                    if failure.reason == RetryReason::ServerError {
                        self.server_errors_in_row += 1;
                        if self.server_errors_in_row > UNBOUNDED_SERVER_ERROR_LIMIT {
                            return RetryDecision::GiveUp;
                        }
                    } else {
                        self.server_errors_in_row = 0;
                    }
                }
                if visible_output {
                    self.recoveries += 1;
                }
                self.request_retries += 1;
                RetryDecision::Retry {
                    attempt: self.request_retries,
                    max,
                    delay: backoff_delay(self.budget, self.request_retries, failure.retry_after),
                }
            }
        }
    }

    /// An empty completion — no text, no reasoning, no tool call — gets one
    /// silent retry per request.
    pub(crate) fn on_empty(&mut self) -> Option<Duration> {
        if self.empty_retries >= 1 || self.budget == RetryBudget::SingleAttempt {
            return None;
        }
        self.empty_retries += 1;
        Some(backoff_delay(self.budget, 1, None))
    }

    /// A request went through: the next one starts with a full budget.
    pub(crate) fn on_success(&mut self) {
        self.request_retries = 0;
        self.server_errors_in_row = 0;
        self.empty_retries = 0;
        self.overflow_compactions = 0;
    }

    /// The attempt number the next request carries (1 for a first attempt),
    /// for the servers that read it from a header.
    pub(crate) fn next_attempt(&self) -> u32 {
        self.request_retries + 1
    }
}

/// Wait out `delay` unless the run is cancelled first. Returns `true` when
/// it was cancelled.
pub(crate) async fn sleep_or_cancel(delay: Duration, cancel: &CancellationToken) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retryable() -> ModelFailure {
        ModelFailure::retryable(RetryReason::Network, "reset")
    }

    #[test]
    fn a_request_gets_its_budget_and_the_next_one_starts_fresh() {
        let mut state = RetryState::new(RetryBudget::Default);
        for attempt in 1..=10 {
            match state.on_failure(&retryable(), false) {
                RetryDecision::Retry {
                    attempt: got, max, ..
                } => {
                    assert_eq!(got, attempt);
                    assert_eq!(max, Some(10));
                }
                other => panic!("attempt {attempt}: {other:?}"),
            }
        }
        assert_eq!(state.on_failure(&retryable(), false), RetryDecision::GiveUp);

        state.on_success();
        assert!(matches!(
            state.on_failure(&retryable(), false),
            RetryDecision::Retry { attempt: 1, .. }
        ));
    }

    #[test]
    fn taking_back_streamed_output_is_limited_per_prompt() {
        let mut state = RetryState::new(RetryBudget::Default);
        for _ in 0..10 {
            assert!(matches!(
                state.on_failure(&retryable(), true),
                RetryDecision::Retry { .. }
            ));
            state.on_success();
        }
        assert_eq!(state.on_failure(&retryable(), true), RetryDecision::GiveUp);
        // A failure before anything was shown still retries.
        assert!(matches!(
            state.on_failure(&retryable(), false),
            RetryDecision::Retry { .. }
        ));

        let mut light = RetryState::new(RetryBudget::Light);
        assert_eq!(light.on_failure(&retryable(), true), RetryDecision::GiveUp);
    }

    #[test]
    fn fatal_gives_up_and_overflow_compacts_once_per_request() {
        let mut state = RetryState::new(RetryBudget::Default);
        assert_eq!(
            state.on_failure(&ModelFailure::fatal(Some(403), "balance"), false),
            RetryDecision::GiveUp
        );
        let overflow = ModelFailure::context_overflow(Some(400), "prompt is too long");
        assert_eq!(state.on_failure(&overflow, false), RetryDecision::Compact);
        assert_eq!(state.on_failure(&overflow, false), RetryDecision::GiveUp);
        state.on_success();
        assert_eq!(state.on_failure(&overflow, false), RetryDecision::Compact);
    }

    #[test]
    fn unbounded_runs_until_server_errors_pile_up() {
        let mut state = RetryState::new(RetryBudget::Unbounded);
        for _ in 0..100 {
            assert!(matches!(
                state.on_failure(&retryable(), false),
                RetryDecision::Retry { max: None, .. }
            ));
        }
        let server = ModelFailure::retryable(RetryReason::ServerError, "502");
        for _ in 0..UNBOUNDED_SERVER_ERROR_LIMIT {
            assert!(matches!(
                state.on_failure(&server, false),
                RetryDecision::Retry { .. }
            ));
        }
        assert_eq!(state.on_failure(&server, false), RetryDecision::GiveUp);
    }

    #[test]
    fn an_empty_completion_is_retried_once_per_request() {
        let mut state = RetryState::new(RetryBudget::Default);
        assert!(state.on_empty().is_some());
        assert!(state.on_empty().is_none());
        state.on_success();
        assert!(state.on_empty().is_some());
        assert!(RetryState::new(RetryBudget::SingleAttempt)
            .on_empty()
            .is_none());
    }

    #[tokio::test]
    async fn a_cancel_ends_the_wait() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(sleep_or_cancel(Duration::from_secs(3600), &cancel).await);
        assert!(!sleep_or_cancel(Duration::from_millis(1), &CancellationToken::new()).await);
    }
}
