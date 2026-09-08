//! Warming a session's provider process while the user is still typing.
//!
//! Fork addition. Upstream starts the provider lazily, when the first prompt
//! is submitted, and a Node-based CLI takes two to six seconds to boot — on
//! a first message that boot is most of the wait between Enter and the first
//! request leaving the machine. Typing is a good enough declaration of
//! intent: the first edit to the composer starts the process, and the
//! submission then finds it booted.
//!
//! A warm process is **parked**, never installed in `Waku::runtimes`. That is
//! the whole design. An installed runtime is a live session: the event pump
//! drains it, so its `Connected` event would set the session's provider
//! cursor — which is what `has_started` reads, so the task would appear in
//! the sidebar and the empty state would go while the user was still typing
//! — and any error it reported would raise a toast over the composer. Parked,
//! its events sit in their unbounded channel until a submission installs it,
//! at which point they are drained exactly as an ordinary lazy start's are.
//! Nothing about the session changes before Enter.
//!
//! The process is only handed over when it still fits: same provider, same
//! session options. A model or mode changed between the first keystroke and
//! Enter means the warm process was started for something else, so it is
//! closed and the submission boots its own.

use std::sync::mpsc;

use super::*;

/// How long a parked process nobody prompted is kept.
const UNUSED_PREWARM_TTL: Duration = Duration::from_secs(10 * 60);

/// How long a submission waits for an in-flight warm start before giving up
/// on it and starting a process of its own. Generous: the point of waiting
/// is that the warm process is nearly ready.
const PREWARM_HANDOFF_TIMEOUT: Duration = Duration::from_secs(45);

/// How long a session that failed to warm is left alone. Without this every
/// further keystroke would retry a start that just failed — a request per
/// character at a daemon that is, most likely, exactly what failed.
const PREWARM_FAILURE_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// What the session looked like when a warm start began. A warm process is
/// only reusable while this still describes the session.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PrewarmShape {
    provider: ProviderKind,
    options: SessionOptions,
}

/// A warm start that has not landed yet.
struct PrewarmInFlight {
    shape: PrewarmShape,
    /// Set when a submission arrived meanwhile; the result goes to it rather
    /// than to the parking map.
    waiter: Option<mpsc::Sender<anyhow::Result<PreparedDriver>>>,
}

/// A warm process waiting for a submission to claim it.
struct ParkedPrewarm {
    shape: PrewarmShape,
    driver: PreparedDriver,
    parked_at: Instant,
}

/// What a submission found waiting for it.
pub(super) enum PrewarmClaim {
    /// A booted process, ready to install.
    Ready(PreparedDriver),
    /// A start still running; its result arrives on this channel.
    InFlight(mpsc::Receiver<anyhow::Result<PreparedDriver>>),
}

#[derive(Default)]
pub(super) struct RuntimePrewarms {
    in_flight: HashMap<Uuid, PrewarmInFlight>,
    parked: HashMap<Uuid, ParkedPrewarm>,
    /// Sessions whose warm start failed, and when. Cleared by a successful
    /// claim or by the cooldown.
    failed_at: HashMap<Uuid, Instant>,
}

impl Waku {
    /// Start the selected session's provider if the composer holds a draft
    /// and nothing is running, starting or parked for it yet. Called on every
    /// edit; every early return here is a field read, never I/O.
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
            || self.runtime_prewarms.parked.contains_key(&session_id)
            || self.submission_preparations.contains(&session_id)
            || self.goal_runtime_starts.contains(&session_id)
            || self.response_fork_preparations.contains_key(&session_id)
        {
            return;
        }
        // A start that just failed is not retried per keystroke.
        if self
            .runtime_prewarms
            .failed_at
            .get(&session_id)
            .is_some_and(|failed_at| failed_at.elapsed() < PREWARM_FAILURE_COOLDOWN)
        {
            return;
        }
        // Nothing to start a process through, and a warm start has nothing to
        // report: the submission surfaces the disconnection in context.
        if self.daemon.client().is_disconnected() {
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
        let shape = PrewarmShape {
            provider: session.provider,
            options: self.session_options(session),
        };
        self.runtime_prewarms
            .in_flight
            .insert(session_id, PrewarmInFlight { shape, waiter: None });
        cx.spawn(async move |waku, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move { super::runtime::start_driver(request, cwd) })
                .await;
            let _ = waku.update(cx, |waku, _| {
                waku.finish_runtime_prewarm(session_id, prepared);
            });
        })
        .detach();
    }

    /// Park a landed warm start, or hand it to the submission waiting for it.
    /// Never notifies: a warm process is not a change the user can see.
    fn finish_runtime_prewarm(&mut self, session_id: Uuid, prepared: anyhow::Result<PreparedDriver>) {
        let Some(in_flight) = self.runtime_prewarms.in_flight.remove(&session_id) else {
            // The start was abandoned while it ran.
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
        // runs the same start and surfaces the same error in context. It does
        // put the session on the cooldown, so typing on cannot retry it per
        // character.
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(_) => {
                self.runtime_prewarms
                    .failed_at
                    .insert(session_id, Instant::now());
                return;
            }
        };
        if !self.prewarm_still_fits(session_id, &in_flight.shape) {
            prepared.handle.close();
            return;
        }
        self.runtime_prewarms.parked.insert(
            session_id,
            ParkedPrewarm {
                shape: in_flight.shape,
                driver: prepared,
                parked_at: Instant::now(),
            },
        );
    }

    /// Whether a process warmed for `shape` is still the right one for the
    /// session as it stands now.
    fn prewarm_still_fits(&self, session_id: Uuid, shape: &PrewarmShape) -> bool {
        if self.runtimes.contains_key(&session_id)
            || self.goal_runtime_starts.contains(&session_id)
            || self.submission_preparations.contains(&session_id)
        {
            return false;
        }
        self.state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .is_some_and(|session| {
                matches!(session.status, SessionStatus::Idle | SessionStatus::Failed)
                    && prewarm_shape_matches(
                        shape,
                        session.provider,
                        &self.session_options(session),
                    )
            })
    }

    /// For the submission path: the warm process for this session, if one
    /// fits. A parked process that no longer fits — the model or mode moved
    /// between the first keystroke and Enter — is closed here, so the
    /// submission boots one with the options the user actually chose.
    pub(super) fn take_prewarm_claim(&mut self, session_id: Uuid) -> Option<PrewarmClaim> {
        self.runtime_prewarms.failed_at.remove(&session_id);
        let current = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| (session.provider, self.session_options(session)))?;
        let fits = |shape: &PrewarmShape| prewarm_shape_matches(shape, current.0, &current.1);

        if let Some(parked) = self.runtime_prewarms.parked.remove(&session_id) {
            if fits(&parked.shape) {
                return Some(PrewarmClaim::Ready(parked.driver));
            }
            parked.driver.handle.close();
            return None;
        }
        let in_flight = self.runtime_prewarms.in_flight.get_mut(&session_id)?;
        if !fits(&in_flight.shape) {
            // Let it land and be closed rather than waiting for a process the
            // submission cannot use.
            self.runtime_prewarms.in_flight.remove(&session_id);
            return None;
        }
        let (sender, receiver) = mpsc::channel();
        in_flight.waiter = Some(sender);
        Some(PrewarmClaim::InFlight(receiver))
    }

    /// Close parked processes nobody claimed within the leash, and any whose
    /// session is gone. Runs with the idle sweep.
    pub(super) fn reap_unused_prewarms(&mut self) {
        let sessions: HashSet<Uuid> = self
            .state
            .sessions
            .iter()
            .map(|session| session.id)
            .collect();
        let expired: Vec<Uuid> = self
            .runtime_prewarms
            .parked
            .iter()
            .filter(|(session_id, parked)| {
                parked.parked_at.elapsed() >= UNUSED_PREWARM_TTL
                    || !sessions.contains(session_id)
                    // Something else started a runtime meanwhile; this
                    // one will never be claimed.
                    || self.runtimes.contains_key(session_id)
            })
            .map(|(session_id, _)| *session_id)
            .collect();
        for session_id in expired {
            if let Some(parked) = self.runtime_prewarms.parked.remove(&session_id) {
                parked.driver.handle.close();
            }
        }
        self.runtime_prewarms
            .failed_at
            .retain(|_, failed_at| failed_at.elapsed() < PREWARM_FAILURE_COOLDOWN);
    }
}

/// Whether a warm start begun for `shape` matches a session now running
/// `provider` with `options`.
fn prewarm_shape_matches(
    shape: &PrewarmShape,
    provider: ProviderKind,
    options: &SessionOptions,
) -> bool {
    shape.provider == provider && &shape.options == options
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

    fn options(model: &str) -> SessionOptions {
        SessionOptions {
            mode: RuntimeMode::default(),
            interaction_mode: InteractionMode::default(),
            model: Some(model.to_owned()),
            reasoning_effort: None,
            service_tier: None,
            context_window: None,
        }
    }

    fn shape(provider: ProviderKind, model: &str) -> PrewarmShape {
        PrewarmShape {
            provider,
            options: options(model),
        }
    }

    #[test]
    fn a_warm_process_fits_the_session_it_was_started_for() {
        let started = shape(ProviderKind::Claude, "claude-fable-5-1");
        assert!(prewarm_shape_matches(
            &started,
            ProviderKind::Claude,
            &options("claude-fable-5-1")
        ));
    }

    #[test]
    fn a_model_change_before_enter_rejects_the_warm_process() {
        let started = shape(ProviderKind::Claude, "claude-fable-5-1");
        assert!(!prewarm_shape_matches(
            &started,
            ProviderKind::Claude,
            &options("claude-opus-5")
        ));
    }

    #[test]
    fn a_provider_change_before_enter_rejects_the_warm_process() {
        let started = shape(ProviderKind::Claude, "claude-fable-5-1");
        assert!(!prewarm_shape_matches(
            &started,
            ProviderKind::Codex,
            &options("claude-fable-5-1")
        ));
    }

    #[test]
    fn a_mode_change_before_enter_rejects_the_warm_process() {
        let started = shape(ProviderKind::Codex, "gpt-5.6-sol");
        let mut switched = options("gpt-5.6-sol");
        switched.interaction_mode = InteractionMode::Plan;
        assert!(!prewarm_shape_matches(
            &started,
            ProviderKind::Codex,
            &switched
        ));
    }

    /// The regression this module was rewritten for: a warm process that is
    /// installed in the runtime map is a live session — the event pump drains
    /// it, its `Connected` event sets the provider cursor that `has_started`
    /// reads, and its errors raise toasts. Parking is the whole point, so the
    /// landing path must never install and must never notify.
    #[test]
    fn a_landed_warm_start_is_parked_rather_than_installed() {
        let source = include_str!("runtime_prewarm.rs");
        // Anchored on the definition's indentation so this test does not match
        // its own string literals.
        let start = source
            .find("
    fn finish_runtime_prewarm(")
            .expect("landing fn");
        let body = &source[start + 1..];
        let end = body.find("
    /// Whether a process warmed").unwrap_or(body.len());
        let body = &body[..end];

        for forbidden in ["install_prepared_driver", "runtimes.insert", "cx.notify"] {
            assert!(
                !body.contains(forbidden),
                "a warm start must stay out of the UI, found `{forbidden}`"
            );
        }
        assert!(body.contains("parked.insert"), "it has to park somewhere");
    }

    #[test]
    fn handoff_yields_the_warm_result_or_times_out() {
        let (sender, receiver) = mpsc::channel::<anyhow::Result<PreparedDriver>>();
        sender.send(Err(anyhow::anyhow!("boom"))).unwrap();
        let handed = await_prewarmed_driver(receiver).expect("a result arrived");
        assert_eq!(
            handed.err().map(|error| error.to_string()),
            Some("boom".to_owned())
        );

        // A dropped sender resolves at once rather than waiting out the leash.
        let (sender, receiver) = mpsc::channel::<anyhow::Result<PreparedDriver>>();
        drop(sender);
        let started = Instant::now();
        assert!(await_prewarmed_driver(receiver).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
