//! The built-in agent, running in this process.
//!
//! Every other transport in this directory negotiates with a CLI Waku
//! launched. This one calls a library. `waku-agent-bridge` owns the engine's
//! lifecycle and reports `AgentEvent`s in the engine's own vocabulary; this
//! file does the half that needs `waku-core` — turning those into
//! [`DriverEvent`]s and [`ActivityItem`]s through the same normalizer every
//! other provider uses, so the transcript cannot tell the difference.
//!
//! # Why the conversation lives here and not in the agent
//!
//! The engine takes `&mut Vec<Message>`, so Waku owns the transcript. Rewind
//! and branch are therefore truncations rather than protocol calls, and resume
//! is a file read. What the engine keeps is only the working copy for the turn
//! it is running.
//!
//! The serialized conversation is written next to the daemon's state database
//! rather than into it: a transcript is large, opaque to every query the state
//! store answers, and rewritten wholesale once per turn.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use parking_lot::Mutex;
use serde_json::Value;
use uuid::Uuid;
use waku_agent_bridge::{AccessMode, AgentEvent, AgentSession, AgentStartOptions, TurnOptions};

use super::activity;
use crate::driver::{
    DriverControl, DriverEventSender, DriverEventSink, DriverStartOptions, SessionOptions,
};
use crate::model::{
    ActivityKind, DriverEvent, InteractionMode, PermissionOption, ProviderResumeCursor, RuntimeMode,
};

pub struct NativeDriver {
    session: AgentSession,
    /// Where this conversation is persisted, so a resume finds it again.
    store: SessionStore,
    /// This provider's resume cursor. The engine has no session identity of
    /// its own, so the driver mints one and reports it through `Connected` —
    /// the same shape every other provider's thread id takes.
    session_id: Uuid,
}

impl NativeDriver {
    pub fn start(
        options: DriverStartOptions,
        events: DriverEventSender,
    ) -> anyhow::Result<Self> {
        // A cursor from another provider means the session changed provider.
        // That transcript is in the other provider's format and names tools
        // this engine does not have, so it starts a fresh conversation rather
        // than a mistranslated one.
        let resumed = match &options.provider_cursor {
            Some(ProviderResumeCursor::Native { session_id }) => Uuid::parse_str(session_id).ok(),
            _ => None,
        };
        let session_id = resumed.unwrap_or_else(Uuid::new_v4);
        let store = SessionStore::new(session_id);
        let history = if resumed.is_some() {
            store.load()
        } else {
            Vec::new()
        };

        let start = AgentStartOptions {
            cwd: options.cwd.clone(),
            access_mode: access_mode(options.mode),
            plan_mode: options.interaction_mode == InteractionMode::Plan,
            model: options.model.clone(),
            reasoning_effort: options.reasoning_effort.clone(),
            history,
        };

        let sink = EventTranslator::new(events.clone(), store.clone());
        let session = AgentSession::start(start, sink.into_sink())?;

        // Report the cursor immediately rather than after the first turn: the
        // transcript file is named by it, so a session closed before it ever
        // answered still reopens onto its own history.
        let _ = events.send(DriverEvent::Connected {
            provider_cursor: Some(ProviderResumeCursor::Native {
                session_id: session_id.to_string(),
            }),
        });

        Ok(Self {
            session,
            store,
            session_id,
        })
    }
}

impl DriverControl for NativeDriver {
    fn prompt(&self, prompt: String) {
        self.session.prompt(prompt);
    }

    /// The engine drains its command queue at every turn boundary, so a
    /// message sent while tools are running joins the conversation before the
    /// next request — no restart, no lost turn.
    fn supports_steer(&self) -> bool {
        true
    }

    fn steer(&self, prompt: String) {
        self.session.steer(prompt);
    }

    fn cancel(&self) {
        self.session.cancel();
    }

    fn respond(&self, request_id: String, option_id: String) {
        self.session.respond(&request_id, &option_id);
    }

    /// Always absorbed. Model, effort and access mode are read from a fresh
    /// `QueryConfig` at the start of every turn, so none of them can require a
    /// restart the way a launch argument would.
    fn apply_options(&self, options: SessionOptions) -> bool {
        self.session.apply_options(TurnOptions {
            access_mode: Some(access_mode(options.mode)),
            plan_mode: Some(options.interaction_mode == InteractionMode::Plan),
            model: options.model,
            reasoning_effort: options.reasoning_effort,
        })
    }

    fn rollback(&self, turns: usize) -> anyhow::Result<Option<ProviderResumeCursor>> {
        let removed = self.session.rollback(turns)?;
        if removed == 0 {
            return Ok(None);
        }
        self.store.save(&self.session.history_snapshot());
        Ok(Some(ProviderResumeCursor::Native {
            session_id: self.session_id.to_string(),
        }))
    }

    fn fork(&self, turns_to_remove: usize) -> anyhow::Result<ProviderResumeCursor> {
        let branched = self.session.fork(turns_to_remove)?;
        // The branch is a new session with its own transcript file. Waku
        // creates the session and starts a driver against this cursor; the
        // history has to be on disk before that happens.
        let branch_id = Uuid::new_v4();
        SessionStore::new(branch_id).save(&branched);
        Ok(ProviderResumeCursor::Native {
            session_id: branch_id.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

fn access_mode(mode: RuntimeMode) -> AccessMode {
    match mode {
        RuntimeMode::Ask => AccessMode::Ask,
        RuntimeMode::AutoAcceptEdits => AccessMode::AutoAcceptEdits,
        RuntimeMode::Auto => AccessMode::Auto,
        RuntimeMode::FullAccess => AccessMode::FullAccess,
        // The legacy combined mode; state migration moves it into
        // `interaction_mode`, and a session that still carries it should be
        // treated as the most cautious access level rather than the loosest.
        RuntimeMode::Plan => AccessMode::Ask,
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    fn new(session_id: Uuid) -> Self {
        Self {
            path: Self::directory().join(format!("{session_id}.json")),
        }
    }

    fn directory() -> PathBuf {
        crate::persistence::StateStore::default_path().with_file_name("agent-sessions")
    }

    fn load(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap_or_default()
    }

    /// Write the transcript. Atomic, because a crash between `TurnFinished`
    /// and the next prompt must not leave half a conversation on disk.
    fn save(&self, bytes: &[u8]) {
        if let Err(error) = self.write(bytes) {
            report_warning(&format!(
                "could not save the built-in agent's transcript to {}: {error:#}",
                self.path.display()
            ));
        }
    }

    fn write(&self, bytes: &[u8]) -> anyhow::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow!("transcript path has no directory"))?;
        std::fs::create_dir_all(parent).with_context(|| {
            format!("could not create {}", parent.display())
        })?;
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, &self.path)?;
        restrict_permissions(&self.path);
        Ok(())
    }
}

/// A transcript holds whatever the agent read into context. Keep it
/// owner-only, the way the engine keeps its own session database.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// `waku-core` links no logging framework, and the daemon's stderr is what
/// the desktop already surfaces for provider failures. This keeps that one
/// channel rather than adding a dependency for a single line.
fn report_warning(message: &str) {
    eprintln!("warning: {message}");
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Turns the engine's events into the transcript's.
struct EventTranslator {
    events: DriverEventSender,
    store: SessionStore,
    tools: Mutex<std::collections::HashMap<String, ToolCall>>,
}

/// What a tool call looked like when it started, so its completion can be
/// rendered as the same row rather than a second one.
struct ToolCall {
    kind: ActivityKind,
    title: String,
    input: Value,
}

impl EventTranslator {
    fn new(events: DriverEventSender, store: SessionStore) -> Self {
        Self {
            events,
            store,
            tools: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn into_sink(self) -> waku_agent_bridge::EventSink {
        let this = Arc::new(self);
        waku_agent_bridge::EventSink::new(move |event| this.handle(event))
    }

    fn handle(&self, event: AgentEvent) {
        match event {
            AgentEvent::TurnStarted => {
                self.tools.lock().clear();
                self.send(DriverEvent::TurnStarted);
            }
            AgentEvent::Text(text) => self.send(DriverEvent::TextDelta(text)),
            AgentEvent::Reasoning(text) => self.send(DriverEvent::ReasoningDelta(text)),
            AgentEvent::ToolStarted { id, name, input } => {
                let kind = ActivityKind::from_tool_name(&name);
                let title = tool_title(&name, &input);
                self.send(DriverEvent::RichActivity(activity::tool_activity(
                    Some(id.clone()),
                    kind,
                    title.clone(),
                    Some(&input),
                    None,
                    None,
                    false,
                    false,
                )));
                self.tools
                    .lock()
                    .insert(id, ToolCall { kind, title, input });
            }
            AgentEvent::ToolFinished {
                id,
                name,
                output,
                failed,
            } => {
                // A completion whose start was never seen still deserves a
                // row; falling back on the tool name keeps it readable.
                let call = self.tools.lock().remove(&id).unwrap_or_else(|| ToolCall {
                    kind: ActivityKind::from_tool_name(&name),
                    title: name.clone(),
                    input: Value::Null,
                });
                self.send(DriverEvent::RichActivity(activity::tool_activity(
                    Some(id),
                    call.kind,
                    call.title,
                    Some(&call.input),
                    Some(&output),
                    None,
                    failed,
                    true,
                )));
            }
            AgentEvent::Usage {
                context_tokens,
                context_window,
            } => self.send(DriverEvent::UsageUpdated {
                context_tokens,
                context_window,
            }),
            AgentEvent::Permission {
                request_id,
                tool_name: _,
                title,
                detail,
                options,
            } => {
                let options = options
                    .into_iter()
                    .map(|choice| PermissionOption {
                        id: choice.id().to_string(),
                        label: permission_label(choice),
                        allow: choice.is_allow(),
                    })
                    .collect();
                self.send(DriverEvent::Permission {
                    request_id,
                    title,
                    detail,
                    options,
                });
            }
            AgentEvent::SteerAccepted { message } => {
                self.send(DriverEvent::SteerAccepted { message })
            }
            AgentEvent::SteerRejected { message, reason } => {
                self.send(DriverEvent::SteerRejected { message, reason })
            }
            AgentEvent::HistoryCommitted(bytes) => self.store.save(&bytes),
            AgentEvent::Error(message) => self.send(DriverEvent::Error(message)),
            AgentEvent::TurnFinished { success, summary } => {
                self.tools.lock().clear();
                self.send(DriverEvent::TurnFinished { success, summary });
            }
        }
    }

    fn send(&self, event: DriverEvent) {
        let _ = DriverEventSink::send(&self.events, event);
    }
}

/// The row's headline.
///
/// Prefers a `title` the tool supplied, then the compact subject the activity
/// normalizer would show anyway, and falls back to a de-camel-cased tool name —
/// the same precedence every other provider's rows follow.
fn tool_title(name: &str, input: &Value) -> String {
    if let Some(title) = activity::input_title(Some(input)) {
        return title;
    }
    for key in ["command", "query", "pattern", "file_path", "path", "url"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_owned();
            }
        }
    }
    name.to_owned()
}

/// The wording other providers already use, so the dialog reads the same
/// whichever agent raised it. The durable options say "always" because they
/// write a rule the Permissions settings page lists — this is the one place a
/// user can create one without going looking for it.
fn permission_label(choice: waku_agent_bridge::PermissionChoice) -> String {
    use waku_agent_bridge::PermissionChoice as Choice;
    match choice {
        Choice::AllowOnce => tr!("permission.allow_once"),
        Choice::AllowAlways => tr!("permission.always_allow"),
        Choice::RejectOnce => tr!("common.deny"),
        Choice::RejectAlways => tr!("permission.always_deny"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_plan_runtime_mode_degrades_to_asking() {
        assert_eq!(access_mode(RuntimeMode::Plan), AccessMode::Ask);
        assert_eq!(access_mode(RuntimeMode::FullAccess), AccessMode::FullAccess);
    }

    #[test]
    fn a_tool_supplied_title_wins_over_the_argument_scan() {
        let input = serde_json::json!({ "title": "Inspect the app", "command": "ls -la" });
        assert_eq!(tool_title("Bash", &input), "Inspect the app");
    }

    #[test]
    fn a_command_reads_better_than_the_tool_name() {
        let input = serde_json::json!({ "command": "cargo test" });
        assert_eq!(tool_title("Bash", &input), "cargo test");
    }

    #[test]
    fn a_tool_with_nothing_to_show_falls_back_to_its_name() {
        assert_eq!(tool_title("TodoWrite", &Value::Null), "TodoWrite");
        assert_eq!(
            tool_title("Bash", &serde_json::json!({ "command": "   " })),
            "Bash"
        );
    }

    #[test]
    fn transcripts_are_kept_out_of_the_state_database() {
        let store = SessionStore::new(Uuid::nil());
        assert_eq!(
            store.path.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("agent-sessions"))
        );
    }
}
