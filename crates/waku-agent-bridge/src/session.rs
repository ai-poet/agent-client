//! One conversation, driven by the vendored engine.
//!
//! The whole design follows from one line of the engine's signature:
//!
//! ```ignore
//! run_query_loop(client, messages: &mut Vec<Message>, tools, ctx, config, …)
//! ```
//!
//! The caller owns the transcript. So this session holds it, hands the loop a
//! working copy for the duration of a turn, and takes it back afterwards —
//! which is what makes rewind, branch and resume ordinary vector operations
//! (see [`crate::history`]) instead of a protocol negotiation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use claurst_api::AnthropicClient;
use claurst_api::client::ClientConfig;
use claurst_core::config::{Config, Settings};
use claurst_core::types::Message;
use claurst_core::{CostTracker, PermissionManager};
use claurst_query::{
    CommandPriority, CommandQueue, QueryConfig, QueryEvent, QueryOutcome, QueuedCommand,
};
use claurst_tools::{Tool, ToolContext, UserQuestionEvent};
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::background;
use crate::config::{
    AgentStartOptions, TurnOptions, build_config, build_config_from, build_query_config,
    load_settings, missing_route_key,
};
use crate::events::{AgentEvent, EventSink, PermissionChoice, StreamDecoder};
use crate::history;
use crate::mcp_tool::McpTool;
use crate::permission::{GuiPermissionHandler, PermissionBridge};
use crate::runtime;

/// How often the steer watcher checks whether the queue was drained. The
/// engine drains between turns, so this only decides how promptly the composer
/// learns the message landed — not when it lands.
const STEER_POLL: Duration = Duration::from_millis(120);

/// A live conversation. Cloneable; every clone drives the same session.
#[derive(Clone)]
pub struct AgentSession {
    inner: Arc<Inner>,
}

type ToolSet = Arc<Vec<Box<dyn Tool>>>;

/// Engine tools that exist but do nothing in this process. Their runners —
/// the team swarm, the inbox reader, the cron scheduler, remote triggers —
/// belong to the engine's CLI, which is not here, so a model that calls them
/// is told it succeeded at something that never happens.
pub(crate) const UNAVAILABLE_TOOLS: [&str; 7] = [
    "TeamCreate",
    "TeamDelete",
    "SendMessage",
    "CronCreate",
    "CronDelete",
    "CronList",
    "RemoteTrigger",
];

/// Offered only while a goal is active: with none, calling it would report a
/// goal completed that was never set.
pub(crate) const GOAL_COMPLETE_TOOL: &str = "GoalComplete";

/// The two tool sets a turn can start with — with `GoalComplete` and
/// without. Built side by side because a boxed tool cannot be cloned.
#[derive(Clone)]
struct ToolSets {
    plain: ToolSet,
    goal: ToolSet,
}

impl ToolSets {
    fn build(disallowed: &[String], mcp: Option<&Arc<claurst_mcp::McpManager>>) -> Self {
        Self {
            plain: builtin_tools(disallowed, mcp, false),
            goal: builtin_tools(disallowed, mcp, true),
        }
    }
}

struct Inner {
    events: EventSink,
    id: String,
    cwd: PathBuf,
    /// Rebuilt with the registry when the route changes — a different key,
    /// platform or wire format — so an option change never sends the next
    /// turn through the previous one's credentials.
    client: Mutex<Arc<AnthropicClient>>,
    /// Replaced, not mutated, when the MCP roster connects: a turn already
    /// running keeps the set it started with.
    tools: Mutex<ToolSets>,
    cost_tracker: Arc<CostTracker>,
    file_history: Arc<Mutex<claurst_core::file_history::FileHistory>>,
    manager: Arc<std::sync::Mutex<PermissionManager>>,
    bridge: Arc<PermissionBridge>,
    /// The settings document the session started from, shared with the
    /// permission bridge (which appends rules to it). The fallback when the
    /// file cannot be re-read mid-session.
    settings: Arc<Mutex<Settings>>,
    /// Filled in the background once the MCP roster has connected. A turn that
    /// starts before then simply runs without MCP rather than waiting for
    /// servers that may never come up.
    mcp: Mutex<Option<Arc<claurst_mcp::McpManager>>>,
    config: Mutex<Config>,
    query: Mutex<QueryConfig>,
    options: Mutex<AgentStartOptions>,
    history: Mutex<Vec<Message>>,
    turn: Mutex<Option<Turn>>,
    turn_counter: Arc<AtomicUsize>,
    /// `AskUserQuestion` calls waiting on the user, by request id.
    questions: Mutex<HashMap<String, oneshot::Sender<String>>>,
}

/// The parts of a running turn that outside callers need to reach.
struct Turn {
    cancel: CancellationToken,
    queue: CommandQueue,
    /// Steering messages pushed but not yet observed as drained.
    steers: Arc<Mutex<Vec<String>>>,
    /// Set once a watcher is running, so a second steer does not start a
    /// second one.
    watching: bool,
}

impl AgentSession {
    /// Build a session and its engine runtime. Does not contact the model.
    pub fn start(options: AgentStartOptions, events: EventSink) -> anyhow::Result<Self> {
        let loaded = load_settings()?;
        let mut config = build_config_from(loaded.clone(), &options);
        crate::config::apply_compaction_settings(
            &mut config,
            crate::config::settings_document().as_ref(),
        );
        let settings = Arc::new(Mutex::new(loaded));
        let mut query = build_query_config(&config, &options);

        let (client, registry) = build_clients(&config, options.platform.as_deref())?;
        query.provider_registry = Some(registry);

        let manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
            config.permission_mode.clone(),
            &settings.lock(),
        )));
        let bridge = PermissionBridge::new(
            events.clone(),
            manager.clone(),
            settings.clone(),
            options.access_mode.auto_answer(),
        );
        // Turning Computer Use on is the consent for its tools; the skill
        // that documents them is written where the engine's `Skill` tool
        // looks, and consented too so reading it raises nothing.
        match options.computer_use.as_ref() {
            Some(wiring) => {
                let skill = wiring.skill_markdown.as_deref().and_then(|markdown| {
                    crate::computer_use::install_skill(markdown)
                        .inspect_err(|error| {
                            tracing::warn!(%error, "agent: computer-use skill not installed");
                        })
                        .ok()
                });
                let tools = crate::config::COMPUTER_USE_TOOLS
                    .iter()
                    .map(|tool| (*tool).to_owned())
                    .collect();
                bridge.set_consented(tools, skill);
            }
            None => crate::computer_use::remove_skill(),
        }

        let tools = ToolSets::build(&config.disallowed_tools, None);
        let inner = Arc::new(Inner {
            events,
            id: options
                .session_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            cwd: options.cwd.clone(),
            client: Mutex::new(client),
            tools: Mutex::new(tools),
            cost_tracker: CostTracker::new(),
            file_history: Arc::new(Mutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            manager,
            bridge,
            settings,
            mcp: Mutex::new(None),
            config: Mutex::new(config),
            query: Mutex::new(query),
            history: Mutex::new(history::deserialize(&options.history)),
            options: Mutex::new(options),
            turn: Mutex::new(None),
            turn_counter: Arc::new(AtomicUsize::new(0)),
            questions: Mutex::new(HashMap::new()),
        });

        connect_mcp_in_background(&inner);
        Ok(Self { inner })
    }

    /// Whether a turn is currently running.
    pub fn is_busy(&self) -> bool {
        self.inner.turn.lock().is_some()
    }

    /// Start a turn. While one is running this is delivered as steering
    /// instead, which is what the composer means by sending during a turn.
    pub fn prompt(&self, text: String) {
        let inner = self.inner.clone();
        let Ok(rt) = runtime::shared() else {
            inner
                .events
                .emit(AgentEvent::Error("the agent runtime is unavailable".to_string()));
            return;
        };

        // Decide and claim the turn under one lock, so two prompts arriving
        // together cannot both see "idle" and both start a loop over the same
        // transcript.
        let (cancel, queue) = {
            let mut guard = inner.turn.lock();
            if let Some(turn) = guard.as_mut() {
                push_steer(&inner, turn, text);
                return;
            }
            let cancel = CancellationToken::new();
            let queue = CommandQueue::new();
            *guard = Some(Turn {
                cancel: cancel.clone(),
                queue: queue.clone(),
                steers: Arc::new(Mutex::new(Vec::new())),
                watching: false,
            });
            (cancel, queue)
        };

        rt.spawn(async move {
            run_turn(inner, text, cancel, queue).await;
        });
    }

    /// Inject a message into the running turn. It reaches the conversation at
    /// the next turn boundary — after the tools now running finish, before the
    /// next request goes out.
    pub fn steer(&self, text: String) {
        let mut guard = self.inner.turn.lock();
        let Some(turn) = guard.as_mut() else {
            self.inner.events.emit(AgentEvent::SteerRejected {
                message: text,
                reason: "no turn is running".to_string(),
            });
            return;
        };
        push_steer(&self.inner, turn, text);
    }

    /// Stop the running turn. Releases anything blocked on an approval or a
    /// question so no tool is left waiting on a dialog that is going away.
    pub fn cancel(&self) {
        let cancel = self
            .inner
            .turn
            .lock()
            .as_ref()
            .map(|turn| turn.cancel.clone());
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        self.inner.bridge.release_all();
        // Dropping the reply senders makes each waiting `AskUserQuestion`
        // return an error to the model, which the cancelled loop discards.
        self.inner.questions.lock().clear();
    }

    /// Change the session's goal and report it back as
    /// [`AgentEvent::GoalUpdated`]. A goal set or resumed while nothing runs
    /// starts pursuing it at once; one set during a turn is picked up when
    /// that turn ends.
    pub fn goal(&self, op: crate::goal::GoalOp) {
        let Some(store) = crate::goal::open_store() else {
            self.inner.events.emit(AgentEvent::Error(
                "goals are unavailable: the goal store could not be opened".to_owned(),
            ));
            return;
        };
        match crate::goal::apply(&store, &self.inner.id, op) {
            Err(error) => self.inner.events.emit(AgentEvent::Error(error)),
            Ok(effect) => {
                self.inner.events.emit(AgentEvent::GoalUpdated(crate::goal::snapshot(
                    &store,
                    &self.inner.id,
                )));
                match effect {
                    crate::goal::GoalEffect::Kickoff(prompt)
                    | crate::goal::GoalEffect::Resume(prompt)
                        if !self.is_busy() =>
                    {
                        self.prompt(prompt);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Fold the conversation so far into a summary now — `/compact`, with the
    /// person's own instructions for what the summary should keep.
    ///
    /// Holds the turn slot while it runs, so a prompt waits behind it and a
    /// stop cancels it. It never reports a `TurnStarted`: nothing is added to
    /// the conversation Waku could count as a turn, and the summary it leaves
    /// stands for the turns it replaced.
    pub fn compact(&self, instructions: Option<String>) {
        let inner = self.inner.clone();
        let Ok(rt) = runtime::shared() else {
            inner
                .events
                .emit(AgentEvent::Error("the agent runtime is unavailable".to_string()));
            return;
        };
        let cancel = {
            let mut guard = inner.turn.lock();
            if guard.is_some() {
                drop(guard);
                inner.events.emit(AgentEvent::Error(
                    "the conversation cannot be compacted while a turn is running".to_string(),
                ));
                return;
            }
            let cancel = CancellationToken::new();
            *guard = Some(Turn {
                cancel: cancel.clone(),
                queue: CommandQueue::new(),
                steers: Arc::new(Mutex::new(Vec::new())),
                watching: false,
            });
            cancel
        };
        rt.spawn(async move {
            compact_now(inner, instructions, cancel).await;
        });
    }

    /// Answer a [`AgentEvent::Permission`].
    pub fn respond(&self, request_id: &str, option_id: &str) {
        let Some(choice) = PermissionChoice::from_id(option_id) else {
            tracing::warn!(option_id, "agent: unknown permission option");
            return;
        };
        self.inner.bridge.resolve(request_id, choice);
    }

    /// Answer a [`AgentEvent::UserInput`]. A late answer to a question the
    /// cancel path already released is dropped, not an error.
    pub fn answer(&self, request_id: &str, text: String) {
        let reply = self.inner.questions.lock().remove(request_id);
        if let Some(reply) = reply {
            let _ = reply.send(text);
        }
    }

    /// Publish the current state of every piece of background work.
    pub fn refresh_background_work(&self) {
        self.inner
            .events
            .emit(AgentEvent::BackgroundWork(background::snapshot()));
    }

    /// Stop one piece of background work by the id the snapshot reported.
    /// The refreshed snapshot follows either way, so the panel shows the
    /// entry's real state rather than an optimistic one.
    pub fn stop_background_work(&self, id: &str) -> Result<(), String> {
        let outcome = background::stop(id);
        self.refresh_background_work();
        outcome
    }

    /// Apply changed turn options in place.
    ///
    /// Always succeeds: model, effort and access mode all ride on the next
    /// turn's `QueryConfig` and permission policy, so there is nothing a
    /// change here could require a restart for.
    pub fn apply_options(&self, changes: TurnOptions) -> bool {
        let mut options = self.inner.options.lock();
        if let Some(mode) = changes.access_mode {
            options.access_mode = mode;
        }
        if let Some(plan) = changes.plan_mode {
            options.plan_mode = plan;
        }
        // The effort belongs to the model: a switch carries the new model's
        // effort even when that is none, so the previous model's does not
        // ride along to one that never asked for it.
        if changes.model.is_some() {
            options.model = changes.model;
            options.reasoning_effort = changes.reasoning_effort.clone();
        }
        if let Some(platform) = changes.platform {
            options.platform = platform;
        }
        if let Some(format) = changes.wire_format {
            options.wire_format = format;
        }
        if changes.reasoning_effort.is_some() {
            options.reasoning_effort = changes.reasoning_effort;
        }

        let config = match build_config(&options) {
            Ok(config) => config,
            Err(error) => {
                self.inner
                    .events
                    .emit(AgentEvent::Error(format!("{error:#}")));
                return true;
            }
        };
        let mut query = build_query_config(&config, &options);
        let route_changed = {
            let current = self.inner.config.lock();
            current.provider != config.provider
                || current.api_key != config.api_key
                || current.provider_configs.get("anthropic").map(|entry| entry.api_base.clone())
                    != config.provider_configs.get("anthropic").map(|entry| entry.api_base.clone())
        };
        if route_changed {
            match build_clients(&config, options.platform.as_deref()) {
                Ok((client, registry)) => {
                    *self.inner.client.lock() = client;
                    query.provider_registry = Some(registry);
                }
                Err(error) => {
                    self.inner.events.emit(AgentEvent::Error(format!(
                        "could not switch the agent's route: {error:#}"
                    )));
                    return true;
                }
            }
        }
        self.inner.bridge.set_auto(options.access_mode.auto_answer());
        if let Ok(mut manager) = self.inner.manager.lock() {
            // The manager caches the mode it evaluates against; rebuild it so
            // a switch to Full Access stops asking immediately rather than at
            // the next session.
            // Re-read so rules persisted by another session count; an
            // unreadable file keeps the rules this session already has
            // rather than dropping to none.
            let settings = load_settings().unwrap_or_else(|error| {
                tracing::warn!(%error, "agent: settings unreadable; keeping this session's rules");
                self.inner.settings.lock().clone()
            });
            *manager = PermissionManager::new(config.permission_mode.clone(), &settings);
        }

        {
            // Held one at a time, and in the same order `run_turn` reads them,
            // so the two can never wait on each other.
            let mut current_query = self.inner.query.lock();
            // The registry owns live provider clients; it is only rebuilt when
            // the route changed, so a plain model switch keeps its pools.
            if query.provider_registry.is_none() {
                query.provider_registry = current_query.provider_registry.clone();
            }
            *current_query = query;
        }
        *self.inner.config.lock() = config;
        true
    }

    /// The conversation, serialized for Waku's session store.
    pub fn history_snapshot(&self) -> Vec<u8> {
        history::serialize(&self.inner.history.lock())
    }

    /// Number of user-authored turns, which is what the rewind control counts.
    pub fn turn_count(&self) -> usize {
        history::turn_count(&self.inner.history.lock())
    }

    /// Drop the last `turns` user turns. Refused while a turn is running: the
    /// loop holds a working copy, and truncating underneath it would be
    /// undone the moment that copy is written back. Also refused, with
    /// [`history::RewindPastCompaction`], when the cut would fall inside a
    /// compaction summary.
    pub fn rollback(&self, turns: usize) -> anyhow::Result<usize> {
        if self.is_busy() {
            anyhow::bail!("cannot rewind while a turn is running");
        }
        let removed = history::rollback(&mut self.inner.history.lock(), turns)?;
        Ok(removed)
    }

    /// A copy of the conversation with the last `turns_to_remove` turns
    /// dropped, ready to seed a branch. Leaves this session untouched.
    pub fn fork(&self, turns_to_remove: usize) -> anyhow::Result<Vec<u8>> {
        let branched = history::fork(&self.inner.history.lock(), turns_to_remove)?;
        Ok(history::serialize(&branched))
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(turn) = self.turn.lock().as_ref() {
            turn.cancel.cancel();
        }
        self.bridge.release_all();
        self.questions.lock().clear();
    }
}

/// The Anthropic client and the provider registry for a route.
///
/// Both come from the same `Config`, so whichever wire format the route
/// selects, the credentials are the ones `select_route` chose. Resolving the
/// key may refresh an OAuth token, hence the `block_on`; the caller is never
/// on the runtime.
pub(crate) fn build_clients(
    config: &Config,
    platform: Option<&str>,
) -> anyhow::Result<(Arc<AnthropicClient>, Arc<claurst_api::ProviderRegistry>)> {
    let rt = runtime::shared()?;
    let (api_key, use_bearer_auth) = rt
        .block_on(config.resolve_anthropic_auth_async())
        .unwrap_or_default();
    // Refuse to build a keyless client. The engine would accept one and fail
    // on the first request with advice about a CLI this product does not
    // ship; failing here names the route and the file instead.
    let resolved = Some(api_key.as_str())
        .filter(|key| !key.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| config.resolve_provider_api_key(config.selected_provider_id()));
    if let Some(missing) = missing_route_key(config, platform, resolved.as_deref()) {
        return Err(missing.into());
    }
    let client_config = ClientConfig {
        api_key,
        api_base: config.resolve_anthropic_api_base(),
        use_bearer_auth,
        ..Default::default()
    };
    let client = Arc::new(AnthropicClient::new(client_config.clone())?);
    let registry = Arc::new(claurst_api::ProviderRegistry::from_config(config, client_config));
    Ok((client, registry))
}

/// The built-in tools, minus the ones the user switched off, plus every
/// tool the connected MCP servers advertise.
///
/// Filtering here rather than in the prompt is what makes the Tools page
/// reliable: a tool that is not in this list is not sent to the model at all,
/// so there is nothing for it to be talked into.
fn builtin_tools(
    disallowed: &[String],
    mcp: Option<&Arc<claurst_mcp::McpManager>>,
    with_goal: bool,
) -> ToolSet {
    let mut tools: Vec<Box<dyn Tool>> = claurst_tools::all_tools();
    tools.push(Box::new(claurst_query::AgentTool));
    tools.retain(|tool| !disallowed.iter().any(|name| name == tool.name()));
    tools.retain(|tool| !UNAVAILABLE_TOOLS.contains(&tool.name()));
    if !with_goal {
        tools.retain(|tool| tool.name() != GOAL_COMPLETE_TOOL);
    }
    // The PowerShell tool runs `pwsh`, which a Mac or Linux machine almost
    // never has; a tool the model can call but not run only teaches it to
    // fail. Claude Code and Pi offer it on Windows alone.
    if !cfg!(windows) {
        tools.retain(|tool| tool.name() != "PowerShell");
    }
    if let Some(manager) = mcp {
        tools.extend(McpTool::all(manager));
    }
    Arc::new(tools)
}

/// Connect the configured MCP servers without holding up session start, and
/// swap in a tool set that includes theirs once they are up.
fn connect_mcp_in_background(inner: &Arc<Inner>) {
    let servers = inner.config.lock().mcp_servers.clone();
    if servers.is_empty() {
        return;
    }
    let Ok(rt) = runtime::shared() else { return };
    let inner = inner.clone();
    rt.spawn(async move {
        let manager = Arc::new(claurst_mcp::McpManager::connect_all(&servers).await);
        manager.clone().spawn_notification_poll_loop();
        // A server that did not connect costs its own tools and nothing
        // more, so it is logged rather than raised: `AgentEvent::Error`
        // reaches the desktop as the failure of the turn in progress, and
        // this runs while the first one is starting. That includes the
        // Computer Use REPL, which must never keep a message from being
        // answered.
        for (server, error) in manager.failed_servers() {
            tracing::warn!(%server, %error, "agent: MCP server did not connect");
        }
        let disallowed = inner.config.lock().disallowed_tools.clone();
        *inner.tools.lock() = ToolSets::build(&disallowed, Some(&manager));
        *inner.mcp.lock() = Some(manager);
    });
}

fn push_steer(inner: &Arc<Inner>, turn: &mut Turn, text: String) {
    turn.queue.push(
        QueuedCommand::InjectUserMessage(text.clone()),
        CommandPriority::Normal,
    );
    turn.steers.lock().push(text);
    if !turn.watching {
        turn.watching = true;
        spawn_steer_watcher(inner.clone(), turn.queue.clone(), turn.steers.clone());
    }
}

/// Emit `SteerAccepted` once the engine has taken the queue.
///
/// `CommandQueue::drain` empties the whole queue at once, so an empty queue
/// means every message pushed before it was delivered — in the order they were
/// pushed.
fn spawn_steer_watcher(inner: Arc<Inner>, queue: CommandQueue, steers: Arc<Mutex<Vec<String>>>) {
    let Ok(rt) = runtime::shared() else { return };
    rt.spawn(async move {
        loop {
            tokio::time::sleep(STEER_POLL).await;
            // The turn ending is handled by `run_turn`, which settles whatever
            // is still pending. Stopping here avoids a double report.
            if inner.turn.lock().is_none() {
                return;
            }
            if !queue.is_empty() {
                continue;
            }
            let delivered: Vec<String> = steers.lock().drain(..).collect();
            for message in delivered {
                inner.events.emit(AgentEvent::SteerAccepted { message });
            }
        }
    });
}

/// Forward `AskUserQuestion` calls to the GUI and park their replies.
async fn forward_questions(inner: Arc<Inner>, mut rx: mpsc::UnboundedReceiver<UserQuestionEvent>) {
    while let Some(event) = rx.recv().await {
        let request_id = uuid::Uuid::new_v4().to_string();
        inner
            .questions
            .lock()
            .insert(request_id.clone(), event.reply_tx);
        inner.events.emit(AgentEvent::UserInput {
            request_id,
            question: event.question,
            options: event.options.unwrap_or_default(),
        });
    }
}

async fn run_turn(
    inner: Arc<Inner>,
    prompt: String,
    cancel: CancellationToken,
    queue: CommandQueue,
) {
    // Work on a copy. The engine mutates it — appending the assistant turn,
    // tool calls, tool results, and synthetic results for anything abandoned
    // by a cancel — and the result is what gets written back.
    let mut messages = inner.history.lock().clone();
    // What the conversation weighed going in, for marking a summary the
    // engine may write this turn with the turns it replaced.
    let weight_before = history::turn_count(&messages) + 1;
    let mut prompt = Message::user(prompt);
    // Found again by its mark rather than by index: a compaction mid-turn
    // rewrites everything before it, and everything after the prompt is this
    // turn's — which is what lets a turn that produced nothing be told apart
    // from one that answered.
    let turn_mark = history::mark_turn(&mut prompt);
    messages.push(prompt);

    let config = inner.config.lock().clone();
    let mut query = inner.query.lock().clone();
    // Re-derived every turn: an instruction file edited between turns applies
    // to the next one. Locked on its own, after the two above, as elsewhere.
    {
        let options = inner.options.lock().clone();
        crate::config::refresh_session_rules(&mut query, &config, &options);
    }
    query.command_queue = Some(queue.clone());
    // A session with an active goal keeps going after each answer until the
    // model closes it with `GoalComplete` — the one tool set that has it.
    let goal_store = crate::goal::open_store();
    let goal_mode = goal_store
        .as_ref()
        .is_some_and(|store| crate::goal::is_active(store, &inner.id));
    let had_goal = goal_store
        .as_ref()
        .is_some_and(|store| crate::goal::snapshot(store, &inner.id).is_some());
    drop(goal_store);
    if goal_mode {
        query.continuation = claurst_query::ContinuationMode::Goal;
    }
    let tokens_before = inner.cost_tracker.total_tokens();
    // Read while `config` is still here: it moves into the tool context
    // below, and a turn that ends up saying nothing has to name the route it
    // tried.
    let route = Route {
        provider: config.selected_provider_id().to_owned(),
        model: query.model.clone(),
        api_base: config.resolve_anthropic_api_base(),
    };
    let tools = {
        let sets = inner.tools.lock();
        if goal_mode {
            sets.goal.clone()
        } else {
            sets.plain.clone()
        }
    };
    // `None` means "nobody here knows", and the meter then shows a token
    // count with no percentage rather than a percentage of the wrong number.
    // The engine's heuristic answers a fixed guess for every model it does
    // not know, which is no basis for a percentage.
    let context_window = crate::config::registry_context_window(&query.model, &route.provider);

    let client = inner.client.lock().clone();
    let handler: Arc<dyn claurst_core::PermissionHandler> =
        Arc::new(GuiPermissionHandler::new(inner.bridge.clone()));

    let (question_tx, question_rx) = mpsc::unbounded_channel::<UserQuestionEvent>();
    let questions = tokio::spawn(forward_questions(inner.clone(), question_rx));

    let tool_ctx = ToolContext {
        working_dir: inner.cwd.clone(),
        permission_mode: config.permission_mode.clone(),
        permission_handler: handler,
        cost_tracker: inner.cost_tracker.clone(),
        session_id: inner.id.clone(),
        file_history: inner.file_history.clone(),
        current_turn: inner.turn_counter.clone(),
        // The GUI can answer, so this is an interactive session even though
        // there is no terminal attached.
        non_interactive: false,
        mcp_manager: inner.mcp.lock().clone(),
        managed_agent_config: config.managed_agents.clone(),
        config,
        completion_notifier: None,
        // Unused: `GuiPermissionHandler` never returns `Ask`, which is the
        // only thing that routes a request into this queue.
        pending_permissions: None,
        permission_manager: Some(inner.manager.clone()),
        user_question_tx: Some(question_tx),
        cancel_token: cancel.clone(),
    };

    let (tx, rx) = mpsc::unbounded_channel::<QueryEvent>();
    let forwarder = tokio::spawn(forward_events(inner.clone(), rx, context_window));

    let outcome = claurst_query::run_query_loop(
        client.as_ref(),
        &mut messages,
        tools.as_slice(),
        &tool_ctx,
        &query,
        inner.cost_tracker.clone(),
        Some(tx),
        cancel,
        None,
    )
    .await;

    // Both forwarders end when their senders go: the loop dropped its event
    // sender on return, and the question sender lives in `tool_ctx`.
    drop(tool_ctx);
    let _ = forwarder.await;
    let _ = questions.await;

    // Write the transcript back *before* the turn is taken down. `prompt`
    // treats "no turn" as "idle" and clones the history to start the next
    // loop; if that could happen between these two steps the next turn would
    // start from a transcript missing this one, and then overwrite it.
    history::settle_compaction(weight_before, &mut messages);
    // A prompt a compaction folded away was followed by enough work to fill
    // the context, so the turn did not go quiet.
    let produced_output = match history::position_of(&messages, &turn_mark) {
        Some(index) => history::produced_visible_output(&messages, index + 1),
        None => true,
    };
    *inner.history.lock() = messages;

    // Now let the next prompt in. Whatever steering was still queued is
    // settled below, once the engine can no longer drain it.
    let leftover = {
        let mut guard = inner.turn.lock();
        guard
            .take()
            .map(|turn| turn.steers.lock().drain(..).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    inner.questions.lock().clear();

    // Only a turn the engine called *finished* counts as empty. A cancel
    // produces nothing either, and saying "the model ended without saying
    // anything" to someone who just pressed stop would be a lie.
    let ended_empty = !produced_output && matches!(outcome, QueryOutcome::EndTurn { .. });
    let cancelled = matches!(outcome, QueryOutcome::Cancelled);
    let (success, summary) = describe(outcome, produced_output, &route);
    if !success && let Some(reason) = summary.clone() {
        // The empty turn gets its own event so the desktop can say it in the
        // user's language; the sentence in `summary` is the fallback for any
        // client that does not.
        if ended_empty {
            inner.events.emit(AgentEvent::ProducedNothing {
                provider: route.provider.clone(),
                model: route.model.clone(),
                api_base: route.api_base.clone(),
            });
        } else {
            inner.events.emit(AgentEvent::Error(reason));
        }
    }
    inner.events.emit(AgentEvent::TurnFinished { success, summary });
    inner.events.emit(AgentEvent::HistoryCommitted(history::serialize(
        &inner.history.lock(),
    )));
    // A turn is the only thing that creates background work, so this is the
    // moment the panel needs a fresh level signal.
    inner
        .events
        .emit(AgentEvent::BackgroundWork(background::snapshot()));
    let goal_follow_up = settle_goal(&inner, goal_mode, had_goal, tokens_before, cancelled);

    // Steering messages the watcher had not yet accounted for. The queue
    // itself is the authority, not the watcher's 120ms poll: `drain` empties
    // it in one go, so an empty queue means every one of them reached the
    // conversation, and a turn that ends within a poll interval of the drain
    // must not report a delivered message as rejected.
    let delivered = queue.is_empty();
    for message in leftover {
        inner.events.emit(if delivered {
            AgentEvent::SteerAccepted { message }
        } else {
            AgentEvent::SteerRejected {
                message,
                reason: "the turn ended before the message could be delivered".to_string(),
            }
        });
    }

    // Last, so the next turn starts after this one has fully reported.
    if let Some(prompt) = goal_follow_up {
        AgentSession { inner }.prompt(prompt);
    }
}

/// The body of [`AgentSession::compact`].
async fn compact_now(inner: Arc<Inner>, instructions: Option<String>, cancel: CancellationToken) {
    use crate::events::CompactionPhase;

    let messages = inner.history.lock().clone();
    let weight_before = history::turn_count(&messages);
    let config = inner.config.lock().clone();
    let query = inner.query.lock().clone();
    let provider_id = config.selected_provider_id().to_owned();
    let tokens_before = claurst_query::estimate_context_tokens(&messages, None);
    let report = |phase, tokens_after| {
        inner.events.emit(AgentEvent::Compaction {
            phase,
            automatic: false,
            tokens_before,
            tokens_after,
        });
    };
    report(CompactionPhase::Started, None);

    let summarise = async {
        if provider_id == "anthropic" {
            let client = inner.client.lock().clone();
            claurst_query::compact_conversation_with(
                client.as_ref(),
                &messages,
                &query.model,
                instructions.as_deref(),
            )
            .await
        } else {
            match summary_provider(&config, &query, &provider_id) {
                Some(provider) => {
                    claurst_query::compact_conversation_via_provider(
                        provider.as_ref(),
                        &query.model,
                        &messages,
                        query.max_tokens,
                        instructions.as_deref(),
                    )
                    .await
                }
                None => Err(claurst_core::error::ClaudeError::Other(format!(
                    "no provider is configured for {provider_id}"
                ))),
            }
        }
    };
    let result = tokio::select! {
        _ = cancel.cancelled() => None,
        result = summarise => Some(result),
    };

    let (success, summary) = match result {
        None => {
            report(CompactionPhase::Failed, None);
            (false, Some("Stopped.".to_string()))
        }
        Some(Ok(mut compacted)) if compacted.len() < messages.len() => {
            history::settle_compaction(weight_before, &mut compacted);
            let tokens_after = claurst_query::estimate_context_tokens(&compacted, None);
            *inner.history.lock() = compacted;
            report(CompactionPhase::Finished, Some(tokens_after));
            inner.events.emit(AgentEvent::Usage {
                context_tokens: Some(tokens_after),
                context_window: crate::config::registry_context_window(&query.model, &provider_id),
            });
            (true, None)
        }
        // Everything fits the recent tail the summary would keep verbatim:
        // there is nothing old enough to fold away.
        Some(Ok(_)) => {
            report(CompactionPhase::Failed, None);
            (true, None)
        }
        Some(Err(error)) => {
            report(CompactionPhase::Failed, None);
            (false, Some(error.to_string()))
        }
    };

    let leftover = inner
        .turn
        .lock()
        .take()
        .map(|turn| turn.steers.lock().drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    inner.events.emit(AgentEvent::TurnFinished { success, summary });
    inner.events.emit(AgentEvent::HistoryCommitted(history::serialize(
        &inner.history.lock(),
    )));
    for message in leftover {
        inner.events.emit(AgentEvent::SteerRejected {
            message,
            reason: "the conversation was being compacted".to_string(),
        });
    }
}

/// The provider adapter a summary goes through on a route that is not the
/// Messages API — found the way the engine's loop finds it for a turn.
fn summary_provider(
    config: &Config,
    query: &QueryConfig,
    provider_id: &str,
) -> Option<Arc<dyn claurst_api::LlmProvider>> {
    if claurst_api::registry::resolve_provider_api_base(config, provider_id).is_some()
        && let Some(provider) = claurst_api::registry::provider_from_config(config, provider_id)
    {
        return Some(provider);
    }
    claurst_api::registry::runtime_provider_for(provider_id).or_else(|| {
        query
            .provider_registry
            .as_ref()?
            .get(&claurst_core::provider_id::ProviderId::new(provider_id))
            .cloned()
    })
}

/// After a turn: charge its tokens to the goal, report where the goal
/// stands, and say whether to keep going.
///
/// Stopping a turn pauses an active goal — otherwise the next message would
/// quietly pick the pursuit back up. A goal set while an ordinary turn ran
/// was not pursued by it, so it starts now; one the turn already pursued was
/// kept going by the engine itself until it stopped.
fn settle_goal(
    inner: &Arc<Inner>,
    goal_mode: bool,
    had_goal: bool,
    tokens_before: u64,
    cancelled: bool,
) -> Option<String> {
    let store = crate::goal::open_store()?;
    let goal = store.get_goal(&inner.id);
    if let Some(goal) = &goal {
        let spent = inner.cost_tracker.total_tokens().saturating_sub(tokens_before);
        if spent > 0 {
            let _ = store.add_tokens(&inner.id, spent);
        }
        if cancelled && goal.status == claurst_core::GoalStatus::Active {
            let _ = store.set_status(&inner.id, claurst_core::GoalStatus::Paused);
        }
    }
    if goal.is_some() || had_goal {
        inner
            .events
            .emit(AgentEvent::GoalUpdated(crate::goal::snapshot(&store, &inner.id)));
    }
    if goal_mode || cancelled {
        return None;
    }
    store
        .get_active_goal(&inner.id)
        .map(|goal| claurst_core::goal_continuation_message(&goal))
}

async fn forward_events(
    inner: Arc<Inner>,
    mut rx: mpsc::UnboundedReceiver<QueryEvent>,
    // `None` when no source knows this model's window — the meter then omits
    // the percentage rather than showing one against a guess.
    context_window: Option<u64>,
) {
    let mut decoder = StreamDecoder::new(context_window);
    while let Some(event) = rx.recv().await {
        for mut translated in decoder.push(event) {
            if let AgentEvent::TokenUsage { session, .. } = &mut translated {
                *session = crate::events::TokenCounts::from_tracker(&inner.cost_tracker);
            }
            if let AgentEvent::PlanModeChanged(plan) = &translated {
                // The model entered or left plan mode mid-turn. Move the
                // engine's permission policy now — waiting for the next
                // turn would let a just-approved plan sit unexecutable
                // behind a manager that still says Plan.
                let (permission_mode, options) = {
                    let mut options = inner.options.lock();
                    options.plan_mode = *plan;
                    (options.access_mode.permission_mode(*plan), options.clone())
                };
                let config = {
                    let mut config = inner.config.lock();
                    config.permission_mode = permission_mode.clone();
                    config.clone()
                };
                // The plan rule rides in the system prompt of every later
                // turn; left as it was, the turn after an approved plan
                // would be told it is still planning while its tools say
                // otherwise. Locked last and alone, matching `run_turn`.
                crate::config::refresh_session_rules(
                    &mut inner.query.lock(),
                    &config,
                    &options,
                );
                if let Ok(mut manager) = inner.manager.lock() {
                    // Same reasoning as apply_options: the manager caches the
                    // mode it evaluates against, so rebuild it rather than
                    // patching the field.
                    let settings = load_settings().unwrap_or_else(|error| {
                        tracing::warn!(%error, "agent: settings unreadable; keeping this session's rules");
                        inner.settings.lock().clone()
                    });
                    *manager = PermissionManager::new(permission_mode, &settings);
                }
            }
            let closed_goal = matches!(
                &translated,
                AgentEvent::ToolFinished { name, failed: false, .. }
                    if name == GOAL_COMPLETE_TOOL
            );
            inner.events.emit(translated);
            // The model just marked the goal complete; the chip should say
            // so now, not when the run winds down.
            if closed_goal && let Some(store) = crate::goal::open_store() {
                inner
                    .events
                    .emit(AgentEvent::GoalUpdated(crate::goal::snapshot(&store, &inner.id)));
            }
        }
    }
}

/// What to tell the transcript about how the turn ended.
///
/// What a turn was pointed at, for the message an empty turn has to write.
struct Route {
    provider: String,
    model: String,
    api_base: String,
}

/// `MaxTokens` counts as a success: the model produced an answer and the
/// engine's own recovery already ran. Everything else that is not `EndTurn`
/// carries a reason the user can act on — reporting a turn that produced
/// nothing as a success is the failure mode this exists to avoid.
///
/// `produced_output` is what closes the last hole in that: a clean `EndTurn`
/// that added no assistant text is not a success either, whatever the stop
/// reason said. It names the route it tried, because the thing that went
/// wrong is upstream of here and that is the only handle the user has on it.
fn describe(
    outcome: QueryOutcome,
    produced_output: bool,
    route: &Route,
) -> (bool, Option<String>) {
    match outcome {
        QueryOutcome::EndTurn { .. } if !produced_output => (
            false,
            Some(format!(
                "The model ended the turn without saying anything. \
                 Route: {} · {} · {}",
                route.provider, route.model, route.api_base
            )),
        ),
        QueryOutcome::EndTurn { .. } => (true, None),
        QueryOutcome::MaxTokens { .. } => (
            true,
            Some("The model reached its output limit for this turn.".to_string()),
        ),
        QueryOutcome::Cancelled => (false, Some("Stopped.".to_string())),
        QueryOutcome::BudgetExceeded { cost_usd, limit_usd } => (
            false,
            Some(format!(
                "This session has spent ${cost_usd:.2}, past its ${limit_usd:.2} cap."
            )),
        ),
        QueryOutcome::Error(error) => (false, Some(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route() -> Route {
        Route {
            provider: "anthropic".into(),
            model: "claude-sonnet-5".into(),
            api_base: "https://gateway.example.org".into(),
        }
    }

    #[test]
    fn a_cancelled_turn_is_never_reported_as_a_success() {
        let (success, summary) = describe(QueryOutcome::Cancelled, true, &route());
        assert!(!success);
        assert!(summary.is_some());
    }

    /// The shape a swallowed upstream failure takes: the engine reports a
    /// clean end_turn, the transcript gained nothing. Reporting that as a
    /// success is what left the user reading "turn completed" and no answer.
    #[test]
    fn a_turn_that_said_nothing_is_a_failure_that_names_its_route() {
        let (success, summary) = describe(
            QueryOutcome::EndTurn {
                message: Message::assistant_blocks(Vec::new()),
                usage: Default::default(),
            },
            false,
            &route(),
        );
        assert!(!success);
        let summary = summary.expect("an empty turn must explain itself");
        assert!(summary.contains("anthropic"), "{summary}");
        assert!(summary.contains("claude-sonnet-5"), "{summary}");
        assert!(summary.contains("gateway.example.org"), "{summary}");
    }

    /// A cancel produces nothing either. It must keep reading as a stop,
    /// not as the model having gone quiet.
    #[test]
    fn a_cancel_is_not_reported_as_an_empty_turn() {
        let (success, summary) = describe(QueryOutcome::Cancelled, false, &route());
        assert!(!success);
        let summary = summary.expect("a cancel says so");
        assert!(summary.contains("Stopped"), "{summary}");
        assert!(!summary.contains("without saying anything"), "{summary}");
    }

    #[test]
    fn a_turn_that_answered_stays_a_success() {
        let (success, summary) = describe(
            QueryOutcome::EndTurn {
                message: Message::assistant_blocks(Vec::new()),
                usage: Default::default(),
            },
            true,
            &route(),
        );
        assert!(success);
        assert!(summary.is_none());
    }

    #[test]
    fn a_spend_cap_names_both_numbers() {
        let (success, summary) = describe(
            QueryOutcome::BudgetExceeded {
                cost_usd: 12.5,
                limit_usd: 10.0,
            },
            false,
            &route(),
        );
        assert!(!success);
        let summary = summary.unwrap();
        assert!(summary.contains("$12.50"), "{summary}");
        assert!(summary.contains("$10.00"), "{summary}");
    }

    #[test]
    fn hitting_the_output_limit_still_counts_as_an_answer() {
        let (success, summary) = describe(
            QueryOutcome::MaxTokens {
                partial_message: Message::assistant("half an answer"),
                usage: Default::default(),
            },
            true,
            &route(),
        );
        assert!(success);
        assert!(summary.is_some(), "the user should still be told why it stopped");
    }

    #[test]
    fn the_builtin_tool_set_includes_the_sub_agent_tool() {
        let tools = builtin_tools(&[], None, false);
        assert!(tools.iter().any(|tool| tool.name() == "Agent"));
        assert!(tools.iter().any(|tool| tool.name() == "Read"));
    }

    /// Tools whose runner is not in this process would report success at
    /// something that never happens; GoalComplete would close a goal that
    /// was never set.
    #[test]
    fn tools_that_cannot_work_here_are_never_offered() {
        let plain = builtin_tools(&[], None, false);
        for name in UNAVAILABLE_TOOLS.iter().chain([&GOAL_COMPLETE_TOOL]) {
            assert!(!plain.iter().any(|tool| tool.name() == *name), "{name}");
        }
        let goal = builtin_tools(&[], None, true);
        assert!(goal.iter().any(|tool| tool.name() == GOAL_COMPLETE_TOOL));
        assert!(!goal.iter().any(|tool| tool.name() == "TeamCreate"));
    }

    #[test]
    fn a_disallowed_tool_is_not_offered_at_all() {
        let tools = builtin_tools(&["WebSearch".to_string()], None, false);
        assert!(!tools.iter().any(|tool| tool.name() == "WebSearch"));
        assert!(tools.iter().any(|tool| tool.name() == "Read"));
    }
}
