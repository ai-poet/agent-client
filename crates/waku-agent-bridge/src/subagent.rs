//! The `Agent` tool, served by the bridge rather than the engine.
//!
//! The engine ships its own (`claurst_query::AgentTool`), and it works — but
//! it runs the sub-agent with no event channel, so all anyone sees of it is
//! one "Agent" row and, much later, its answer. It also builds everything
//! afresh for the child: a client with the plain Anthropic key (not the
//! gateway key the session routes with), a model table parsed again from the
//! bundle, and `all_tools()` — no MCP, no Computer Use, none of the user's
//! switched-off tools honoured, none of the session's rules in the prompt. A
//! background child kept the parent's question channel alive, which held the
//! turn open until the child finished, and it was filed in the process-wide
//! task registry where every session's panel listed it.
//!
//! This one runs the same engine loop (`claurst_query::run_query_loop`, which
//! is public) with a real event channel, the parent's client, registries and
//! tool set, and forwards what the child does as [`AgentEvent::Subagent`] —
//! which the desktop shows as the sub-agent's own live record. The input
//! schema is the engine's, minus the two options this product cannot honour
//! (see [`SubagentTool::input_schema`]).
//!
//! # Finding the call a run belongs to
//!
//! `Tool::execute` is not told the id of the call it serves, and the record
//! has to hang off that call's row. The engine does announce every call —
//! `QueryEvent::ToolStart { tool_id, input_json }` — *before* running it, and
//! the session's event forwarder hands each `Agent` announcement to
//! [`SubagentHost::announce`]. `execute` then [`claim`](SubagentHost::claim)s
//! the pending announcement whose description and prompt match its own input.
//! The forwarder runs on its own task, so a claim may arrive first and waits
//! for the announcement; one that never comes (a hook rewrote the input) gives
//! up after a few seconds and runs under a made-up id, unlinked but visible.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use claurst_api::AnthropicClient;
use claurst_core::config::Config;
use claurst_core::types::Message;
use claurst_query::{QueryConfig, QueryEvent, QueryOutcome};
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::AgentStartOptions;
use crate::events::{AgentEvent, EventSink, StreamDecoder, SubagentEvent, SubagentStatus};

/// The tool's name, the engine's own: prompts and permission rules written
/// for the engine's tool keep applying.
pub(crate) const AGENT_TOOL_NAME: &str = claurst_core::constants::TOOL_NAME_AGENT;

/// Tools a sub-agent is not given.
///
/// `Agent` itself, so delegation does not recurse. `AskUserQuestion`: the
/// child works unattended, and a question from it would reach the user with
/// no sign of who is asking. The plan-mode and worktree switches and
/// `GoalComplete` change the *session's* state, which is not the child's to
/// change.
pub(crate) const CHILD_EXCLUDED_TOOLS: [&str; 7] = [
    AGENT_TOOL_NAME,
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "GoalComplete",
    "EnterWorktree",
    "ExitWorktree",
];

/// How long `execute` waits for the announcement of its own call.
const CLAIM_TIMEOUT: Duration = Duration::from_secs(3);

/// What a sub-agent is told about its situation, after the session's own
/// rules.
const SUBAGENT_RULE: &str = "You are a sub-agent: another agent handed you the task below and \
is waiting for your answer. Work on it on your own with the tools you have; you cannot ask the \
user anything. When you are done, reply with a complete, self-contained report of what you \
found or did — that reply is all the other agent will see of your work.";

/// Added while the session is in plan mode. The permission layer refuses
/// changes anyway; saying so up front saves the child from trying.
const SUBAGENT_PLAN_RULE: &str = "The session is in plan mode: research and report only. Do not \
edit files or run commands that change anything.";

/// What a turn hands the tool: what it needs to run a child the way the
/// parent runs.
pub(crate) struct TurnScope {
    pub client: Arc<AnthropicClient>,
    /// The parent's config for this turn, before its steering queue is
    /// attached.
    pub query: QueryConfig,
    pub options: AgentStartOptions,
}

/// An announced `Agent` call nobody has claimed yet.
#[derive(Clone, Debug)]
struct PendingCall {
    tool_id: String,
    description: String,
    prompt: String,
}

/// A child that is still running.
struct ChildHandle {
    cancel: CancellationToken,
    background: bool,
    stopped_by_user: Arc<AtomicBool>,
}

/// The session's side of every sub-agent it starts.
pub(crate) struct SubagentHost {
    events: EventSink,
    pending: Mutex<VecDeque<PendingCall>>,
    announced: Notify,
    scope: Mutex<Option<Arc<TurnScope>>>,
    children: Mutex<HashMap<String, ChildHandle>>,
    /// Every background child this session started, finished or not: the
    /// ones whose entries in the engine's process-wide registry are this
    /// session's to show.
    background: Mutex<HashSet<String>>,
}

impl SubagentHost {
    pub fn new(events: EventSink) -> Arc<Self> {
        Arc::new(Self {
            events,
            pending: Mutex::new(VecDeque::new()),
            announced: Notify::new(),
            scope: Mutex::new(None),
            children: Mutex::new(HashMap::new()),
            background: Mutex::new(HashSet::new()),
        })
    }

    pub fn begin_turn(&self, scope: TurnScope) {
        *self.scope.lock() = Some(Arc::new(scope));
    }

    /// A call announced and never run — refused, or cut off by a cancel —
    /// must not be claimed by next turn's call with the same text.
    pub fn end_turn(&self) {
        *self.scope.lock() = None;
        self.pending.lock().clear();
    }

    /// The engine is about to run `Agent` call `tool_id` with `input`.
    pub fn announce(&self, tool_id: &str, input: &Value) {
        let text = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        self.pending.lock().push_back(PendingCall {
            tool_id: tool_id.to_owned(),
            description: text("description").trim().to_owned(),
            prompt: text("prompt"),
        });
        self.announced.notify_waiters();
    }

    /// Call `tool_id` finished; if it never ran, it can no longer be claimed.
    pub fn forget(&self, tool_id: &str) {
        self.pending.lock().retain(|call| call.tool_id != tool_id);
    }

    /// The id of the announced call a run with this input serves. Waits up
    /// to `within` for the announcement.
    async fn claim(&self, description: &str, prompt: &str, within: Duration) -> Option<String> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            // Created before looking, so an announcement landing between the
            // look and the wait still wakes this.
            let announced = self.announced.notified();
            if let Some(id) = self.take_match(description, prompt) {
                return Some(id);
            }
            if tokio::time::timeout_at(deadline, announced).await.is_err() {
                return self.take_match(description, prompt);
            }
        }
    }

    /// The first pending call with this exact text; failing that, the only
    /// pending call, if there is exactly one.
    fn take_match(&self, description: &str, prompt: &str) -> Option<String> {
        let mut pending = self.pending.lock();
        let description = description.trim();
        let index = pending
            .iter()
            .position(|call| call.description == description && call.prompt == prompt)
            .or_else(|| (pending.len() == 1).then_some(0))?;
        pending.remove(index).map(|call| call.tool_id)
    }

    fn scope(&self) -> Option<Arc<TurnScope>> {
        self.scope.lock().clone()
    }

    /// Stop one child the user pointed at. `false` when no running child has
    /// this id — the caller then tries the engine's registry.
    pub fn stop(&self, id: &str) -> bool {
        let children = self.children.lock();
        let Some(child) = children.get(id) else {
            return false;
        };
        child.stopped_by_user.store(true, Ordering::Release);
        child.cancel.cancel();
        if child.background {
            claurst_core::tasks::global_registry().cancel(id);
        }
        true
    }

    /// The background children this session started.
    pub fn owned_background(&self) -> HashSet<String> {
        self.background.lock().clone()
    }

    /// The session is going away: so are its children.
    pub fn cancel_all(&self) {
        for child in self.children.lock().values() {
            child.cancel.cancel();
        }
    }

    fn emit(&self, parent_tool_id: &str, event: SubagentEvent) {
        self.events.emit(AgentEvent::Subagent {
            parent_tool_id: parent_tool_id.to_owned(),
            event,
        });
    }
}

/// The input, read by hand: the fields are few, and a typo in one is better
/// answered with the field's name than with a serde path.
struct AgentInput {
    description: String,
    prompt: String,
    tools: Option<Vec<String>>,
    system_prompt: Option<String>,
    max_turns: Option<u32>,
    run_in_background: bool,
}

impl AgentInput {
    fn parse(input: &Value) -> Result<Self, String> {
        let required = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("Invalid input: `{key}` is required"))
        };
        Ok(Self {
            description: required("description")?.trim().to_owned(),
            prompt: required("prompt")?,
            tools: input.get("tools").and_then(Value::as_array).map(|tools| {
                tools
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            }),
            system_prompt: input
                .get("system_prompt")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .map(str::to_owned),
            max_turns: input
                .get("max_turns")
                .and_then(Value::as_u64)
                .filter(|turns| *turns > 0)
                .map(|turns| u32::try_from(turns).unwrap_or(u32::MAX)),
            run_in_background: input
                .get("run_in_background")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

pub(crate) struct SubagentTool {
    host: Arc<SubagentHost>,
}

impl SubagentTool {
    pub fn new(host: Arc<SubagentHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &str {
        AGENT_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Launch a sub-agent to handle a complex, multi-step task on its own. It runs its own \
         loop with the same tools you have (except this one and the ones that talk to the user \
         or change the session's mode) and returns its final report. Use it to delegate \
         research or self-contained work, or to run several independent tasks in parallel by \
         calling it more than once in one message. Give it a complete prompt: it sees nothing \
         of this conversation."
    }

    fn permission_level(&self) -> PermissionLevel {
        // The child's own tool calls are gated one by one, as the parent's are.
        PermissionLevel::None
    }

    /// The engine's schema without `model` and `isolation`. A session's
    /// keys and route are chosen for its model, so another model may have no
    /// key on it; and the engine's worktree isolation force-removes the
    /// worktree when the child ends, which throws away whatever it changed.
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short description of the agent's task (3-5 words)"
                },
                "prompt": {
                    "type": "string",
                    "description": "The complete task for the agent to perform"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of tool names to make available. Defaults to all tools."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional instructions describing the agent's role"
                },
                "max_turns": {
                    "type": "number",
                    "description": "Maximum number of turns for the sub-agent (default: no limit)"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "If true, the agent starts immediately and this call returns an \
                                    agent_id without waiting for completion. Use the monitor tool \
                                    with action=status/output and task_id=agent_id. Default: false."
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params = match AgentInput::parse(&input) {
            Ok(params) => params,
            Err(error) => return ToolResult::error(error),
        };
        let Some(scope) = self.host.scope() else {
            return ToolResult::error("Sub-agents can only be started during a turn.");
        };
        let parent_id = match self
            .host
            .claim(&params.description, &params.prompt, CLAIM_TIMEOUT)
            .await
        {
            Some(id) => id,
            None => {
                tracing::warn!(
                    description = %params.description,
                    "agent: no announced Agent call matched; the sub-agent runs unlinked"
                );
                format!("agent-{}", uuid::Uuid::new_v4())
            }
        };

        let tools = child_tool_set(ctx, params.tools.as_deref());
        let mut query = child_template(&scope.query, &ctx.config, &scope.options);
        query.max_turns = params.max_turns.unwrap_or(u32::MAX);
        query.system_prompt = join_prompts(
            query.system_prompt.take(),
            Some(params.system_prompt.clone().unwrap_or_else(|| {
                "You are a specialized agent helping with a specific sub-task. Complete the task \
                 thoroughly and report your findings."
                    .to_owned()
            })),
        );
        query.enabled_tools = Some(tools.iter().map(|tool| tool.name().to_owned()).collect());

        let cancel = ctx.cancel_token.child_token();
        let mut child_ctx = ctx.clone();
        // Not the parent's: a background child holding it would keep the
        // turn's question forwarder — and with it the turn — open.
        child_ctx.user_question_tx = None;
        // The loop stores its step number here every step; a shared counter
        // would have the child overwrite the parent's.
        child_ctx.current_turn = Arc::new(AtomicUsize::new(0));
        // The shell's cwd and the todo list are kept per session id.
        child_ctx.session_id = format!("{}::{}", ctx.session_id, parent_id);
        child_ctx.completion_notifier = None;
        child_ctx.cancel_token = cancel.clone();

        let stopped_by_user = Arc::new(AtomicBool::new(false));
        self.host.children.lock().insert(
            parent_id.clone(),
            ChildHandle {
                cancel: cancel.clone(),
                background: params.run_in_background,
                stopped_by_user: stopped_by_user.clone(),
            },
        );
        let started = Instant::now();
        self.host.emit(
            &parent_id,
            SubagentEvent::Started {
                description: params.description.clone(),
                prompt: params.prompt.clone(),
                model: query.model.clone(),
                background: params.run_in_background,
                started_at_ms: unix_millis(),
            },
        );

        let run = ChildRun {
            host: self.host.clone(),
            parent_id: parent_id.clone(),
            client: scope.client.clone(),
            tools,
            query,
            ctx: child_ctx,
            cancel: cancel.clone(),
            prompt: params.prompt.clone(),
        };

        if params.run_in_background {
            let mut task = claurst_core::tasks::BackgroundTask::new(format!(
                "subagent: {}",
                params.description
            ));
            task.id = parent_id.clone();
            task.cancel_token = Some(cancel);
            let _ = claurst_core::tasks::global_registry().register(task);
            self.host.background.lock().insert(parent_id.clone());

            let host = self.host.clone();
            let id = parent_id.clone();
            tokio::spawn(async move {
                let outcome = run.run().await;
                let stopped = stopped_by_user.load(Ordering::Acquire);
                let registry = claurst_core::tasks::global_registry();
                let text = format_outcome(&outcome);
                registry.append_output(&id, &text);
                let cancelled = matches!(
                    registry.get(&id).map(|task| task.status),
                    Some(claurst_core::tasks::TaskStatus::Cancelled)
                );
                if !cancelled {
                    registry.update_status(&id, match status_of(&outcome, stopped) {
                        SubagentStatus::Completed => claurst_core::tasks::TaskStatus::Completed,
                        _ => claurst_core::tasks::TaskStatus::Failed(text.clone()),
                    });
                }
                finish(&host, &id, &outcome, stopped, started);
            });

            return ToolResult::success(
                json!({
                    "agent_id": parent_id,
                    "status": "running",
                    "message": format!(
                        "Agent '{}' started in background. Use monitor with action=status/output and task_id='{}'.",
                        params.description, parent_id
                    )
                })
                .to_string(),
            );
        }

        // Cancelling the parent's turn drops this future where it stands;
        // the guard is what tells the record the child stopped.
        let mut guard = FinishGuard {
            host: self.host.clone(),
            parent_id: parent_id.clone(),
            started,
            cancel,
            armed: true,
        };
        // On a task of its own, as a background child is. The engine polls a
        // message's calls on the parent's one task, so a child run inline
        // there froze its siblings whenever it waited on something blocking —
        // an approval dialog, a tool that reads the disk synchronously — and
        // sub-agents asked for together did not run together.
        let outcome = match tokio::spawn(run.run()).await {
            Ok(outcome) => outcome,
            Err(error) => QueryOutcome::Error(claurst_core::error::ClaudeError::Other(format!(
                "the sub-agent stopped unexpectedly: {error}"
            ))),
        };
        guard.armed = false;
        let stopped = stopped_by_user.load(Ordering::Acquire);
        finish(&self.host, &parent_id, &outcome, stopped, started);
        tool_result(outcome, stopped)
    }
}

/// Everything one child run needs, owned, so a background run can move it
/// onto its own task.
struct ChildRun {
    host: Arc<SubagentHost>,
    parent_id: String,
    client: Arc<AnthropicClient>,
    tools: Vec<Box<dyn Tool>>,
    query: QueryConfig,
    ctx: ToolContext,
    cancel: CancellationToken,
    prompt: String,
}

impl ChildRun {
    async fn run(self) -> QueryOutcome {
        let (tx, rx) = mpsc::unbounded_channel::<QueryEvent>();
        let forwarder = tokio::spawn(forward_child(
            self.host.clone(),
            self.parent_id.clone(),
            rx,
        ));
        let mut messages = vec![Message::user(self.prompt)];
        let outcome = claurst_query::run_query_loop(
            self.client.as_ref(),
            &mut messages,
            &self.tools,
            &self.ctx,
            &self.query,
            self.ctx.cost_tracker.clone(),
            Some(tx),
            self.cancel,
            None,
        )
        .await;
        // The loop dropped its sender on return, so this ends once the last
        // event is out — before the `Finished` that follows.
        let _ = forwarder.await;
        outcome
    }
}

/// Carry what the child does to the session's sink, as [`SubagentEvent`]s.
/// Text arriving together is sent together: the stream is a delta per
/// token, and each one is a message across the daemon wire.
async fn forward_child(
    host: Arc<SubagentHost>,
    parent_id: String,
    mut rx: mpsc::UnboundedReceiver<QueryEvent>,
) {
    let mut decoder = StreamDecoder::new(None);
    while let Some(first) = rx.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        let mut text = String::new();
        for event in batch {
            for translated in decoder.push(event) {
                match child_event(translated) {
                    Some(SubagentEvent::Text(delta)) => text.push_str(&delta),
                    Some(other) => {
                        if !text.is_empty() {
                            host.emit(&parent_id, SubagentEvent::Text(std::mem::take(&mut text)));
                        }
                        host.emit(&parent_id, other);
                    }
                    None => {}
                }
            }
        }
        if !text.is_empty() {
            host.emit(&parent_id, SubagentEvent::Text(text));
        }
    }
}

/// The part of a child's stream its record shows: what it wrote and the
/// tools it called. Its usage is already in the session's totals (the cost
/// tracker is shared) and must not move the parent's context meter; its
/// errors end up in `Finished`.
fn child_event(event: AgentEvent) -> Option<SubagentEvent> {
    match event {
        AgentEvent::Text(text) => Some(SubagentEvent::Text(text)),
        AgentEvent::ToolStarted { id, name, input } => {
            Some(SubagentEvent::ToolStarted { id, name, input })
        }
        AgentEvent::ToolFinished {
            id,
            name,
            output,
            failed,
            image_source,
        } => Some(SubagentEvent::ToolFinished {
            id,
            name,
            output,
            failed,
            image_source,
        }),
        _ => None,
    }
}

/// Sends `Finished { Stopped }` if dropped while armed — the one way a
/// foreground child ends without reaching its own ending.
struct FinishGuard {
    host: Arc<SubagentHost>,
    parent_id: String,
    started: Instant,
    /// The child's token: its run is a task of its own, which would
    /// otherwise outlive the call it answers.
    cancel: CancellationToken,
    armed: bool,
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.cancel.cancel();
        self.host.children.lock().remove(&self.parent_id);
        self.host.emit(
            &self.parent_id,
            SubagentEvent::Finished {
                status: SubagentStatus::Stopped,
                summary: None,
                result: None,
                duration_ms: elapsed_ms(self.started),
            },
        );
    }
}

fn finish(
    host: &SubagentHost,
    parent_id: &str,
    outcome: &QueryOutcome,
    stopped_by_user: bool,
    started: Instant,
) {
    host.children.lock().remove(parent_id);
    let status = status_of(outcome, stopped_by_user);
    let (summary, result) = match outcome {
        QueryOutcome::EndTurn { message, .. } => (None, Some(message.get_all_text())),
        QueryOutcome::MaxTokens {
            partial_message, ..
        } => (None, Some(partial_message.get_all_text())),
        QueryOutcome::Cancelled => (None, None),
        other => (Some(format_outcome(other)), None),
    };
    host.emit(
        parent_id,
        SubagentEvent::Finished {
            status,
            summary,
            result: result.filter(|text| !text.trim().is_empty()),
            duration_ms: elapsed_ms(started),
        },
    );
}

fn status_of(outcome: &QueryOutcome, stopped_by_user: bool) -> SubagentStatus {
    if stopped_by_user {
        return SubagentStatus::Stopped;
    }
    match outcome {
        QueryOutcome::EndTurn { .. } | QueryOutcome::MaxTokens { .. } => SubagentStatus::Completed,
        QueryOutcome::Cancelled => SubagentStatus::Stopped,
        QueryOutcome::Error(_) | QueryOutcome::BudgetExceeded { .. } => SubagentStatus::Failed,
    }
}

/// What the parent model reads back from a foreground child, worded as the
/// engine's own tool words it.
fn tool_result(outcome: QueryOutcome, stopped_by_user: bool) -> ToolResult {
    match outcome {
        QueryOutcome::EndTurn { message, .. } => ToolResult::success(message.get_all_text()),
        QueryOutcome::MaxTokens {
            partial_message, ..
        } => ToolResult::success(format!(
            "{}\n\n[Note: Agent hit max_tokens limit]",
            partial_message.get_all_text()
        )),
        QueryOutcome::Cancelled if stopped_by_user => {
            ToolResult::error("Sub-agent was stopped by the user")
        }
        QueryOutcome::Cancelled => ToolResult::error("Sub-agent was cancelled"),
        QueryOutcome::Error(error) => ToolResult::error(format!("Sub-agent error: {error}")),
        QueryOutcome::BudgetExceeded {
            cost_usd,
            limit_usd,
        } => ToolResult::error(format!(
            "Sub-agent stopped: budget ${cost_usd:.4} exceeded (limit ${limit_usd:.4})"
        )),
    }
}

/// A background child's result, as the engine records it for `monitor`.
fn format_outcome(outcome: &QueryOutcome) -> String {
    match outcome {
        QueryOutcome::EndTurn { message, .. } => message.get_all_text(),
        QueryOutcome::MaxTokens {
            partial_message, ..
        } => format!(
            "{}\n\n[Note: Agent hit max_tokens limit]",
            partial_message.get_all_text()
        ),
        QueryOutcome::Cancelled => "[Agent was cancelled]".to_owned(),
        QueryOutcome::Error(error) => format!("[Agent error: {error}]"),
        QueryOutcome::BudgetExceeded {
            cost_usd,
            limit_usd,
        } => format!("[Agent stopped: budget ${cost_usd:.4} exceeded (limit ${limit_usd:.4})]"),
    }
}

/// The child's config: the parent's, without what belongs to the parent's
/// turn alone — its steering queue and goal continuation — and with the
/// session's rules re-derived for an agent that is not the one planning.
pub(crate) fn child_template(
    parent: &QueryConfig,
    config: &Config,
    options: &AgentStartOptions,
) -> QueryConfig {
    let mut child = parent.clone();
    child.command_queue = None;
    child.continuation = claurst_query::ContinuationMode::Default;
    child.agent_name = None;
    child.agent_definition = None;
    child.managed_agents = None;
    // The plan-mode rule tells the model to hand its plan back with
    // ExitPlanMode, which the child does not have.
    let mut child_options = options.clone();
    child_options.plan_mode = false;
    crate::config::refresh_session_rules(&mut child, config, &child_options);
    let rule = if options.plan_mode {
        format!("{SUBAGENT_RULE}\n\n{SUBAGENT_PLAN_RULE}")
    } else {
        SUBAGENT_RULE.to_owned()
    };
    child.append_system_prompt = join_prompts(child.append_system_prompt.take(), Some(rule));
    child
}

/// The parent's tool set as the child gets it: the same built-ins the user
/// left on, the same MCP tools, minus [`CHILD_EXCLUDED_TOOLS`], narrowed to
/// `allowed` when the call names a list.
pub(crate) fn child_tool_set(ctx: &ToolContext, allowed: Option<&[String]>) -> Vec<Box<dyn Tool>> {
    let mut tools = crate::session::engine_tools(
        &ctx.config.disallowed_tools,
        ctx.mcp_manager.as_ref(),
    );
    tools.retain(|tool| !CHILD_EXCLUDED_TOOLS.contains(&tool.name()));
    if let Some(allowed) = allowed.filter(|allowed| !allowed.is_empty()) {
        tools.retain(|tool| allowed.iter().any(|name| name == tool.name()));
    }
    tools
}

fn join_prompts(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first.filter(|text| !text.trim().is_empty()), second) {
        (Some(first), Some(second)) => Some(format!("{first}\n\n{second}")),
        (first, second) => first.or(second),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> (Arc<SubagentHost>, Arc<Mutex<Vec<AgentEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            EventSink::new(move |event| seen.lock().push(event))
        };
        (SubagentHost::new(sink), seen)
    }

    fn call(description: &str, prompt: &str) -> Value {
        json!({ "description": description, "prompt": prompt })
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    /// Two calls in one message are announced before either runs; each run
    /// finds its own by its text, in whatever order they come.
    #[test]
    fn a_run_claims_the_call_with_its_own_text() {
        let (host, _) = host();
        host.announce("call-a", &call("Find login", "look for login"));
        host.announce("call-b", &call("Find logout", "look for logout"));
        runtime().block_on(async {
            assert_eq!(
                host.claim("Find logout", "look for logout", Duration::from_millis(10))
                    .await
                    .as_deref(),
                Some("call-b")
            );
            assert_eq!(
                host.claim(" Find login ", "look for login", Duration::from_millis(10))
                    .await
                    .as_deref(),
                Some("call-a")
            );
        });
    }

    /// The forwarder runs on its own task, so a run may look before the
    /// announcement is in; it waits for it.
    #[test]
    fn a_claim_waits_for_a_late_announcement() {
        let (host, _) = host();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .unwrap();
        let announcer = host.clone();
        let claimed = rt.block_on(async move {
            let late = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                announcer.announce("call-late", &call("Late", "p"));
            });
            let claimed = host.claim("Late", "p", Duration::from_secs(2)).await;
            let _ = late.await;
            claimed
        });
        assert_eq!(claimed.as_deref(), Some("call-late"));
    }

    #[test]
    fn a_claim_nothing_answers_gives_up() {
        let (host, _) = host();
        let claimed =
            runtime().block_on(host.claim("Nobody", "p", Duration::from_millis(20)));
        assert!(claimed.is_none());
    }

    /// Identical calls are claimed in the order they were announced, and a
    /// lone pending call is taken even when a hook changed its text.
    #[test]
    fn identical_calls_are_claimed_first_in_first_out() {
        let (host, _) = host();
        host.announce("first", &call("Same", "same"));
        host.announce("second", &call("Same", "same"));
        assert_eq!(host.take_match("Same", "same").as_deref(), Some("first"));
        assert_eq!(host.take_match("Rewritten", "by a hook").as_deref(), Some("second"));
        assert_eq!(host.take_match("Same", "same"), None);
    }

    /// A call that finished without running, and every call left pending
    /// when the turn ends, can no longer be claimed.
    #[test]
    fn forgotten_and_stale_calls_cannot_be_claimed() {
        let (host, _) = host();
        host.announce("refused", &call("A", "a"));
        host.forget("refused");
        assert_eq!(host.take_match("A", "a"), None);
        host.announce("stale", &call("B", "b"));
        host.end_turn();
        assert_eq!(host.take_match("B", "b"), None);
    }

    /// The record shows what the child wrote and the tools it called — not
    /// its usage, which is not the parent's context, nor turn bookkeeping.
    #[test]
    fn only_text_and_tool_calls_reach_the_record() {
        assert!(matches!(
            child_event(AgentEvent::Text("hi".into())),
            Some(SubagentEvent::Text(_))
        ));
        assert!(matches!(
            child_event(AgentEvent::ToolStarted {
                id: "t".into(),
                name: "Grep".into(),
                input: Value::Null
            }),
            Some(SubagentEvent::ToolStarted { .. })
        ));
        assert!(child_event(AgentEvent::TurnStarted).is_none());
        assert!(child_event(AgentEvent::Usage {
            context_tokens: Some(1),
            context_window: None
        })
        .is_none());
        assert!(child_event(AgentEvent::Error("boom".into())).is_none());
    }

    #[test]
    fn outcomes_read_back_as_the_engines_own_tool_words_them() {
        let text = |result: ToolResult| (result.content, result.is_error);
        assert_eq!(
            text(tool_result(QueryOutcome::Cancelled, false)),
            ("Sub-agent was cancelled".to_owned(), true)
        );
        assert_eq!(
            text(tool_result(QueryOutcome::Cancelled, true)).1,
            true
        );
        assert_eq!(
            text(tool_result(
                QueryOutcome::BudgetExceeded {
                    cost_usd: 1.0,
                    limit_usd: 0.5
                },
                false
            )),
            (
                "Sub-agent stopped: budget $1.0000 exceeded (limit $0.5000)".to_owned(),
                true
            )
        );
        assert_eq!(format_outcome(&QueryOutcome::Cancelled), "[Agent was cancelled]");
        assert_eq!(status_of(&QueryOutcome::Cancelled, false), SubagentStatus::Stopped);
        assert_eq!(
            status_of(
                &QueryOutcome::BudgetExceeded {
                    cost_usd: 1.0,
                    limit_usd: 0.5
                },
                false
            ),
            SubagentStatus::Failed
        );
        // Whatever the loop answered, a child the user stopped was stopped.
        assert_eq!(
            status_of(&QueryOutcome::Error(claurst_core::error::ClaudeError::Other("x".into())), true),
            SubagentStatus::Stopped
        );
    }

    #[test]
    fn the_input_is_checked_and_read() {
        assert!(AgentInput::parse(&json!({ "prompt": "p" })).is_err());
        assert!(AgentInput::parse(&json!({ "description": " ", "prompt": "p" })).is_err());
        let input = AgentInput::parse(&json!({
            "description": " Find it ",
            "prompt": "p",
            "tools": ["Read", "Grep"],
            "max_turns": 4,
            "run_in_background": true,
        }))
        .unwrap();
        assert_eq!(input.description, "Find it");
        assert_eq!(input.tools.as_deref(), Some(&["Read".to_owned(), "Grep".to_owned()][..]));
        assert_eq!(input.max_turns, Some(4));
        assert!(input.run_in_background);
    }

    /// A foreground child dropped mid-run — its parent's turn was stopped —
    /// still ends its record, and its run, on a task of its own, is told to
    /// stop rather than left to finish unseen.
    #[test]
    fn a_dropped_run_ends_its_record_as_stopped() {
        let (host, seen) = host();
        let child = CancellationToken::new();
        drop(FinishGuard {
            host: host.clone(),
            parent_id: "call".into(),
            started: Instant::now(),
            cancel: child.clone(),
            armed: true,
        });
        assert!(child.is_cancelled());
        let seen = seen.lock();
        assert!(matches!(
            seen.as_slice(),
            [AgentEvent::Subagent {
                event: SubagentEvent::Finished {
                    status: SubagentStatus::Stopped,
                    ..
                },
                ..
            }]
        ));
        drop(seen);
        let finished = CancellationToken::new();
        drop(FinishGuard {
            host,
            parent_id: "done".into(),
            started: Instant::now(),
            cancel: finished.clone(),
            armed: false,
        });
        assert!(!finished.is_cancelled());
    }

    #[test]
    fn the_session_rules_follow_the_child_and_plan_mode_does_not() {
        let options = AgentStartOptions {
            plan_mode: true,
            ..AgentStartOptions::default()
        };
        let mut parent = QueryConfig::default();
        parent.command_queue = Some(claurst_query::CommandQueue::new());
        parent.continuation = claurst_query::ContinuationMode::Goal;
        let child = child_template(&parent, &Config::default(), &options);
        assert!(child.command_queue.is_none());
        assert!(matches!(child.continuation, claurst_query::ContinuationMode::Default));
        let rules = child.append_system_prompt.unwrap();
        assert!(rules.contains("You are a sub-agent"));
        assert!(rules.contains("plan mode: research and report only"));
        assert!(!rules.contains("call ExitPlanMode"));
    }

    #[test]
    fn the_child_gets_the_parents_tools_minus_the_sessions_own() {
        let ctx = test_context();
        let names: Vec<String> = child_tool_set(&ctx, None)
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        for excluded in CHILD_EXCLUDED_TOOLS {
            assert!(!names.iter().any(|name| name == excluded), "{excluded}");
        }
        assert!(names.iter().any(|name| name == "Read"));
        let narrowed: Vec<String> = child_tool_set(&ctx, Some(&["Read".to_owned(), "Agent".to_owned()]))
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(narrowed, ["Read"]);
    }

    fn test_context() -> ToolContext {
        ToolContext {
            working_dir: std::path::PathBuf::from("."),
            permission_mode: claurst_core::PermissionMode::Default,
            permission_handler: Arc::new(claurst_core::permissions::AutoPermissionHandler {
                mode: claurst_core::PermissionMode::Default,
            }),
            cost_tracker: claurst_core::CostTracker::new(),
            session_id: "test".into(),
            file_history: Arc::new(parking_lot::Mutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            current_turn: Arc::new(AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config: Config::default(),
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
            cancel_token: CancellationToken::new(),
        }
    }
}
