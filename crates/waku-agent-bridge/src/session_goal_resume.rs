//! Fork addition: an autonomous goal resumes on its own after a turn that
//! failed on something transient.
//!
//! A goal runs unattended — that is the point of it — and the engine
//! already retries each request through its policy. When even that runs
//! out (the network was down for minutes, say), the turn ends as failed and
//! the goal used to sit there until the person came back. Instead the
//! session waits and sends the goal's own continuation message again, a few
//! times with growing pauses. The text is the one the engine sends between
//! goal turns, so the prompt cache is unaffected.
//!
//! Anything the person does — a message, Stop, a goal change, closing the
//! session — cancels a resume that is waiting.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, Inner};
use crate::events::AgentEvent;

/// Resumes after failed turns in a row before the goal waits for the person.
pub(super) const MAX_RESUMES: u32 = 5;

/// The pause before each resume.
const DELAYS_SECS: [u64; MAX_RESUMES as usize] = [30, 60, 120, 240, 300];

/// A session's pending resume and its run of failures.
#[derive(Default)]
pub(super) struct GoalResume {
    /// The resume waiting to run, by its number.
    pending: Mutex<Option<(u64, CancellationToken)>>,
    armed: AtomicU64,
    failures: AtomicU32,
}

impl GoalResume {
    /// Count a failed goal turn; the resume number and its pause, or `None`
    /// once the goal has failed too often in a row.
    fn next(&self) -> Option<(u32, Duration)> {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        let delay = DELAYS_SECS.get(failures as usize - 1)?;
        Some((failures, Duration::from_secs(*delay)))
    }

    /// A goal turn went through: the next failure starts the count again.
    fn succeeded(&self) {
        self.failures.store(0, Ordering::SeqCst);
    }

    /// Wait for a new resume, cancelling any older one. Returns its number
    /// and the token that cancels it.
    fn arm(&self) -> (u64, CancellationToken) {
        let id = self.armed.fetch_add(1, Ordering::SeqCst) + 1;
        let token = CancellationToken::new();
        if let Some((_, previous)) = self.pending.lock().replace((id, token.clone())) {
            previous.cancel();
        }
        (id, token)
    }

    /// Whether resume `id` is still the one to run; takes it if so.
    fn take(&self, id: u64) -> bool {
        let mut pending = self.pending.lock();
        if pending.as_ref().is_some_and(|(current, _)| *current == id) {
            *pending = None;
            true
        } else {
            false
        }
    }

    /// Cancel a waiting resume, if any. `reset` also forgets the failures —
    /// the person stopped the run, so the next one starts fresh.
    pub(super) fn cancel(&self, reset: bool) {
        if let Some((_, token)) = self.pending.lock().take() {
            token.cancel();
        }
        if reset {
            self.succeeded();
        }
    }
}

/// After a turn: count a goal turn's outcome, and schedule a resume for one
/// that failed on something transient while its goal is still active.
pub(super) fn after_turn(
    inner: &Arc<Inner>,
    goal_mode: bool,
    success: bool,
    transient_failure: bool,
    cancelled: bool,
) {
    if !goal_mode || cancelled {
        return;
    }
    if success {
        inner.goal_resume.succeeded();
        return;
    }
    if !transient_failure {
        return;
    }
    let Some(goal) = crate::goal::open_store().and_then(|store| store.get_active_goal(&inner.id))
    else {
        return;
    };
    let Some((attempt, delay)) = inner.goal_resume.next() else {
        return;
    };
    let Ok(rt) = crate::runtime::shared() else {
        return;
    };
    let message = claurst_core::goal_continuation_message(&goal);
    let (id, token) = inner.goal_resume.arm();
    let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
    // Fork: on the record of the turn that failed (`crate::trajectory`).
    inner
        .trajectory
        .goal_resume(attempt, Some(MAX_RESUMES), delay_ms);
    inner.events.emit(AgentEvent::Retry {
        attempt,
        max_attempts: Some(MAX_RESUMES),
        delay_ms,
        reason: "turn_failed".to_string(),
        discarded_output: false,
        detail: String::new(),
        between_turns: true,
    });
    // Weak: a closed session must not be kept alive by its own timer.
    let weak: Weak<Inner> = Arc::downgrade(inner);
    rt.spawn(async move {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        let Some(inner) = weak.upgrade() else {
            return;
        };
        if !inner.goal_resume.take(id) {
            return;
        }
        // The goal may have been paused or finished meanwhile, or a turn
        // started some other way.
        let still_active = crate::goal::open_store()
            .and_then(|store| store.get_active_goal(&inner.id))
            .is_some();
        if !still_active || inner.turn.lock().is_some() {
            return;
        }
        AgentSession { inner }.prompt(message);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumes_wait_longer_each_time_and_stop_after_five() {
        let resume = GoalResume::default();
        let delays: Vec<u64> = std::iter::from_fn(|| resume.next())
            .map(|(_, delay)| delay.as_secs())
            .collect();
        assert_eq!(delays, vec![30, 60, 120, 240, 300]);
        assert!(resume.next().is_none());
        resume.succeeded();
        assert_eq!(resume.next(), Some((1, Duration::from_secs(30))));
    }

    #[test]
    fn a_newer_or_cancelled_resume_supersedes_the_old_one() {
        let resume = GoalResume::default();
        let (first, first_token) = resume.arm();
        let (second, _) = resume.arm();
        assert!(first_token.is_cancelled());
        assert!(!resume.take(first));
        assert!(resume.take(second));
        // Taken once.
        assert!(!resume.take(second));

        let (third, third_token) = resume.arm();
        resume.cancel(false);
        assert!(third_token.is_cancelled());
        assert!(!resume.take(third));
    }
}
