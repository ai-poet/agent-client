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
use waku_agent_bridge::{
    AccessMode, AgentEvent, AgentSession, AgentStartOptions, BackgroundEntry, BackgroundKind,
    BackgroundStatus, TurnOptions, WireFormat, split_model,
};

use super::activity;
use crate::driver::{
    DriverControl, DriverEventSender, DriverEventSink, DriverStartOptions, SessionOptions,
};
use crate::model::{
    ActivityKind, AgentTurn, BackgroundWorkEvent, BackgroundWorkItem, BackgroundWorkKey,
    BackgroundWorkKind, BackgroundWorkStatus, DriverEvent, InteractionMode, Message, MessageRole,
    PermissionOption, ProviderResumeCursor, ProviderSessionHistory, RuntimeMode, TurnStatus,
    UserInputAnswer, UserInputOption, UserInputQuestion, unix_time_millis,
};

pub struct NativeDriver {
    session: AgentSession,
    events: DriverEventSender,
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

        let (platform, model) = route_of(options.model.as_deref());
        let start = AgentStartOptions {
            cwd: options.cwd.clone(),
            access_mode: access_mode(options.mode),
            plan_mode: options.interaction_mode == InteractionMode::Plan,
            model,
            platform,
            wire_format: wire_format_of(options.service_tier.as_deref()),
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
            events,
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

    /// One question per request, so the first answer is the whole answer.
    /// The engine's `AskUserQuestion` takes a single string; a multi-select
    /// is joined the way a person would type it.
    fn respond_user_input(&self, request_id: String, answers: Vec<UserInputAnswer>) {
        let text = answers
            .into_iter()
            .find(|answer| answer.question_id == request_id)
            .map(|answer| answer.answers.join(", "))
            .unwrap_or_default();
        self.session.answer(&request_id, text);
    }

    fn refresh_background_work(&self) {
        self.session.refresh_background_work();
    }

    fn stop_background_work(&self, key: BackgroundWorkKey, control_id: String) {
        if let Err(message) = self.session.stop_background_work(&control_id) {
            let _ = self.events.send(DriverEvent::BackgroundWork(
                BackgroundWorkEvent::StopFailed { key, message },
            ));
        }
    }

    /// Always absorbed. Model, effort and access mode are read from a fresh
    /// `QueryConfig` at the start of every turn, so none of them can require a
    /// restart the way a launch argument would.
    fn apply_options(&self, options: SessionOptions) -> bool {
        let (platform, model) = route_of(options.model.as_deref());
        self.session.apply_options(TurnOptions {
            access_mode: Some(access_mode(options.mode)),
            plan_mode: Some(options.interaction_mode == InteractionMode::Plan),
            model,
            platform: Some(platform),
            wire_format: Some(wire_format_of(options.service_tier.as_deref())),
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

/// The picker's model id is `<platform>::<model>`; a bare id is Anthropic.
fn route_of(model: Option<&str>) -> (Option<String>, Option<String>) {
    match model {
        Some(id) => {
            let (platform, model) = split_model(id);
            (platform, Some(model))
        }
        None => (None, None),
    }
}

/// The picker's "service tier" slot carries the wire format for the built-in
/// agent. An unknown or absent tier means the platform's native format.
fn wire_format_of(tier: Option<&str>) -> Option<WireFormat> {
    tier.and_then(WireFormat::from_id)
}

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

/// Read a stored transcript back as Waku turns and messages, for
/// `LoadProviderSession`.
///
/// Every turn shell is kept so provider turn numbering stays exact; only the
/// text of the last `turn_limit` turns is imported, the same bound the CLI
/// importers apply. Tool calls, results and thinking are not text a reader
/// would see, so they are left out — the engine's own transcript file
/// remains the full record.
pub(crate) fn provider_session_history(
    transcript_id: &str,
    turn_limit: usize,
) -> anyhow::Result<ProviderSessionHistory> {
    let id = Uuid::parse_str(transcript_id)
        .with_context(|| format!("`{transcript_id}` is not a transcript id"))?;
    let store = SessionStore::new(id);
    let bytes = store.load();
    if bytes.is_empty() {
        anyhow::bail!(
            "the built-in agent has no transcript for {transcript_id} at {}",
            store.path.display()
        );
    }
    let stored = waku_agent_bridge::history::deserialize(&bytes);
    let display = waku_agent_bridge::history::turns_for_display(&stored);
    let first_visible = display.len().saturating_sub(turn_limit);
    let now = unix_time_millis() / 1000;

    let mut history = ProviderSessionHistory::default();
    for (index, turn) in display.into_iter().enumerate() {
        let turn_id = Uuid::new_v4();
        history.turns.push(AgentTurn {
            id: turn_id,
            turn_count: index + 1,
            status: TurnStatus::Completed,
            provider_turn_started: true,
            provider_resume_at: None,
            started_at: now,
            completed_at: Some(now),
            checkpoint: None,
        });
        if index < first_visible {
            continue;
        }
        history
            .messages
            .push(Message::new_for_turn(MessageRole::User, turn.user, turn_id));
        if !turn.assistant.is_empty() {
            history.messages.push(Message::new_for_turn(
                MessageRole::Assistant,
                turn.assistant,
                turn_id,
            ));
        }
    }
    Ok(history)
}

/// Delete the transcript file for a session Waku has removed. A file that is
/// already gone is not an error; an id that is not a UUID is refused rather
/// than turned into a path.
pub(crate) fn delete_agent_transcript(transcript_id: &str) -> anyhow::Result<()> {
    let id = Uuid::parse_str(transcript_id)
        .with_context(|| format!("`{transcript_id}` is not a transcript id"))?;
    let store = SessionStore::new(id);
    match std::fs::remove_file(&store.path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("could not delete {}", store.path.display())),
    }
}

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
                let kind = activity_kind(&name);
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
                    kind: activity_kind(&name),
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
            AgentEvent::UserInput {
                request_id,
                question,
                options,
            } => {
                let options = options
                    .into_iter()
                    .map(|label| UserInputOption {
                        label,
                        description: None,
                    })
                    .collect();
                self.send(DriverEvent::UserInputRequested {
                    request_id: request_id.clone(),
                    questions: vec![UserInputQuestion {
                        id: request_id,
                        header: String::new(),
                        question,
                        options,
                        multi_select: false,
                    }],
                });
            }
            AgentEvent::BackgroundWork(entries) => {
                let items = entries.into_iter().map(background_item).collect();
                self.send(DriverEvent::BackgroundWork(
                    BackgroundWorkEvent::ReconcileLive { items },
                ));
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

/// The engine's tool names that `ActivityKind::from_tool_name` does not
/// recognise, checked first. Everything else — Read, Edit, Bash, Grep, Glob,
/// TodoWrite, WebFetch — is already covered by the generic classifier.
fn activity_kind(name: &str) -> ActivityKind {
    match name {
        "PowerShell" | "REPL" => ActivityKind::Command,
        "BatchEdit" => ActivityKind::FileChange,
        "EnterPlanMode" | "ExitPlanMode" => ActivityKind::Plan,
        "ToolSearch" => ActivityKind::Search,
        _ => ActivityKind::from_tool_name(name),
    }
}

/// One background entry as the panel files it. The engine's task id is both
/// the provider id and the control id: it is what `stop` takes back.
fn background_item(entry: BackgroundEntry) -> BackgroundWorkItem {
    let kind = match entry.kind {
        BackgroundKind::Process => BackgroundWorkKind::Process,
        BackgroundKind::Subagent => BackgroundWorkKind::Subagent,
    };
    let status = match entry.status {
        BackgroundStatus::Running => BackgroundWorkStatus::Running,
        BackgroundStatus::Completed => BackgroundWorkStatus::Completed,
        BackgroundStatus::Failed => BackgroundWorkStatus::Failed,
        BackgroundStatus::Stopped => BackgroundWorkStatus::Stopped,
    };
    let mut item = BackgroundWorkItem::new(kind, entry.id.clone(), entry.title.clone(), status);
    if kind == BackgroundWorkKind::Process {
        item.command = Some(entry.title);
    }
    item.detail = entry.detail.or_else(|| entry.pid.map(|pid| format!("PID {pid}")));
    item.output = entry.output;
    item.started_at_ms = entry.started_at_ms;
    item.updated_at_ms = entry.finished_at_ms.unwrap_or_else(unix_time_millis);
    item.duration_ms = entry
        .finished_at_ms
        .map(|finished| finished.saturating_sub(entry.started_at_ms));
    item.background = true;
    item.can_stop = status.is_stoppable();
    item.control_id = Some(entry.id);
    item
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
    fn the_picker_id_carries_the_platform_and_the_tier_carries_the_format() {
        assert_eq!(
            route_of(Some("openai::gpt-5.6-sol")),
            (Some("openai".into()), Some("gpt-5.6-sol".into()))
        );
        assert_eq!(route_of(Some("claude-sonnet-5")), (None, Some("claude-sonnet-5".into())));
        assert_eq!(wire_format_of(Some("responses")), Some(WireFormat::Responses));
        assert_eq!(wire_format_of(Some("default")), None);
        assert_eq!(wire_format_of(None), None);
    }

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
    fn the_engines_own_tool_names_classify_before_the_generic_table() {
        assert_eq!(activity_kind("PowerShell"), ActivityKind::Command);
        assert_eq!(activity_kind("REPL"), ActivityKind::Command);
        assert_eq!(activity_kind("BatchEdit"), ActivityKind::FileChange);
        assert_eq!(activity_kind("ExitPlanMode"), ActivityKind::Plan);
        // Still the generic classifier's answer for names it knows.
        assert_eq!(activity_kind("Read"), ActivityKind::FileRead);
        assert_eq!(activity_kind("TodoWrite"), ActivityKind::Plan);
    }

    #[test]
    fn a_running_background_shell_is_stoppable_by_its_task_id() {
        let item = background_item(BackgroundEntry {
            id: "task-1".into(),
            kind: BackgroundKind::Process,
            title: "cargo test".into(),
            status: BackgroundStatus::Running,
            detail: None,
            pid: Some(4242),
            output: None,
            started_at_ms: 1_000,
            finished_at_ms: None,
        });
        assert_eq!(item.key.kind, BackgroundWorkKind::Process);
        assert!(item.can_stop);
        assert_eq!(item.control_id.as_deref(), Some("task-1"));
        assert_eq!(item.command.as_deref(), Some("cargo test"));
        assert_eq!(item.detail.as_deref(), Some("PID 4242"));
    }

    #[test]
    fn a_finished_sub_agent_carries_its_duration_and_cannot_be_stopped() {
        let item = background_item(BackgroundEntry {
            id: "agent-1".into(),
            kind: BackgroundKind::Subagent,
            title: "review the diff".into(),
            status: BackgroundStatus::Completed,
            detail: None,
            pid: None,
            output: Some("done".into()),
            started_at_ms: 1_000,
            finished_at_ms: Some(4_000),
        });
        assert_eq!(item.key.kind, BackgroundWorkKind::Subagent);
        assert_eq!(item.duration_ms, Some(3_000));
        assert!(!item.can_stop);
        assert!(item.command.is_none());
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
