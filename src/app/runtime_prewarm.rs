//! Warming a session's provider process while the user is still typing.
//!
//! Fork addition. Upstream starts the provider lazily, when the first prompt
//! is submitted, and a Node-based CLI takes two to six seconds to boot — on
//! a first message that boot is most of the wait between Enter and the first
//! request leaving the machine. Typing is a good enough declaration of
//! intent: the first edit to the composer starts the runtime exactly the way
//! the submission would have, and the submission then finds it installed. If
//! the boot is still in flight when Enter lands, the submission waits for
//! that boot instead of racing a second process into existence.
//!
//! A warmed runtime is an ordinary idle runtime — the same thing a session
//! holds between two turns — so option changes, provider changes, deletion
//! and idle reaping all treat it as they would any other. The one addition
//! is a shorter leash: a runtime warmed for a prompt that never came is
//! closed after ten minutes rather than the usual half hour.

use std::sync::mpsc;

use super::*;

/// How long a warmed runtime that never carried a prompt is kept.
const UNUSED_PREWARM_TTL: Duration = Duration::from_secs(10 * 60);

/// How long a submission waits for an in-flight warm start before giving up
/// on it and starting a process of its own. Generous: the point of waiting
/// is that the warm process is nearly ready.
const PREWARM_HANDOFF_TIMEOUT: Duration = Duration::from_secs(45);

/// A warm start that has not landed yet.
struct PrewarmInFlight {
    provider: ProviderKind,
    /// Set when a submission arrived meanwhile; the result goes to it rather
    /// than into the runtime map.
    waiter: Option<mpsc::Sender<anyhow::Result<PreparedDriver>>>,
}

#[derive(Default)]
pub(super) struct RuntimePrewarms {
    in_flight: HashMap<Uuid, PrewarmInFlight>,
    /// Runtimes a warm start installed that have not carried a prompt yet,
    /// with when they were installed.
    unused: HashMap<Uuid, Instant>,
}

impl Waku {
    /// Start the selected session's provider if the composer holds a draft
    /// and nothing is running or starting for it yet. Called on every edit;
    /// every early return here is a field read, never I/O.
    pub(super) fn maybe_prewarm_selected_runtime(&mut self, cx: &mut Context<Self>) {
        if self.composer.read(cx).content(cx).trim().is_empty() {
            return;
        }
        let Some(session) = self.selected_session() else {
            return;
        };
        let session_id = session.id;
        if !matches!(session.status, SessionStatus::Idle | SessionStatus::Failed)
            || self.runtimes.contains_key(&session_id)
            || self.runtime_prewarms.in_flight.contains_key(&session_id)
            || self.submission_preparations.contains(&session_id)
            || self.goal_runtime_starts.contains(&session_id)
            || self.response_fork_preparations.contains_key(&session_id)
        {
            return;
        }
        // A worktree that does not exist yet is created by the submission;
        // there is no directory to start the provider in before that.
        if matches!(session.workspace, SessionWorkspace::NewWorktree { .. }) {
            return;
        }
        let Some(cwd) = self
            .workspace_path_for_session(session)
            .map(Path::to_path_buf)
        else {
            return;
        };
        // No binary for this provider: the submission will say so; a warm
        // start has nothing to add.
        let Ok(request) = self.driver_start_request_for_session(session, cwd.clone()) else {
            return;
        };
        let provider = session.provider;
        self.runtime_prewarms.in_flight.insert(
            session_id,
            PrewarmInFlight {
                provider,
                waiter: None,
            },
        );
        cx.spawn(async move |waku, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move { super::runtime::start_driver(request, cwd) })
                .await;
            let _ = waku.update(cx, |waku, cx| {
                waku.finish_runtime_prewarm(session_id, prepared, cx);
            });
        })
        .detach();
    }

    fn finish_runtime_prewarm(
        &mut self,
        session_id: Uuid,
        prepared: anyhow::Result<PreparedDriver>,
        cx: &mut Context<Self>,
    ) {
        let Some(in_flight) = self.runtime_prewarms.in_flight.remove(&session_id) else {
            if let Ok(prepared) = prepared {
                prepared.handle.close();
            }
            return;
        };
        if let Some(waiter) = in_flight.waiter {
            // A submission is waiting on the background executor; it owns the
            // process from here. A receiver that is already gone means the
            // submission stopped waiting and started its own.
            if let Err(mpsc::SendError(Ok(prepared))) = waiter.send(prepared) {
                prepared.handle.close();
            }
            return;
        }
        // A failed warm start is not reported: the submission that follows
        // runs the same start and surfaces the same error in context.
        let Ok(prepared) = prepared else {
            return;
        };
        let still_wanted = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .is_some_and(|session| {
                session.provider == in_flight.provider
                    && matches!(session.status, SessionStatus::Idle | SessionStatus::Failed)
            })
            && !self.runtimes.contains_key(&session_id)
            && !self.goal_runtime_starts.contains(&session_id)
            && !self.submission_preparations.contains(&session_id);
        if !still_wanted {
            prepared.handle.close();
            return;
        }
        self.install_prepared_driver(session_id, prepared);
        self.runtime_prewarms
            .unused
            .insert(session_id, Instant::now());
        cx.notify();
    }

    /// For the submission path: when a warm start is in flight for the
    /// session, a receiver its process will be handed through. `None` when
    /// there is nothing to wait for.
    pub(super) fn take_prewarm_handoff(
        &mut self,
        session_id: Uuid,
    ) -> Option<mpsc::Receiver<anyhow::Result<PreparedDriver>>> {
        let in_flight = self.runtime_prewarms.in_flight.get_mut(&session_id)?;
        let (sender, receiver) = mpsc::channel();
        in_flight.waiter = Some(sender);
        Some(receiver)
    }

    /// The runtime carried a prompt; it is a working runtime now, on the
    /// ordinary idle rules.
    pub(super) fn note_runtime_prompted(&mut self, session_id: Uuid) {
        self.runtime_prewarms.unused.remove(&session_id);
    }

    /// Close warmed runtimes nobody prompted within the leash. Runs with the
    /// idle sweep.
    pub(super) fn reap_unused_prewarms(&mut self) {
        let expired: Vec<Uuid> = self
            .runtime_prewarms
            .unused
            .iter()
            .filter(|(session_id, installed_at)| {
                installed_at.elapsed() >= UNUSED_PREWARM_TTL
                    && self
                        .state
                        .sessions
                        .iter()
                        .find(|session| session.id == **session_id)
                        .is_none_or(|session| session.active_turn_id().is_none())
            })
            .map(|(session_id, _)| *session_id)
            .collect();
        for session_id in expired {
            self.runtime_prewarms.unused.remove(&session_id);
            if let Some(runtime) = self.runtimes.remove(&session_id) {
                runtime.driver.close();
            }
        }
    }
}

/// Wait for a warm start's process. `None` when it did not arrive in time,
/// in which case the caller starts its own.
pub(super) fn await_prewarmed_driver(
    handoff: mpsc::Receiver<anyhow::Result<PreparedDriver>>,
) -> Option<anyhow::Result<PreparedDriver>> {
    handoff.recv_timeout(PREWARM_HANDOFF_TIMEOUT).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_yields_the_warm_result_or_times_out() {
        let (sender, receiver) = mpsc::channel::<anyhow::Result<PreparedDriver>>();
        sender.send(Err(anyhow::anyhow!("boom"))).unwrap();
        let handed = await_prewarmed_driver(receiver).expect("a result arrived");
        assert_eq!(handed.err().map(|error| error.to_string()), Some("boom".to_owned()));

        // A dropped sender resolves at once rather than waiting out the leash.
        let (sender, receiver) = mpsc::channel::<anyhow::Result<PreparedDriver>>();
        drop(sender);
        let started = Instant::now();
        assert!(await_prewarmed_driver(receiver).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
