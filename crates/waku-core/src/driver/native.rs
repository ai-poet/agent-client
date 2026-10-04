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
    BackgroundStatus, ComputerUseWiring, GoalOp, GoalSnapshot, GoalState, MissingApiKey,
    SubagentEvent, SubagentStatus, TurnOptions, UnknownEndpoint, WireFormat, split_model,
};

use super::activity;
use super::subagent::{ChildCall, SubagentFeed};
use crate::driver::{
    DriverControl, DriverEventSender, DriverEventSink, DriverStartOptions, SessionOptions,
};
use crate::model::{
    ActivityKind, AgentTurn, BackgroundWorkEvent, BackgroundWorkItem, BackgroundWorkKey,
    BackgroundWorkKind, BackgroundWorkStatus, CompactionPhase, DriverEvent, InteractionMode,
    Message, MessageRole, PermissionOption, ProviderResumeCursor, ProviderSessionHistory,
    RuntimeMode, SubagentCall, TurnStatus, UserInputAnswer, UserInputOption, UserInputQuestion,
    unix_time_millis,
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
    /// Held for its `Drop`, which reaps the helper processes this session
    /// registered and removes their directory.
    _computer_use: Option<super::computer_use::ComputerUseRuntime>,
}

/// Translate the runtime's resolved paths into what the bridge needs.
///
/// The bridge cannot call `crate::computer_use`'s resolvers itself — it
/// depends on neither this crate nor `waku-protocol` — so the paths cross as
/// plain values. The skill travels as its text rather than its path because
/// the engine's `Skill` tool reads flat files from its own config directory,
/// not the `SKILL.md` directories the app ships.
fn computer_use_wiring(config: &super::computer_use::ComputerUseConfig) -> ComputerUseWiring {
    ComputerUseWiring {
        repl_server: config.repl_path.clone(),
        native_helper: Some(config.server_path.clone()),
        process_directory: Some(config.process_directory.clone()),
        // A skill that cannot be read is not worth failing the session over;
        // the tools still work, only their manual is missing.
        skill_markdown: std::fs::read_to_string(&config.skill_path)
            .inspect_err(|error| {
                report_warning(&format!(
                    "the computer-use skill at {} could not be read: {error}",
                    config.skill_path.display()
                ));
            })
            .ok(),
        image_only: false,
    }
}

/// Fork addition: whether the desktop that spawned this daemon can drive its
/// in-app browser for the agent. It says so with `WAKU_IN_APP_BROWSER` when
/// it starts the daemon; a daemon nobody spawned has no browser to offer.
fn in_app_browser_offered() -> bool {
    std::env::var("WAKU_IN_APP_BROWSER").is_ok_and(|value| value == "1")
}

/// What a session gets of Computer Use.
///
/// Desktop control is set up only when its helper is installed, and a setup
/// that fails never fails the session or the message that started it
/// (`support::optional_computer_use`) — there is no event sender here to fail
/// it with. The REPL still goes in without the helper: it also carries image
/// generation, which needs no desktop access, and the session rules tell the
/// model that desktop control is off so it does not try.
fn session_computer_use(
    enabled: bool,
    start: impl FnOnce() -> anyhow::Result<super::computer_use::ComputerUseRuntime>,
    repl_server: impl FnOnce() -> anyhow::Result<std::path::PathBuf>,
) -> (
    Option<super::computer_use::ComputerUseRuntime>,
    Option<ComputerUseWiring>,
) {
    if !enabled {
        // Fork: drawing a picture needs no desktop access, so with the
        // switch off the REPL still comes, offering `generate_image` alone.
        // The helper is not even looked up.
        let wiring = repl_server().ok().map(|repl_server| ComputerUseWiring {
            repl_server,
            native_helper: None,
            process_directory: None,
            skill_markdown: None,
            image_only: true,
        });
        return (None, wiring);
    }
    match super::support::optional_computer_use(start()) {
        Some(runtime) => {
            let wiring = computer_use_wiring(&runtime.config);
            (Some(runtime), Some(wiring))
        }
        None => {
            let wiring = repl_server().ok().map(|repl_server| ComputerUseWiring {
                repl_server,
                native_helper: None,
                process_directory: None,
                skill_markdown: None,
                image_only: false,
            });
            (None, wiring)
        }
    }
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

        let (computer_use, wiring) = session_computer_use(
            options.computer_use_enabled,
            || super::computer_use::ComputerUseRuntime::start(events.clone()),
            crate::computer_use::js_repl_server_path,
        );

        let (platform, model) = route_of(options.model.as_deref());
        let start = AgentStartOptions {
            cwd: options.cwd.clone(),
            access_mode: access_mode(options.mode),
            plan_mode: options.interaction_mode == InteractionMode::Plan,
            model,
            platform,
            wire_format: wire_format_of(options.service_tier.as_deref()),
            reasoning_effort: options.reasoning_effort.clone(),
            narration_language: narration_language(),
            history,
            computer_use: wiring,
            session_id: Some(session_id.to_string()),
            browser_tools: in_app_browser_offered(),
        };

        let sink = EventTranslator::new(events.clone(), store.clone());
        // A missing key is the one start failure a user can fix without
        // reading code: say which route, and that signing in is the fix.
        let session = AgentSession::start(start, sink.into_sink()).map_err(|error| {
            if let Some(missing) = error.downcast_ref::<MissingApiKey>() {
                return anyhow!(tr!(
                    "native.no_api_key",
                    name = sub2api::brand::DISPLAY_NAME,
                    provider = missing.provider.clone(),
                    path = missing.settings_path.display().to_string()
                ));
            }
            // The model belongs to an endpoint of the user's own that is no
            // longer there; the fix is on the providers page, not in code.
            if error.downcast_ref::<UnknownEndpoint>().is_some() {
                return anyhow!(tr!("native.endpoint_unavailable"));
            }
            error
        })?;

        // Report the cursor immediately rather than after the first turn: the
        // transcript file is named by it, so a session closed before it ever
        // answered still reopens onto its own history.
        let _ = events.send(DriverEvent::Connected {
            provider_cursor: Some(ProviderResumeCursor::Native {
                session_id: session_id.to_string(),
            }),
        });
        // A resumed session shows its goal before anything else happens.
        session.goal(GoalOp::Refresh);

        Ok(Self {
            session,
            events,
            store,
            session_id,
            _computer_use: computer_use,
        })
    }
}

impl DriverControl for NativeDriver {
    fn prompt(&self, prompt: String) {
        if let Some(instructions) = compact_command(&prompt) {
            self.session.compact(instructions);
            return;
        }
        self.session.prompt(prompt);
    }

    /// The engine keeps one goal per session and pursues it across turns;
    /// the desktop's goal chip and dialog drive it the way they drive Codex.
    fn goal(&self, operation: crate::model::GoalOperation) {
        self.session.goal(goal_op(operation));
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

    fn browser_result(&self, request_id: String, result: Value) {
        self.session.browser_result(&request_id, result);
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
        let removed = self
            .session
            .rollback(turns)
            .map_err(localize_rewind_error)?;
        if removed == 0 {
            return Ok(None);
        }
        self.store.save(&self.session.history_snapshot());
        Ok(Some(ProviderResumeCursor::Native {
            session_id: self.session_id.to_string(),
        }))
    }

    fn fork(&self, turns_to_remove: usize) -> anyhow::Result<ProviderResumeCursor> {
        let branched = self
            .session
            .fork(turns_to_remove)
            .map_err(localize_rewind_error)?;
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

/// `/compact` and `/compact <instructions>`: the built-in agent's own
/// command, answered by compacting rather than by a model turn.
fn compact_command(prompt: &str) -> Option<Option<String>> {
    let rest = prompt.trim().strip_prefix("/compact")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    Some((!rest.is_empty()).then(|| rest.to_owned()))
}

/// The desktop's goal operation in the bridge's terms. The statuses only
/// Codex has — blocked, usage-limited — mean "not being pursued" here.
fn goal_op(operation: crate::model::GoalOperation) -> GoalOp {
    use crate::model::{GoalOperation, ThreadGoalStatus};
    match operation {
        GoalOperation::Refresh => GoalOp::Refresh,
        GoalOperation::Clear => GoalOp::Clear,
        GoalOperation::Set {
            objective,
            status,
            replace,
        } => GoalOp::Set {
            objective,
            status: status.map(|status| match status {
                ThreadGoalStatus::Active => GoalState::Active,
                ThreadGoalStatus::Complete => GoalState::Complete,
                ThreadGoalStatus::BudgetLimited => GoalState::BudgetLimited,
                ThreadGoalStatus::Paused
                | ThreadGoalStatus::Blocked
                | ThreadGoalStatus::UsageLimited => GoalState::Paused,
            }),
            replace,
        },
    }
}

fn thread_goal(goal: GoalSnapshot) -> crate::model::ThreadGoal {
    use crate::model::ThreadGoalStatus;
    crate::model::ThreadGoal {
        objective: goal.objective,
        status: match goal.status {
            GoalState::Active => ThreadGoalStatus::Active,
            GoalState::Paused => ThreadGoalStatus::Paused,
            GoalState::BudgetLimited => ThreadGoalStatus::BudgetLimited,
            GoalState::Complete => ThreadGoalStatus::Complete,
        },
        token_budget: goal
            .token_budget
            .map(|budget| i64::try_from(budget).unwrap_or(i64::MAX)),
        tokens_used: i64::try_from(goal.tokens_used).unwrap_or(i64::MAX),
        time_used_seconds: i64::try_from(goal.time_used_secs).unwrap_or(i64::MAX),
    }
}

fn token_totals(counts: waku_agent_bridge::TokenCounts) -> crate::usage_history::TokenTotals {
    crate::usage_history::TokenTotals {
        uncached_input: counts.uncached_input,
        cached_input: counts.cached_input,
        cache_creation: counts.cache_creation,
        output: counts.output,
        reasoning: 0,
    }
}

/// A rewind that would cut into a compaction summary is refused by the
/// bridge; the person sees why in their own language.
fn localize_rewind_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<waku_agent_bridge::history::RewindPastCompaction>()
        .is_some()
    {
        anyhow!(tr!("native.rewind_past_compaction"))
    } else {
        error
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

/// The language to tell the agent to narrate in, or `None` when that is
/// English and the instruction would be a line of prompt saying nothing.
///
/// Read from this process's locale, which the desktop pushes down with the
/// rest of its settings (`DaemonSettings::LOCALE_KEY`).
fn narration_language() -> Option<String> {
    let language = crate::i18n::current_language();
    (language.resolved() != crate::i18n::AppLanguage::English)
        .then(|| language.english_name().to_owned())
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
            pauses: Vec::new(),
            error: None,
            undone_at: None,
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
    /// The sub-agents running now, by the `Agent` call that started each.
    /// Kept apart from `tools`: a background one outlives the turn, and
    /// `tools` is cleared at every turn's start.
    subagents: Mutex<std::collections::HashMap<String, RunningSubagent>>,
    /// Fork (AgentTeams): a team member's record between its turns. A member
    /// runs many turns under one key; a fresh feed would restart the entry
    /// ids and overwrite its earlier rows.
    member_feeds: Mutex<std::collections::HashMap<String, SubagentFeed>>,
}

/// What a tool call looked like when it started, so its completion can be
/// rendered as the same row rather than a second one.
struct ToolCall {
    name: String,
    kind: ActivityKind,
    title: String,
    input: Value,
}

/// A sub-agent between its `Started` and its `Finished`: its panel entry as
/// last sent, and the stream of its record.
struct RunningSubagent {
    item: BackgroundWorkItem,
    feed: SubagentFeed,
}

/// Replace an engine refusal the fork owns the wording of with the user's
/// own language, or `None` for anything else.
///
/// The engine has no i18n — every message it produces is English — but these
/// two are ours (see `claurst_tools`), and they are the ones a user meets
/// while simply using plan mode, so they are the ones worth translating. The
/// match is against exported constants rather than prose, so a reworded
/// engine string is a compile-time concern rather than a silent regression.
///
/// Other refusals pass through: they name the specific command or path the
/// engine objected to, which is the useful part and not ours to restate.
fn localize_refusal(output: &Value) -> Option<Value> {
    // Only a plain-string result can be one of these; a structured output is
    // some tool's own payload.
    let trimmed = output.as_str()?.trim();
    if trimmed == waku_agent_bridge::KEEP_PLANNING_DENIAL {
        return Some(Value::String(tr!("native.keep_planning")));
    }
    if trimmed == waku_agent_bridge::MISSING_PLAN_ERROR {
        return Some(Value::String(tr!("native.plan_missing")));
    }
    trimmed
        .strip_suffix(waku_agent_bridge::PLAN_MODE_DENIAL_SUFFIX)
        .map(|_| Value::String(tr!("native.plan_mode_denied")))
}

/// The "finished planning" dialog's body, in the user's language.
///
/// The bridge sends either the plan — `ExitPlanMode`'s required `plan`,
/// exactly as the model wrote it — or its generic line when there is none.
/// The plan is kept whole under a translated lead-in; the generic line is
/// translated outright, told apart by comparing against the bridge's
/// exported constant rather than by guessing at the content. Nothing else
/// stands in for the plan: an earlier version took the model's last reply
/// when it was longer than the plan, and showed "Let me present the plan."
/// as the plan to approve.
fn localize_exit_plan_detail(detail: String) -> String {
    let plan = detail.trim();
    if plan.is_empty() || plan == waku_agent_bridge::EXIT_PLAN_MODE_DETAIL {
        tr!("plan.ready_detail")
    } else {
        format!("{}\n\n{plan}", tr!("plan.summary_lead"))
    }
}

impl EventTranslator {
    fn new(events: DriverEventSender, store: SessionStore) -> Self {
        Self {
            events,
            store,
            tools: Mutex::new(std::collections::HashMap::new()),
            subagents: Mutex::new(std::collections::HashMap::new()),
            member_feeds: Mutex::new(std::collections::HashMap::new()),
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
            AgentEvent::PlanModeChanged(plan) => {
                self.send(DriverEvent::InteractionModeUpdated(if plan {
                    InteractionMode::Plan
                } else {
                    InteractionMode::Build
                }));
            }
            AgentEvent::ToolStarted { id, name, input } => {
                // Fork: handing a plan over reads as that, not as the plan's
                // Markdown dumped into the row as escaped JSON; the plan
                // itself is read in the Plan panel.
                let input = if name == "ExitPlanMode" { Value::Null } else { input };
                let kind = activity_kind(&name);
                let title = if name == "ExitPlanMode" {
                    tr!("plan.handed_over")
                } else {
                    tool_title(&name, &input)
                };
                self.send(DriverEvent::RichActivity(
                    activity::tool_activity(
                        Some(id.clone()),
                        kind,
                        title.clone(),
                        Some(&input),
                        None,
                        None,
                        false,
                        false,
                    )
                    .with_subagent(SubagentCall::from_tool(&name, Some(&input))),
                ));
                self.tools.lock().insert(
                    id,
                    ToolCall {
                        name,
                        kind,
                        title,
                        input,
                    },
                );
            }
            AgentEvent::ToolFinished {
                id,
                name,
                output,
                failed,
                image_source,
            } => {
                // A completion whose start was never seen still deserves a
                // row; falling back on the tool name keeps it readable.
                let call = self.tools.lock().remove(&id).unwrap_or_else(|| ToolCall {
                    kind: activity_kind(&name),
                    title: name.clone(),
                    input: Value::Null,
                    name,
                });
                let output = localize_refusal(&output).unwrap_or(output);
                // Images arrive on their own sideband rather than inside
                // `output`, which is also what the model reads back.
                self.send(DriverEvent::RichActivity(
                    activity::tool_activity(
                        Some(id),
                        call.kind,
                        call.title,
                        Some(&call.input),
                        Some(&output),
                        image_source.as_ref(),
                        failed,
                        true,
                    )
                    .with_subagent(SubagentCall::from_tool(&call.name, Some(&call.input))),
                ));
            }
            AgentEvent::Subagent {
                parent_tool_id,
                event,
            } => self.handle_subagent(parent_tool_id, event),
            AgentEvent::Usage {
                context_tokens,
                context_window,
            } => self.send(DriverEvent::UsageUpdated {
                context_tokens,
                context_window,
            }),
            AgentEvent::GoalUpdated(goal) => {
                self.send(DriverEvent::GoalUpdated(goal.map(thread_goal)));
            }
            AgentEvent::TokenUsage { last, session } => self.send(DriverEvent::TokenUsageUpdated {
                last: token_totals(last),
                session: token_totals(session),
            }),
            AgentEvent::Compaction {
                phase,
                automatic,
                tokens_before,
                tokens_after,
            } => self.send(DriverEvent::ContextCompaction {
                phase: match phase {
                    waku_agent_bridge::CompactionPhase::Started => CompactionPhase::Started,
                    waku_agent_bridge::CompactionPhase::Finished => CompactionPhase::Finished,
                    waku_agent_bridge::CompactionPhase::Failed => CompactionPhase::Failed,
                },
                automatic,
                tokens_before,
                tokens_after,
            }),
            AgentEvent::Permission {
                request_id,
                tool_name,
                title,
                detail,
                options,
            } => {
                // The bridge has no i18n of its own (it depends on neither
                // this crate nor `waku-protocol`), so the one dialog whose
                // wording is ours rather than the engine's gets translated
                // here. Everything else keeps the engine's description, which
                // names the actual command and is the specific thing to
                // decide on.
                let plan = tool_name == "ExitPlanMode";
                let (title, detail) = if plan {
                    (tr!("plan.ready_title"), localize_exit_plan_detail(detail))
                } else {
                    (title, detail)
                };
                let options = options
                    .into_iter()
                    .map(|choice| PermissionOption {
                        id: choice.id().to_string(),
                        // The same two answers the Claude Code dialog offers,
                        // in the same words: "allow once" says nothing about
                        // what approving a plan starts.
                        label: if plan {
                            plan_answer_label(choice)
                        } else {
                            permission_label(choice)
                        },
                        allow: choice.is_allow(),
                    })
                    .collect();
                // Fork (AgentTeams): a member's dialog names the member asking.
                let title = match agent_teams::requests::requesting_member(&request_id) {
                    Some(member) => tr!("team.permission_title", member = member, title = title),
                    None => title,
                };
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
            // Fork addition: a browser tool's request, for the desktop.
            AgentEvent::BrowserRequest {
                request_id,
                operation,
            } => self.send(DriverEvent::BrowserRequest {
                request_id,
                operation,
            }),
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
            // The gateway accepted the request and returned nothing usable.
            // Name the route: the cause is upstream, and which model over
            // which API is the only part of it the user can change.
            AgentEvent::ProducedNothing {
                provider,
                model,
                api_base,
            } => self.send(DriverEvent::Error(tr!(
                "native.empty_turn",
                model = model,
                provider = provider,
                endpoint = api_base
            ))),
            AgentEvent::RouteNotFound {
                provider,
                model,
                url,
                detail,
            } => self.send(DriverEvent::Error(tr!(
                "native.route_not_found",
                model = model,
                provider = provider,
                url = url,
                detail = detail
            ))),
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

    /// One step of a sub-agent's run: its entry in the background-work
    /// panel, keyed by the `Agent` call that started it (so the call's row
    /// finds it), and its record, streamed into that entry.
    fn handle_subagent(&self, parent_tool_id: String, event: SubagentEvent) {
        let mut running = self.subagents.lock();
        match event {
            SubagentEvent::Started {
                description,
                prompt,
                model,
                background,
                started_at_ms,
            } => {
                let mut item = BackgroundWorkItem::new(
                    BackgroundWorkKind::Subagent,
                    parent_tool_id.clone(),
                    description,
                    BackgroundWorkStatus::Running,
                );
                item.origin_activity_id = Some(parent_tool_id.clone());
                item.command = Some(prompt);
                item.model = Some(model).filter(|model| !model.trim().is_empty());
                item.background = background;
                item.can_stop = true;
                item.control_id = Some(parent_tool_id.clone());
                item.started_at_ms = started_at_ms;
                let feed = self
                    .member_feeds
                    .lock()
                    .remove(&parent_tool_id)
                    .unwrap_or_else(|| SubagentFeed::new(item.key.clone()));
                self.send(DriverEvent::BackgroundWork(BackgroundWorkEvent::Upsert(
                    item.clone(),
                )));
                running.insert(parent_tool_id, RunningSubagent { item, feed });
            }
            SubagentEvent::Text(text) => {
                if let Some(event) = running
                    .get_mut(&parent_tool_id)
                    .and_then(|child| child.feed.text(&text))
                {
                    self.send(DriverEvent::BackgroundWork(event));
                }
            }
            SubagentEvent::ToolStarted { id, name, input } => {
                let Some(child) = running.get_mut(&parent_tool_id) else {
                    return;
                };
                let kind = activity_kind(&name);
                let title = tool_title(&name, &input);
                let row = activity::tool_activity(
                    Some(id.clone()),
                    kind,
                    title.clone(),
                    Some(&input),
                    None,
                    None,
                    false,
                    false,
                );
                child.feed.remember(&id, ChildCall { kind, title, input });
                self.send(DriverEvent::BackgroundWork(child.feed.activity(row)));
            }
            SubagentEvent::ToolFinished {
                id,
                name,
                output,
                failed,
                image_source,
            } => {
                let Some(child) = running.get_mut(&parent_tool_id) else {
                    return;
                };
                let call = child.feed.take(&id).unwrap_or_else(|| ChildCall {
                    kind: activity_kind(&name),
                    title: name.clone(),
                    input: Value::Null,
                });
                let output = localize_refusal(&output).unwrap_or(output);
                let row = activity::tool_activity(
                    Some(id),
                    call.kind,
                    call.title,
                    Some(&call.input),
                    Some(&output),
                    image_source.as_ref(),
                    failed,
                    true,
                );
                self.send(DriverEvent::BackgroundWork(child.feed.activity(row)));
            }
            SubagentEvent::Finished {
                status,
                summary,
                result,
                duration_ms,
            } => {
                let Some(RunningSubagent { mut item, mut feed }) = running.remove(&parent_tool_id) else {
                    return;
                };
                if parent_tool_id.contains("::team:") {
                    feed.close_text();
                    self.member_feeds.lock().insert(parent_tool_id.clone(), feed);
                }
                item.status = match status {
                    SubagentStatus::Completed => BackgroundWorkStatus::Completed,
                    SubagentStatus::Failed => BackgroundWorkStatus::Failed,
                    SubagentStatus::Stopped => BackgroundWorkStatus::Stopped,
                };
                item.can_stop = false;
                item.duration_ms = Some(duration_ms);
                item.updated_at_ms = unix_time_millis();
                item.detail = summary;
                // The final report, for any client that shows the entry but
                // not the record.
                item.output = result;
                self.send(DriverEvent::BackgroundWork(BackgroundWorkEvent::Upsert(item)));
            }
        }
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
        // The Computer Use REPL: driving the desktop reads as a command,
        // drawing reads as a tool.
        "waku_js_repl_js" | "waku_js_repl_js_reset" => ActivityKind::Command,
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
    // A sub-agent's task id is the id of the `Agent` call that started it
    // (the bridge files it so), which is what links the entry to its row.
    if kind == BackgroundWorkKind::Subagent {
        item.origin_activity_id = Some(entry.id.clone());
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
    // A sub-agent is named by the task it was given, as Claude Code's are.
    if let Some(call) = SubagentCall::from_tool(name, Some(input)) {
        return call.description;
    }
    // Fork: `prompt` last — a picture is named by what was asked for, not by
    // the tool's prefixed name ("Waku js repl generate image").
    for key in ["command", "query", "pattern", "file_path", "path", "url", "prompt"] {
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
/// The plan dialog's two answers. The bridge offers only once-scoped choices
/// for it, so these are the only two that can arrive.
fn plan_answer_label(choice: waku_agent_bridge::PermissionChoice) -> String {
    if choice.is_allow() {
        tr!("plan.approve")
    } else {
        tr!("plan.keep_planning")
    }
}

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

    /// A missing `cua-driver` used to reach the desktop as an error, which
    /// failed the message that started the session. Now the session simply
    /// starts without desktop control: no runtime, and the REPL wired for
    /// image generation alone — no helper, no skill, no process directory.
    #[test]
    fn a_missing_helper_turns_desktop_control_off_and_keeps_the_repl() {
        let (runtime, wiring) = super::session_computer_use(
            true,
            || Err(anyhow::anyhow!("Computer Use driver (cua-driver) is not installed")),
            || Ok("/opt/waku_js_repl".into()),
        );
        assert!(runtime.is_none());
        let wiring = wiring.expect("the REPL still carries image generation");
        assert_eq!(wiring.repl_server, std::path::PathBuf::from("/opt/waku_js_repl"));
        assert_eq!(wiring.native_helper, None);
        assert_eq!(wiring.process_directory, None);
        assert_eq!(wiring.skill_markdown, None);

        // A build without the REPL has nothing to wire at all.
        let (runtime, wiring) = super::session_computer_use(
            true,
            || Err(anyhow::anyhow!("Computer Use driver (cua-driver) is not installed")),
            || Err(anyhow::anyhow!("Waku JavaScript REPL is missing from this Waku build")),
        );
        assert!(runtime.is_none() && wiring.is_none());
    }

    /// With the switch off the helper is not looked up, let alone launched;
    /// the REPL still comes, to draw pictures and nothing else.
    #[test]
    fn computer_use_off_keeps_only_image_generation() {
        let (runtime, wiring) = super::session_computer_use(
            false,
            || panic!("the helper must not be resolved when Computer Use is off"),
            || Ok("/opt/waku_js_repl".into()),
        );
        assert!(runtime.is_none());
        let wiring = wiring.expect("the REPL is wired for image generation");
        assert!(wiring.image_only);
        assert_eq!(wiring.native_helper, None);
        assert_eq!(wiring.skill_markdown, None);

        let (runtime, wiring) = super::session_computer_use(
            false,
            || panic!("the helper must not be resolved when Computer Use is off"),
            || Err(anyhow::anyhow!("Waku JavaScript REPL is missing from this Waku build")),
        );
        assert!(runtime.is_none() && wiring.is_none());
    }

    /// The bridge cannot resolve bundle paths itself, so the driver hands
    /// them over as values — and the skill as text, because the engine's
    /// `Skill` tool reads flat files rather than the `SKILL.md` directories
    /// the app ships.
    #[test]
    fn the_wiring_carries_paths_across_and_reads_the_skill() {
        let dir = std::env::temp_dir().join(format!("waku-wiring-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let skill_path = dir.join("SKILL.md");
        std::fs::write(&skill_path, "# how to drive the desktop").expect("write skill");

        let config = super::super::computer_use::ComputerUseConfig {
            server_path: PathBuf::from("/opt/helper"),
            repl_path: PathBuf::from("/opt/waku_js_repl"),
            skill_path: skill_path.clone(),
            process_directory: dir.clone(),
        };
        let wiring = computer_use_wiring(&config);
        assert_eq!(wiring.repl_server, PathBuf::from("/opt/waku_js_repl"));
        assert_eq!(wiring.native_helper, Some(PathBuf::from("/opt/helper")));
        assert_eq!(wiring.process_directory, Some(dir.clone()));
        assert_eq!(wiring.skill_markdown.as_deref(), Some("# how to drive the desktop"));

        // An unreadable skill costs the manual, not the session.
        std::fs::remove_file(&skill_path).expect("remove skill");
        assert!(computer_use_wiring(&config).skill_markdown.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn translator() -> (EventTranslator, crossbeam_channel::Receiver<DriverEvent>) {
        let (events, received) = crate::driver::test_event_channel();
        (EventTranslator::new(events, SessionStore::new(Uuid::new_v4())), received)
    }

    /// An `Agent` call reads as the task it hands over, and carries the mark
    /// that makes the transcript show it as a sub-agent — on its start and
    /// on its completion alike.
    #[test]
    fn an_agent_call_is_titled_by_its_task_and_marked_as_a_sub_agent() {
        let (translator, received) = translator();
        let input = serde_json::json!({"description": "Find the login code", "prompt": "Look for it"});
        translator.handle(AgentEvent::ToolStarted {
            id: "toolu_1".into(),
            name: "Agent".into(),
            input,
        });
        translator.handle(AgentEvent::ToolFinished {
            id: "toolu_1".into(),
            name: "Agent".into(),
            output: Value::String("Found it in auth.rs".into()),
            failed: false,
            image_source: None,
        });
        let rows: Vec<_> = received
            .try_iter()
            .filter_map(|event| match event {
                DriverEvent::RichActivity(row) => Some(row),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(row.title, "Find the login code");
            assert_eq!(
                row.subagent.as_ref().map(|call| call.description.as_str()),
                Some("Find the login code")
            );
        }
    }

    /// A sub-agent's run becomes one panel entry keyed by the call that
    /// started it, with its text and tool calls streamed into its record,
    /// and settles with its report.
    #[test]
    fn a_sub_agents_run_streams_into_one_entry() {
        let (translator, received) = translator();
        let step = |event| {
            translator.handle(AgentEvent::Subagent {
                parent_tool_id: "toolu_1".into(),
                event,
            })
        };
        step(SubagentEvent::Started {
            description: "Find it".into(),
            prompt: "Look for the login code".into(),
            model: "claude-sonnet-5".into(),
            background: false,
            started_at_ms: 1_000,
        });
        step(SubagentEvent::Text("Looking ".into()));
        step(SubagentEvent::Text("around.".into()));
        step(SubagentEvent::ToolStarted {
            id: "child_1".into(),
            name: "Grep".into(),
            input: serde_json::json!({"pattern": "login"}),
        });
        step(SubagentEvent::ToolFinished {
            id: "child_1".into(),
            name: "Grep".into(),
            output: Value::String("src/auth.rs".into()),
            failed: false,
            image_source: None,
        });
        step(SubagentEvent::Finished {
            status: SubagentStatus::Completed,
            summary: None,
            result: Some("It is in src/auth.rs".into()),
            duration_ms: 4_000,
        });

        let work: Vec<BackgroundWorkEvent> = received
            .try_iter()
            .filter_map(|event| match event {
                DriverEvent::BackgroundWork(work) => Some(work),
                _ => None,
            })
            .collect();
        let BackgroundWorkEvent::Upsert(started) = &work[0] else {
            panic!("expected the entry first");
        };
        assert_eq!(started.key, BackgroundWorkKey::new(BackgroundWorkKind::Subagent, "toolu_1"));
        assert_eq!(started.origin_activity_id.as_deref(), Some("toolu_1"));
        assert_eq!(started.control_id.as_deref(), Some("toolu_1"));
        assert_eq!(started.command.as_deref(), Some("Look for the login code"));
        assert!(started.can_stop);

        let entries: Vec<_> = work[1..work.len() - 1]
            .iter()
            .map(|event| match event {
                BackgroundWorkEvent::Transcript { entry, .. } => entry.clone(),
                other => panic!("expected a record entry, got {other:?}"),
            })
            .collect();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].id, entries[1].id, "the text runs into one entry");
        assert!(matches!(
            &entries[1].body,
            crate::model::SubagentTranscriptBody::Text { append: true, .. }
        ));
        assert_eq!(entries[2].id, "child_1");
        let crate::model::SubagentTranscriptBody::Activity { activity } = &entries[3].body else {
            panic!("expected the finished call");
        };
        assert!(activity.complete);
        assert_eq!(activity.output.as_deref(), Some("src/auth.rs"));

        let Some(BackgroundWorkEvent::Upsert(done)) = work.last() else {
            panic!("expected the settled entry last");
        };
        assert_eq!(done.status, BackgroundWorkStatus::Completed);
        assert_eq!(done.output.as_deref(), Some("It is in src/auth.rs"));
        assert!(!done.can_stop);
    }

    /// A background sub-agent the registry reports links back to its row.
    #[test]
    fn a_registry_sub_agent_links_back_to_its_row() {
        let item = background_item(BackgroundEntry {
            id: "toolu_7".into(),
            kind: BackgroundKind::Subagent,
            title: "Survey the crate".into(),
            status: BackgroundStatus::Running,
            detail: None,
            pid: None,
            output: None,
            started_at_ms: 0,
            finished_at_ms: None,
        });
        assert_eq!(item.origin_activity_id.as_deref(), Some("toolu_7"));
    }

    /// Driving the desktop reads as a command in the transcript, not as the
    /// generic tool row the name would otherwise fall through to.
    #[test]
    fn the_repl_tools_classify_as_commands() {
        assert!(matches!(activity_kind("waku_js_repl_js"), ActivityKind::Command));
        assert!(matches!(activity_kind("waku_js_repl_js_reset"), ActivityKind::Command));
    }

    /// The generic line is translated as a whole; a plan summary is kept
    /// under a lead-in, so the user reads the plan and not a stand-in for it.
    ///
    /// Asserted on shape rather than on wording: under the English locale the
    /// generic line's translation is the constant itself, so "differs from
    /// the constant" would be a false test. What holds in every locale is
    /// that only the summary gets a lead-in prepended.
    #[test]
    fn the_plan_summary_survives_localization_and_the_generic_line_does_not() {
        let generic = localize_exit_plan_detail(waku_agent_bridge::EXIT_PLAN_MODE_DETAIL.to_owned());
        assert!(!generic.contains("

"), "generic line must not get a lead-in: {generic}");

        let summary = "1. Add the field. 2. Wire the picker.";
        let kept = localize_exit_plan_detail(summary.to_owned());
        assert!(kept.contains("

"), "summary must sit under a lead-in: {kept}");
        assert!(kept.ends_with(summary), "the plan itself must be intact: {kept}");
    }

    /// The dialog shows the plan the tool carried and nothing else: the
    /// model's reply around the call never stands in for it.
    #[test]
    fn the_plan_to_approve_is_the_plan_the_tool_carried() {
        let plan = "## Plan\n\n1. Add the field.\n2. Wire the picker.";
        let detail = localize_exit_plan_detail(format!("  {plan}\n"));
        assert!(detail.ends_with(plan), "the plan, whole: {detail}");
        assert_eq!(localize_exit_plan_detail("   ".to_owned()), tr!("plan.ready_detail"));
    }

    /// A call with no plan reaches the transcript in the user's language,
    /// and the row of a plan handed over does not carry the plan as JSON.
    #[test]
    fn a_missing_plan_is_translated_and_the_plan_is_not_dumped_into_its_row() {
        let missing = Value::String(waku_agent_bridge::MISSING_PLAN_ERROR.to_owned());
        assert_eq!(
            localize_refusal(&missing),
            Some(Value::String(tr!("native.plan_missing")))
        );

        let (translator, received) = translator();
        translator.handle(AgentEvent::ToolStarted {
            id: "t1".into(),
            name: "ExitPlanMode".into(),
            input: serde_json::json!({"plan": "## Plan\n\n1. Do it."}),
        });
        let Ok(DriverEvent::RichActivity(item)) = received.try_recv() else {
            panic!("a row for the call");
        };
        assert!(item.arguments.is_none(), "{:?}", item.arguments);
    }

    /// The two refusals the fork owns the wording of are recognised by their
    /// exported markers, not by matching prose — a reworded engine string
    /// breaks the build rather than silently shipping English to everyone.
    #[test]
    fn our_own_refusals_are_translated_and_others_are_left_alone() {
        let denial = Value::String(format!(
            "Permission denied for tool 'Bash'{}",
            waku_agent_bridge::PLAN_MODE_DENIAL_SUFFIX
        ));
        assert!(localize_refusal(&denial).is_some());
        assert!(
            localize_refusal(&Value::String(
                waku_agent_bridge::KEEP_PLANNING_DENIAL.to_owned()
            ))
            .is_some()
        );

        // A refusal that names the specific thing the engine objected to is
        // the useful part, and not ours to restate.
        assert!(
            localize_refusal(&Value::String(
                "Permission denied for tool 'Bash'".to_owned()
            ))
            .is_none()
        );
        assert!(localize_refusal(&Value::String("ok".to_owned())).is_none());
        // A structured result is some tool's own payload.
        assert!(localize_refusal(&serde_json::json!({"files": []})).is_none());
    }
    use super::*;

    #[test]
    fn the_picker_id_carries_the_platform_and_the_tier_carries_the_format() {
        assert_eq!(
            route_of(Some("openai::gpt-5.6-sol")),
            (Some("openai".into()), Some("gpt-5.6-sol".into()))
        );
        assert_eq!(route_of(Some("claude-sonnet-5")), (None, Some("claude-sonnet-5".into())));
        // A model on an endpoint of the user's own carries that endpoint as
        // its platform, and its model id intact.
        assert_eq!(
            route_of(Some("custom:pr-1::glm-5.1")),
            (Some("custom:pr-1".into()), Some("glm-5.1".into()))
        );
        assert_eq!(
            route_of(Some("custom:pr-1::qwen3:32b")),
            (Some("custom:pr-1".into()), Some("qwen3:32b".into()))
        );
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
