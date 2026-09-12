//! Routing tool approvals to the GUI and back.
//!
//! The engine's [`PermissionHandler`] is synchronous: it is called from inside
//! a running tool and must return a decision. The GUI's answer arrives much
//! later, on another thread. So `request_permission` publishes a
//! [`AgentEvent::Permission`] and then blocks until [`PermissionBridge::resolve`]
//! is called with the user's choice.
//!
//! Blocking a Tokio worker is normally a defect. Two things make it safe here:
//! the block happens inside [`tokio::task::block_in_place`], which hands the
//! worker's other tasks to another thread first; and every waiter is released
//! by [`PermissionBridge::release_all`] when the turn is cancelled, so a
//! pending dialog can never outlive the turn that raised it.
//!
//! What is *not* reimplemented here is the decision itself. The engine's
//! `PermissionManager` already resolves mode, persistent and session rules,
//! read/write levels and workspace boundaries, and it is the same store the
//! Permissions settings page reads. This bridge asks it first and only shows a
//! dialog for what it reports as genuinely undecided — which is also what makes
//! "always allow" persist instead of evaporating with the session.

use std::sync::Arc;
use std::time::Duration;

use claurst_core::config::Settings;
use claurst_core::{
    PermissionAction, PermissionDecision, PermissionHandler, PermissionManager, PermissionRequest,
    PermissionRule, PermissionScope,
};
use crossbeam_channel::{Receiver, Sender, bounded};
use parking_lot::Mutex;
use std::collections::HashMap;

use crate::events::{AgentEvent, EventSink, PermissionChoice};

/// Upper bound on how long a tool waits for an answer.
///
/// Not a UX deadline — the dialog has none, and the user may take as long as
/// they like. It exists so a lost answer (a closed window, a dropped daemon
/// connection) fails the tool instead of parking a worker for the life of the
/// process.
const ANSWER_DEADLINE: Duration = Duration::from_secs(60 * 60);

/// Shared state between the engine's handler and the session that answers it.
pub struct PermissionBridge {
    events: EventSink,
    pending: Mutex<HashMap<String, Sender<PermissionChoice>>>,
    /// Standing answer for anything the manager leaves undecided. `None` means
    /// the user is asked.
    auto: Mutex<Option<PermissionChoice>>,
    manager: Arc<std::sync::Mutex<PermissionManager>>,
    settings: Arc<Mutex<Settings>>,
}

impl PermissionBridge {
    pub fn new(
        events: EventSink,
        manager: Arc<std::sync::Mutex<PermissionManager>>,
        settings: Arc<Mutex<Settings>>,
        auto: Option<PermissionChoice>,
    ) -> Arc<Self> {
        Arc::new(Self {
            events,
            pending: Mutex::new(HashMap::new()),
            auto: Mutex::new(auto),
            manager,
            settings,
        })
    }

    /// Change the standing answer without restarting the session. Used when
    /// the access mode changes between turns.
    pub fn set_auto(&self, auto: Option<PermissionChoice>) {
        *self.auto.lock() = auto;
    }

    /// Deliver the user's answer. Unknown ids are ignored: a late answer to a
    /// request the cancel path already released is not an error.
    pub fn resolve(&self, request_id: &str, choice: PermissionChoice) {
        let sender = self.pending.lock().remove(request_id);
        if let Some(sender) = sender {
            let _ = sender.send(choice);
        }
    }

    /// Release every waiter with a rejection. Called on cancel and on
    /// shutdown, so no tool is left blocked on a dialog that is gone.
    pub fn release_all(&self) {
        let pending: Vec<_> = self.pending.lock().drain().map(|(_, tx)| tx).collect();
        for sender in pending {
            let _ = sender.send(PermissionChoice::RejectOnce);
        }
    }

    /// Ask the user, or apply the standing answer.
    fn ask(&self, request: &PermissionRequest, reason: &str) -> PermissionChoice {
        if let Some(auto) = *self.auto.lock() {
            return auto;
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx): (Sender<PermissionChoice>, Receiver<PermissionChoice>) = bounded(1);
        self.pending.lock().insert(request_id.clone(), tx);

        self.events.emit(AgentEvent::Permission {
            request_id: request_id.clone(),
            tool_name: request.tool_name.clone(),
            title: title_for(request),
            detail: detail_for(request, reason),
            options: vec![
                PermissionChoice::AllowOnce,
                PermissionChoice::AllowAlways,
                PermissionChoice::RejectOnce,
                PermissionChoice::RejectAlways,
            ],
        });

        let answer = wait(&rx);
        // Answered, timed out, or released — the entry is done either way.
        self.pending.lock().remove(&request_id);
        answer
    }

    /// Record a durable decision so the same tool is not asked about again,
    /// and so it shows up in the Permissions settings page.
    ///
    /// A failure to persist is logged and swallowed: the user's answer still
    /// applies to this call. Turning a read-only settings file into a denied
    /// tool would be the worse failure.
    fn remember(&self, tool_name: &str, action: PermissionAction) {
        let mut settings = self.settings.lock();
        let Ok(mut manager) = self.manager.lock() else {
            return;
        };
        // The engine reports through its own `ClaudeError`, `save_sync`
        // through `anyhow`; both arms land on `anyhow` so one `if let`
        // reports either.
        let result: anyhow::Result<()> = match action {
            PermissionAction::Allow => manager
                .add_persistent_allow(tool_name, &mut settings)
                .map_err(anyhow::Error::from),
            // There is no `add_persistent_deny` upstream, so this does what
            // `add_persistent_allow` does: register the rule with the live
            // manager and append the same rule to the settings file the
            // Permissions page reads back.
            PermissionAction::Deny => {
                let rule = PermissionRule {
                    tool_name: Some(tool_name.to_string()),
                    path_pattern: None,
                    action: PermissionAction::Deny,
                    scope: PermissionScope::Persistent,
                };
                settings
                    .permission_rules
                    .push(claurst_core::SerializedPermissionRule::from(&rule));
                manager.add_rule(rule);
                settings.save_sync()
            }
        };
        if let Err(error) = result {
            tracing::warn!(%error, tool = tool_name, "agent: could not persist a permission rule");
        }
    }
}

/// Wait for the answer without stalling the worker's other tasks.
fn wait(rx: &Receiver<PermissionChoice>) -> PermissionChoice {
    let recv = || {
        rx.recv_timeout(ANSWER_DEADLINE)
            .unwrap_or(PermissionChoice::RejectOnce)
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        // Inside the runtime: hand this worker's remaining tasks to another
        // thread before blocking. Requires the multi-threaded runtime that
        // `crate::runtime` builds.
        tokio::task::block_in_place(recv)
    } else {
        // A tool that reached its permission check from a plain thread blocks
        // only that thread.
        recv()
    }
}

fn title_for(request: &PermissionRequest) -> String {
    if request.description.is_empty() {
        request.tool_name.clone()
    } else {
        request.description.clone()
    }
}

/// The line the user actually decides on.
///
/// The engine's context description is the specific one — "bash: execute
/// `rm -rf build`" rather than a sentence synthesized from the tool kind — so
/// it wins. The manager's reason is the next best, and the bare description is
/// the last resort.
fn detail_for(request: &PermissionRequest, reason: &str) -> String {
    request
        .context_description
        .clone()
        .or_else(|| request.details.clone())
        .unwrap_or_else(|| {
            if reason.is_empty() {
                request.description.clone()
            } else {
                reason.to_string()
            }
        })
}

/// The handler installed on the engine's `ToolContext`.
pub struct GuiPermissionHandler {
    bridge: Arc<PermissionBridge>,
}

impl GuiPermissionHandler {
    pub fn new(bridge: Arc<PermissionBridge>) -> Self {
        Self { bridge }
    }

    fn decide(&self, request: &PermissionRequest) -> PermissionDecision {
        let evaluated = {
            let Ok(manager) = self.bridge.manager.lock() else {
                // A poisoned manager means another thread panicked while
                // holding it. Denying is the only safe reading.
                return PermissionDecision::Deny;
            };
            manager.evaluate(
                &request.tool_name,
                &request.description,
                request.path.as_deref(),
                request.working_dir.as_deref(),
                &request.allowed_roots,
            )
        };

        let reason = match evaluated {
            // Settled by mode or by an existing rule; no dialog.
            PermissionDecision::Allow | PermissionDecision::AllowPermanently => {
                return PermissionDecision::Allow;
            }
            PermissionDecision::Deny | PermissionDecision::DenyPermanently => {
                return PermissionDecision::Deny;
            }
            PermissionDecision::Ask { reason } => reason,
        };

        match self.bridge.ask(request, &reason) {
            PermissionChoice::AllowOnce => PermissionDecision::Allow,
            PermissionChoice::AllowAlways => {
                self.bridge
                    .remember(&request.tool_name, PermissionAction::Allow);
                PermissionDecision::Allow
            }
            PermissionChoice::RejectOnce => PermissionDecision::Deny,
            PermissionChoice::RejectAlways => {
                self.bridge
                    .remember(&request.tool_name, PermissionAction::Deny);
                PermissionDecision::Deny
            }
        }
    }
}

impl PermissionHandler for GuiPermissionHandler {
    /// Never returns `Ask`. `ToolContext::request_permission_inner` treats
    /// `Ask` as "fall back to the pending-permission queue", which this
    /// session does not run — the dialog is raised from `request_permission`
    /// instead, so every answer here is final.
    fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.decide(request)
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.decide(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::PermissionMode;

    fn bridge_with(auto: Option<PermissionChoice>, mode: PermissionMode) -> Arc<PermissionBridge> {
        let settings = Settings::default();
        let manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
            mode, &settings,
        )));
        PermissionBridge::new(
            EventSink::new(|_: AgentEvent| {}),
            manager,
            Arc::new(Mutex::new(settings)),
            auto,
        )
    }

    fn request() -> PermissionRequest {
        PermissionRequest {
            tool_name: "Bash".into(),
            description: "run a command".into(),
            details: None,
            is_read_only: false,
            path: None,
            working_dir: None,
            allowed_roots: Vec::new(),
            context_description: Some("bash: execute `ls`".into()),
        }
    }

    #[test]
    fn full_access_never_reaches_the_dialog() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let settings = Settings::default();
        let manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
            PermissionMode::BypassPermissions,
            &settings,
        )));
        let sink = {
            let seen = seen.clone();
            EventSink::new(move |event: AgentEvent| seen.lock().push(event))
        };
        let bridge = PermissionBridge::new(sink, manager, Arc::new(Mutex::new(settings)), None);
        let handler = GuiPermissionHandler::new(bridge);

        assert!(matches!(
            handler.request_permission(&request()),
            PermissionDecision::Allow
        ));
        assert!(
            seen.lock().is_empty(),
            "bypass mode must not raise a dialog"
        );
    }

    #[test]
    fn an_auto_answer_short_circuits_an_undecided_request() {
        let bridge = bridge_with(Some(PermissionChoice::AllowAlways), PermissionMode::Default);
        assert_eq!(
            bridge.ask(&request(), "why"),
            PermissionChoice::AllowAlways
        );
    }

    #[test]
    fn releasing_waiters_rejects_them() {
        let bridge = bridge_with(None, PermissionMode::Default);
        let (tx, rx) = bounded(1);
        bridge.pending.lock().insert("r1".into(), tx);

        bridge.release_all();

        assert_eq!(rx.recv().unwrap(), PermissionChoice::RejectOnce);
        assert!(bridge.pending.lock().is_empty());
    }

    #[test]
    fn the_detail_prefers_the_engines_context_description() {
        assert_eq!(
            detail_for(&request(), "a generic reason"),
            "bash: execute `ls`"
        );
    }

    #[test]
    fn the_detail_falls_back_to_the_managers_reason() {
        let mut req = request();
        req.context_description = None;
        assert_eq!(detail_for(&req, "Bash wants to run a command"), "Bash wants to run a command");
    }
}
