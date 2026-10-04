//! One team member: a persistent sub-agent of the captain.
//!
//! Unlike an `Agent` call's child, a member keeps its conversation across
//! turns — the captain's guidance, its earlier tasks, what it read — and on
//! disk (`<team>/sessions/<member>.json`), so a restart resumes it. It runs
//! on its own route (any gateway model), with its own permission scope
//! (never plan mode), and reports what it does as a sub-agent record of the
//! captain's session.
//!
//! Input arrives two ways, as in the reference: a task assignment is a turn
//! of its own after the current one (`Queue`); a message joins the running
//! turn at its next step, or starts one (`Steer`).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use agent_teams::runtime::{DeliveryMode, MemberActivity, MemberRoute};
use agent_teams::types::{TeamMember, TeamState};
use claurst_api::AnthropicClient;
use claurst_core::PermissionManager;
use claurst_core::config::Config;
use claurst_core::types::Message;
use claurst_query::{
    CommandPriority, CommandQueue, QueryConfig, QueryEvent, QueryOutcome, QueuedCommand,
};
use claurst_tools::{Tool, ToolContext};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::background::BackgroundEntry;
use crate::config::{AgentStartOptions, build_config, build_query_config, load_settings};
use crate::events::{AgentEvent, StreamDecoder, SubagentEvent, SubagentStatus};
use crate::permission::{GuiPermissionHandler, MemberScope};
use crate::subagent::{CHILD_EXCLUDED_TOOLS, SubagentHost, SubagentTool, TurnScope, join_prompts};

use super::{CaptainShared, TeamHost};

/// What a member runs a turn with: its route's config and client.
pub(crate) struct MemberEngine {
    pub options: AgentStartOptions,
    pub config: Config,
    pub query: QueryConfig,
    pub client: Arc<AnthropicClient>,
}

/// Build the engine for a member route: the captain's options with the
/// member's model, effort and identity, never in plan mode, never driving
/// the desktop or the in-app browser. Resolves keys, so it blocks — callers
/// are on blocking threads.
pub(crate) fn build_engine(
    captain: &AgentStartOptions,
    route: &MemberRoute,
    member_id: Option<&str>,
) -> anyhow::Result<MemberEngine> {
    let mut options = captain.clone();
    options.platform = Some(route.provider.clone());
    options.model = Some(route.model.clone());
    options.reasoning_effort = route.reasoning_effort.clone();
    // The member's model decides its own format, as a model picked in the
    // composer would.
    options.wire_format = None;
    options.plan_mode = false;
    options.history = Vec::new();
    options.computer_use = None;
    options.browser_tools = false;
    options.session_id = member_id.map(str::to_owned);
    let config = build_config(&options)?;
    let mut query = build_query_config(&config, &options);
    let (client, registry) = crate::session::build_clients(&config, options.platform.as_deref())?;
    query.provider_registry = Some(registry);
    Ok(MemberEngine {
        options,
        config,
        query,
        client,
    })
}

/// The route a member record carries, its fallback first when that is the
/// one in use.
pub(crate) fn route_of(member: &TeamMember, captain: &AgentStartOptions) -> MemberRoute {
    let fallback = member.fallback_active == Some(true);
    let provider = if fallback {
        member.active_provider.clone()
    } else {
        None
    }
    .or_else(|| member.provider.clone())
    .or_else(|| captain.platform.clone())
    .unwrap_or_else(|| "anthropic".to_owned());
    let model = if fallback {
        member.active_model.clone()
    } else {
        None
    }
    .or_else(|| member.model.clone())
    .or_else(|| captain.model.clone())
    .unwrap_or_default();
    MemberRoute {
        provider,
        model,
        // An effort id belongs to the original model.
        reasoning_effort: if fallback {
            None
        } else {
            member.reasoning_effort.clone()
        },
        fallback: member.fallback.clone(),
    }
}

#[derive(Default)]
struct TurnState {
    running: bool,
    cancel: Option<CancellationToken>,
    queue: Option<CommandQueue>,
    /// Assignments waiting for the running turn to end.
    queued: VecDeque<String>,
    /// Messages injected into the running turn, for re-delivery when it
    /// ends before taking them.
    steered: Vec<String>,
}

pub(crate) struct MemberRuntime {
    id: String,
    team_id: String,
    name: String,
    title: String,
    host: Weak<TeamHost>,
    persona: String,
    model: String,
    max_depth: u32,
    engine: MemberEngine,
    history: Mutex<Vec<Message>>,
    history_path: PathBuf,
    state: Mutex<TurnState>,
    turn_counter: Arc<AtomicUsize>,
    subagents: Arc<SubagentHost>,
    started_at_ms: AtomicU64,
}

fn now_ms() -> u64 {
    agent_teams::types::now_ms()
}

impl MemberRuntime {
    /// A member's runtime from its record, its conversation reloaded when it
    /// has one. Blocking.
    pub fn create(
        host: &Arc<TeamHost>,
        team: &TeamState,
        member: &TeamMember,
        id: &str,
    ) -> Result<Arc<Self>, String> {
        let port = host.port().ok_or("the captain session is gone")?;
        let captain = port.options().ok_or("the captain session is gone")?;
        let route = route_of(member, &captain);
        let engine =
            build_engine(&captain, &route, Some(id)).map_err(|error| format!("{error:#}"))?;
        let config = host.config();
        let state_dir = config.state_dir().to_owned();
        let persona = agent_teams::prompts::member_persona(
            team,
            member,
            &state_dir,
            config.execution_prompt.as_deref(),
        );
        let root = agent_teams::store::StateRoot::new(host.cwd(), &state_dir);
        let history_path = root.member_history_file(&team.id, &member.name);
        let history = std::fs::read(&history_path)
            .map(|bytes| crate::history::deserialize(&bytes))
            .unwrap_or_default();
        let title = match member
            .role
            .as_deref()
            .map(str::trim)
            .filter(|role| !role.is_empty())
        {
            Some(role) => format!("{} · {role}", member.name),
            None => member.name.clone(),
        };
        Ok(Arc::new(Self {
            id: id.to_owned(),
            team_id: team.id.clone(),
            name: member.name.clone(),
            title,
            host: Arc::downgrade(host),
            persona,
            model: route.model,
            max_depth: config.member_max_depth(),
            engine,
            history: Mutex::new(history),
            history_path,
            state: Mutex::new(TurnState::default()),
            turn_counter: Arc::new(AtomicUsize::new(0)),
            subagents: SubagentHost::new(host.events().clone()),
            started_at_ms: AtomicU64::new(now_ms()),
        }))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn activity(&self) -> MemberActivity {
        if self.state.lock().running {
            MemberActivity::Running
        } else {
            MemberActivity::Idle
        }
    }

    pub fn is_running(&self) -> bool {
        self.state.lock().running
    }

    pub fn live_entry(&self) -> Option<BackgroundEntry> {
        self.is_running().then(|| {
            super::live_entry(
                &self.id,
                &self.title,
                self.started_at_ms.load(Ordering::Acquire),
            )
        })
    }

    /// Hand the member a text. `false` when the session is gone.
    pub fn deliver(self: &Arc<Self>, text: String, mode: DeliveryMode) -> bool {
        let Some(host) = self.host.upgrade() else {
            return false;
        };
        if host.shutdown_token().is_cancelled() {
            return false;
        }
        let mut state = self.state.lock();
        if state.running {
            match (mode, &state.queue) {
                (DeliveryMode::Steer, Some(queue)) => {
                    queue.push(
                        QueuedCommand::InjectUserMessage(text.clone()),
                        CommandPriority::Normal,
                    );
                    state.steered.push(text);
                }
                _ => state.queued.push_back(text),
            }
            return true;
        }
        state.running = true;
        drop(state);
        let Ok(rt) = crate::runtime::shared() else {
            self.state.lock().running = false;
            return false;
        };
        let member = self.clone();
        rt.spawn(async move { member.run(text).await });
        true
    }

    /// Stop the running turn and drop what was waiting. The open attempt
    /// stays with the member, parked.
    pub fn interrupt(&self) {
        let cancel = {
            let mut state = self.state.lock();
            state.queued.clear();
            state.steered.clear();
            state.cancel.clone()
        };
        if let Some(host) = self.host.upgrade()
            && let Some(shared) = host.port().and_then(|port| port.shared())
        {
            shared.bridge.release_member(&self.name);
        }
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
    }

    /// [`Self::interrupt`], then wait (bounded) for the turn to end.
    pub fn drain_blocking(&self) -> Result<(), String> {
        self.interrupt();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.is_running() {
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "member \"{}\" did not stop within 10 seconds",
                    self.name
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        tokio::task::spawn_blocking(f).await.ok()
    }

    /// Turns until nothing waits, then the idle edge (which may hand the
    /// member its next task).
    async fn run(self: Arc<Self>, first: String) {
        let Some(runtime) = self.host.upgrade().and_then(|host| host.runtime()) else {
            self.state.lock().running = false;
            return;
        };
        {
            let (runtime, team, id) = (runtime.clone(), self.team_id.clone(), self.id.clone());
            Self::blocking(move || runtime.member_status_edge(&team, &id, true)).await;
        }
        let mut next = Some(first);
        while let Some(prompt) = next.take() {
            let admitted = {
                let (runtime, team, id) = (runtime.clone(), self.team_id.clone(), self.id.clone());
                Self::blocking(move || runtime.admit_member_step(&team, &id))
                    .await
                    .unwrap_or(false)
            };
            if !admitted {
                break;
            }
            let cancel = match self.host.upgrade() {
                Some(host) => host.shutdown_token().child_token(),
                None => break,
            };
            let queue = CommandQueue::new();
            {
                let mut state = self.state.lock();
                state.cancel = Some(cancel.clone());
                state.queue = Some(queue.clone());
            }
            self.started_at_ms.store(now_ms(), Ordering::Release);
            self.emit(SubagentEvent::Started {
                description: self.title.clone(),
                prompt: prompt.clone(),
                model: self.model.clone(),
                background: true,
                started_at_ms: now_ms(),
            });
            let started = std::time::Instant::now();
            let outcome = self.turn(prompt, cancel.clone(), queue.clone()).await;
            self.report(&outcome, started);
            if let Some(failure) = failure_of(&outcome) {
                let (runtime, team, id) = (runtime.clone(), self.team_id.clone(), self.id.clone());
                Self::blocking(move || {
                    let observed = runtime.observe_member_attempt(&team, &id);
                    runtime.fail_member_open_attempt(&team, &id, observed.as_ref(), &failure)
                })
                .await;
            }
            // What the turn never took goes again, ahead of the next task.
            let leftover: Vec<String> = queue
                .drain()
                .into_iter()
                .filter_map(|command| match command {
                    QueuedCommand::InjectUserMessage(text) => Some(text),
                    _ => None,
                })
                .collect();
            let mut state = self.state.lock();
            state.cancel = None;
            state.queue = None;
            let steered = std::mem::take(&mut state.steered);
            if !cancel.is_cancelled() {
                for text in leftover.into_iter().rev() {
                    if steered.contains(&text) {
                        state.queued.push_front(text);
                    }
                }
            }
            next = state.queued.pop_front();
            if next.is_none() {
                state.running = false;
            }
        }
        {
            let mut state = self.state.lock();
            state.running = false;
            state.queued.clear();
        }
        if let Some(host) = self.host.upgrade() {
            host.refresh_snapshot();
        }
        let (team, id) = (self.team_id.clone(), self.id.clone());
        Self::blocking(move || runtime.member_status_edge(&team, &id, false)).await;
    }

    fn emit(&self, event: SubagentEvent) {
        if let Some(host) = self.host.upgrade() {
            host.events().emit(AgentEvent::Subagent {
                parent_tool_id: self.id.clone(),
                event,
            });
        }
    }

    fn report(&self, outcome: &QueryOutcome, started: std::time::Instant) {
        let status = match outcome {
            QueryOutcome::EndTurn { .. } | QueryOutcome::MaxTokens { .. } => {
                SubagentStatus::Completed
            }
            QueryOutcome::Cancelled => SubagentStatus::Stopped,
            QueryOutcome::Error(_) | QueryOutcome::BudgetExceeded { .. } => SubagentStatus::Failed,
        };
        let (summary, result) = match outcome {
            QueryOutcome::EndTurn { message, .. } => (None, Some(message.get_all_text())),
            QueryOutcome::MaxTokens {
                partial_message, ..
            } => (None, Some(partial_message.get_all_text())),
            QueryOutcome::Cancelled => (None, None),
            other => (failure_of(other), None),
        };
        self.emit(SubagentEvent::Finished {
            status,
            summary,
            result: result.filter(|text| !text.trim().is_empty()),
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
    }

    fn tools(self: &Arc<Self>, host: &Arc<TeamHost>, shared: &CaptainShared) -> Vec<Box<dyn Tool>> {
        let mut tools =
            crate::session::engine_tools(&self.engine.config.disallowed_tools, shared.mcp.as_ref());
        // A member works unattended: no questions to the user, no session
        // mode switches, and no desktop control — two loops driving one
        // screen would fight over it. Drawing pictures stays.
        tools.retain(|tool| {
            !CHILD_EXCLUDED_TOOLS.contains(&tool.name())
                && !matches!(tool.name(), "waku_js_repl_js" | "waku_js_repl_js_reset")
        });
        tools.extend(super::tools::member_tools(host));
        if self.max_depth > 0 {
            tools.push(Box::new(SubagentTool::new(self.subagents.clone())));
        }
        tools
    }

    async fn turn(
        self: &Arc<Self>,
        prompt: String,
        cancel: CancellationToken,
        queue: CommandQueue,
    ) -> QueryOutcome {
        let failed = |text: &str| {
            QueryOutcome::Error(claurst_core::error::ClaudeError::Other(text.to_owned()))
        };
        let Some(host) = self.host.upgrade() else {
            return failed("the captain session is gone");
        };
        let Some(shared) = host.port().and_then(|port| port.shared()) else {
            return failed("the captain session is gone");
        };
        let engine = &self.engine;
        let mut messages = self.history.lock().clone();
        messages.push(Message::user(prompt));

        let mut query = engine.query.clone();
        crate::config::refresh_session_rules(&mut query, &engine.config, &engine.options);
        query.system_prompt = join_prompts(query.system_prompt.take(), Some(self.persona.clone()));
        query.append_system_prompt = join_prompts(
            query.append_system_prompt.take(),
            Some(agent_teams::prompts::TEAM_MEMBER_PROMPT.to_owned()),
        );
        query.continuation = claurst_query::ContinuationMode::Default;
        query.agent_name = None;
        query.agent_definition = None;
        query.managed_agents = None;
        let child_query = query.clone();
        query.command_queue = Some(queue);
        let tools = self.tools(&host, &shared);
        query.enabled_tools = Some(tools.iter().map(|tool| tool.name().to_owned()).collect());

        let settings = load_settings().unwrap_or_default();
        let manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
            engine.config.permission_mode.clone(),
            &settings,
        )));
        let handler: Arc<dyn claurst_core::PermissionHandler> =
            Arc::new(GuiPermissionHandler::for_member(
                shared.bridge.clone(),
                MemberScope {
                    name: self.name.clone(),
                    manager: manager.clone(),
                },
            ));
        if self.max_depth > 0 {
            self.subagents.begin_turn(TurnScope {
                client: engine.client.clone(),
                query: child_query,
                options: engine.options.clone(),
            });
        }
        let tool_ctx = ToolContext {
            working_dir: host.cwd().clone(),
            permission_mode: engine.config.permission_mode.clone(),
            permission_handler: handler,
            cost_tracker: shared.cost_tracker.clone(),
            session_id: self.id.clone(),
            file_history: shared.file_history.clone(),
            current_turn: self.turn_counter.clone(),
            non_interactive: false,
            mcp_manager: shared.mcp.clone(),
            managed_agent_config: engine.config.managed_agents.clone(),
            config: engine.config.clone(),
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: Some(manager),
            user_question_tx: None,
            cancel_token: cancel.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel::<QueryEvent>();
        let forwarder = tokio::spawn(forward(self.clone(), rx));
        let outcome = claurst_query::run_query_loop(
            engine.client.as_ref(),
            &mut messages,
            &tools,
            &tool_ctx,
            &query,
            shared.cost_tracker.clone(),
            Some(tx),
            cancel,
            None,
        )
        .await;
        drop(tool_ctx);
        let _ = forwarder.await;
        if self.max_depth > 0 {
            self.subagents.end_turn();
        }
        let bytes = crate::history::serialize(&messages);
        *self.history.lock() = messages;
        if let Err(error) = agent_teams::store::atomic_write_text(
            &self.history_path,
            &String::from_utf8_lossy(&bytes),
        ) {
            tracing::warn!(%error, member = %self.name, "agent-teams: member conversation not saved");
        }
        outcome
    }
}

/// The failure a turn ended with, as the team records it.
fn failure_of(outcome: &QueryOutcome) -> Option<String> {
    match outcome {
        QueryOutcome::Error(error) => Some(error.to_string()),
        QueryOutcome::BudgetExceeded {
            cost_usd,
            limit_usd,
        } => Some(format!(
            "the session's spend cap was reached (${cost_usd:.2} of ${limit_usd:.2})"
        )),
        _ => None,
    }
}

/// Carry what the member does to the captain's session as its sub-agent
/// record, text batched as it arrives.
async fn forward(member: Arc<MemberRuntime>, mut rx: mpsc::UnboundedReceiver<QueryEvent>) {
    let mut decoder = StreamDecoder::new(None);
    while let Some(first) = rx.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        let mut text = String::new();
        for event in batch {
            for translated in decoder.push(event) {
                match crate::subagent::child_event(translated) {
                    Some(SubagentEvent::Text(delta)) => text.push_str(&delta),
                    Some(other) => {
                        if !text.is_empty() {
                            member.emit(SubagentEvent::Text(std::mem::take(&mut text)));
                        }
                        member.emit(other);
                    }
                    None => {}
                }
            }
        }
        if !text.is_empty() {
            member.emit(SubagentEvent::Text(text));
        }
    }
}
