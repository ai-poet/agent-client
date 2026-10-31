//! Fork: Devin Fusion-style lead / sidekick execution for the built-in agent.
//!
//! The lead is the ordinary session and the only agent that talks to the user.
//! This module owns one persistent, independently-routed sidekick. The two
//! agents exchange briefs and reports, not their whole conversations; each
//! keeps its own cached context. No vendored engine code is changed.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use claurst_api::AnthropicClient;
use claurst_core::PermissionManager;
use claurst_core::config::Config;
use claurst_core::types::Message;
use claurst_query::{QueryConfig, QueryEvent, QueryOutcome};
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::{
    AccessMode, AgentStartOptions, apply_compaction_settings, build_config, build_query_config,
    context_window_for, load_settings, route_fingerprint, settings_document,
};
use crate::events::{SubagentEvent, SubagentStatus};
use crate::history;
use crate::permission::{GuiPermissionHandler, PermissionBridge, SidekickScope};
use crate::subagent::{
    AgentInput, CLAIM_TIMEOUT, FinishGuard, SubagentHost, child_template, child_tool_set,
    elapsed_ms, forward_child, join_prompts, tool_result, unix_millis,
};

pub(crate) const SIDEKICK_TOOL_NAME: &str = "SidekickBrief";

const LEAD_RULE: &str = "You are running in Fusion mode as the lead. You own the user's request, \
the plan, every ambiguous or design-critical decision, and the final review. The user talks to \
you alone. Use SidekickBrief for well-defined mechanical implementation, code exploration needed \
to carry out an already-settled plan, builds, tests, and other repetitive verification. Do work \
yourself when it shapes the plan or needs judgment; do not delegate understanding.\n\
\n\
Each brief must be self-contained: the goal and why, relevant files and constraints, exactly what \
to change or verify, and the success criteria. The sidekick keeps its own context between briefs, \
so later briefs may contain feedback on its previous result instead of repeating the whole task. \
Always wait for its report and review the actual changes and test evidence before telling the \
user the work is done. Never assume a report means the change is correct.\n\
\n\
SIDEKICK_ESCALATION_REQUIRED means an action needed a decision only the user may make. Take \
control back: inspect the request, then either perform the appropriate action yourself through \
your normal permission flow or explain why it must not run. Do not send the same sidekick action \
back unchanged. SIDEKICK_FAILED means the sidekick could not finish: diagnose the report, then \
fix it yourself or delegate a corrected brief. Never hide either outcome from the user.";

const SIDEKICK_RULE: &str = "You are the persistent Fusion sidekick. The lead gives you briefs \
and reviews your work; you never talk to the user. Carry out each brief within its constraints, \
including the requested builds and tests. If an action would require the user's approval, stop \
that action and report it rather than trying to bypass the refusal. Your final answer is a \
self-contained report for the lead: result, files changed, commands and tests run with their \
outcomes, unresolved problems, and anything the lead must review or do itself.";

const TOOL_DESCRIPTION: &str = "Delegate a well-defined execution brief to your persistent \
Fusion sidekick. Use it for mechanical implementation, builds, tests and verification after you \
have settled the plan. It has its own model, tools and persistent context, and returns a report \
for your review. Give it the goal, relevant files, constraints and success criteria. Later calls \
may carry feedback on earlier work. Do not use it for ambiguous product decisions or for work \
you have not reviewed yet. The call runs in the foreground: you must review every report.";

struct SidekickEngine {
    options: AgentStartOptions,
    config: Mutex<Config>,
    query: Mutex<QueryConfig>,
    client: Mutex<Arc<AnthropicClient>>,
}

/// The configured pair's sidekick half. The lead owns the `SubagentHost`; this
/// value deliberately does not point back at it, so the two do not form an Arc
/// cycle. `SidekickBriefTool` borrows both for the duration of a call.
pub(crate) struct FusionHost {
    engine: SidekickEngine,
    bridge: Arc<PermissionBridge>,
    history: AsyncMutex<Vec<Message>>,
    history_path: Option<PathBuf>,
    permission_manager: Arc<std::sync::Mutex<PermissionManager>>,
    access: Mutex<(AccessMode, bool)>,
    escalations: Arc<Mutex<Vec<crate::permission::SidekickEscalation>>>,
    running: AtomicBool,
}

impl FusionHost {
    /// Build the second route from the same routing document as the lead, but
    /// never from the lead's resolved client: custom endpoint keys and the
    /// gateway's per-model keys must stay with their own model.
    pub fn new(
        lead: &AgentStartOptions,
        bridge: Arc<PermissionBridge>,
    ) -> anyhow::Result<Option<Arc<Self>>> {
        let Some(pair) = lead.fusion.clone() else {
            return Ok(None);
        };
        if pair.sidekick_model.trim().is_empty() {
            anyhow::bail!("Fusion needs a sidekick model");
        }
        let history_path = lead.fusion_history_dir.as_ref().and_then(|directory| {
            lead.session_id
                .as_ref()
                .map(|id| directory.join(format!("{id}.json")))
        });
        let saved = history_path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default();

        let mut options = lead.clone();
        options.model = Some(pair.sidekick_model);
        options.platform = pair.sidekick_platform;
        options.wire_format = pair.sidekick_wire_format;
        options.reasoning_effort = pair.sidekick_reasoning_effort;
        options.history = Vec::new();
        options.computer_use = None;
        options.browser_tools = false;
        options.trajectory_dir = None;
        options.fusion = None;
        options.fusion_history_dir = None;
        options.session_id = lead.session_id.as_ref().map(|id| format!("{id}::sidekick"));

        let mut config = build_config(&options)?;
        apply_compaction_settings(&mut config, settings_document().as_ref());
        let (client, registry) =
            crate::session::build_clients(&config, options.platform.as_deref())?;
        let mut query = build_query_config(&config, &options);
        query.provider_registry = Some(registry);
        let provider = config.selected_provider_id().to_owned();
        query.context_window = context_window_for(&config, &query.model, &provider);
        let permission_manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
            config.permission_mode.clone(),
            &load_settings()?,
        )));

        Ok(Some(Arc::new(Self {
            engine: SidekickEngine {
                options,
                config: Mutex::new(config),
                query: Mutex::new(query),
                client: Mutex::new(client),
            },
            bridge,
            history: AsyncMutex::new(history::deserialize(&saved)),
            history_path,
            permission_manager,
            access: Mutex::new((lead.access_mode, lead.plan_mode)),
            escalations: Arc::new(Mutex::new(Vec::new())),
            running: AtomicBool::new(false),
        })))
    }

    pub(crate) fn apply_access(&self, mode: AccessMode, plan: bool) {
        *self.access.lock() = (mode, plan);
        self.refresh_route();
        let permission_mode = mode.permission_mode(plan);
        let Ok(settings) = load_settings() else {
            return;
        };
        if let Ok(mut manager) = self.permission_manager.lock() {
            *manager = PermissionManager::new(permission_mode, &settings);
        }
    }

    /// Pick up a refreshed gateway key or an edited custom endpoint without
    /// disturbing a brief already running on its snapshot of the old route.
    fn refresh_route(&self) {
        let (access, plan) = *self.access.lock();
        let mut options = self.engine.options.clone();
        options.access_mode = access;
        options.plan_mode = plan;
        let mut config = match build_config(&options) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(%error, "fusion: could not refresh the sidekick route");
                return;
            }
        };
        apply_compaction_settings(&mut config, settings_document().as_ref());
        let route_changed =
            route_fingerprint(&self.engine.config.lock()) != route_fingerprint(&config);
        let mut query = build_query_config(&config, &options);
        let provider = config.selected_provider_id().to_owned();
        query.context_window = context_window_for(&config, &query.model, &provider);
        if route_changed {
            match crate::session::build_clients(&config, options.platform.as_deref()) {
                Ok((client, registry)) => {
                    *self.engine.client.lock() = client;
                    query.provider_registry = Some(registry);
                }
                Err(error) => {
                    tracing::warn!(%error, "fusion: could not rebuild the sidekick client");
                    return;
                }
            }
        } else {
            query.provider_registry = self.engine.query.lock().provider_registry.clone();
        }
        *self.engine.config.lock() = config;
        *self.engine.query.lock() = query;
    }

    /// A rewind removes the lead turns those briefs belonged to. Keeping the
    /// sidekick's version of that discarded future would be worse than starting
    /// its context afresh.
    pub(crate) fn reset_after_rewind(&self) {
        if let Ok(mut messages) = self.history.try_lock() {
            messages.clear();
            self.persist(&messages);
        }
    }

    pub(crate) fn extend_lead_rules(&self, query: &mut QueryConfig) {
        query.append_system_prompt = join_prompts(
            query.append_system_prompt.take(),
            Some(LEAD_RULE.to_owned()),
        );
    }

    fn persist(&self, messages: &[Message]) {
        let Some(path) = &self.history_path else {
            return;
        };
        let bytes = history::serialize(messages);
        let text = String::from_utf8_lossy(&bytes);
        if let Err(error) = agent_teams::store::atomic_write_text(path, &text) {
            tracing::warn!(%error, path = %path.display(), "fusion: could not save sidekick history");
        }
    }

    /// Best-effort final flush. A running loop owns the async lock; its normal
    /// ending persists synchronously after the loop, and a dropped session
    /// still has the last completed brief on disk.
    pub fn shutdown(&self) {
        if let Ok(messages) = self.history.try_lock() {
            self.persist(&messages);
        }
    }

    fn sidekick_query(
        &self,
        params: &AgentInput,
        tools: &[Box<dyn Tool>],
        base: &QueryConfig,
        config: &Config,
    ) -> QueryConfig {
        let (access, plan) = *self.access.lock();
        let mut options = self.engine.options.clone();
        options.access_mode = access;
        options.plan_mode = plan;
        let mut query = child_template(base, config, &options);
        query.max_turns = params.max_turns.unwrap_or(u32::MAX);
        query.append_system_prompt = join_prompts(
            query.append_system_prompt.take(),
            Some(SIDEKICK_RULE.to_owned()),
        );
        query.system_prompt =
            join_prompts(query.system_prompt.take(), params.system_prompt.clone());
        query.enabled_tools = Some(tools.iter().map(|tool| tool.name().to_owned()).collect());
        query
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute(
        self: &Arc<Self>,
        subagents: &Arc<SubagentHost>,
        params: AgentInput,
        ctx: &ToolContext,
        cancel: CancellationToken,
        parent_id: String,
    ) -> ToolResult {
        let tools = child_tool_set(ctx, params.tools.as_deref());
        let config = self.engine.config.lock().clone();
        let base_query = self.engine.query.lock().clone();
        let client = self.engine.client.lock().clone();
        let query = self.sidekick_query(&params, &tools, &base_query, &config);
        let model = query.model.clone();
        let mut child_ctx = ctx.clone();
        child_ctx.user_question_tx = None;
        child_ctx.current_turn = Arc::new(AtomicUsize::new(0));
        child_ctx.session_id = self
            .engine
            .options
            .session_id
            .clone()
            .unwrap_or_else(|| format!("fusion-sidekick-{parent_id}"));
        child_ctx.completion_notifier = None;
        child_ctx.cancel_token = cancel.clone();
        child_ctx.managed_agent_config = None;
        child_ctx.config = config;
        let (access, plan) = *self.access.lock();
        child_ctx.config.permission_mode = access.permission_mode(plan);
        child_ctx.permission_mode = child_ctx.config.permission_mode.clone();
        child_ctx.permission_handler = Arc::new(GuiPermissionHandler::for_sidekick(
            self.bridge.clone(),
            SidekickScope {
                manager: self.permission_manager.clone(),
                escalations: self.escalations.clone(),
            },
        ));

        let started = Instant::now();
        let stopped_by_user = Arc::new(AtomicBool::new(false));
        subagents.track_foreground(&parent_id, cancel.clone(), stopped_by_user.clone());
        subagents.emit(
            &parent_id,
            SubagentEvent::Started {
                description: format!("Sidekick · {}", params.description),
                prompt: params.prompt.clone(),
                model,
                background: false,
                started_at_ms: unix_millis(),
            },
        );
        let mut guard = FinishGuard {
            host: subagents.clone(),
            parent_id: parent_id.clone(),
            started,
            cancel: cancel.clone(),
            armed: true,
        };
        self.escalations.lock().clear();

        let (tx, rx) = mpsc::unbounded_channel::<QueryEvent>();
        let forwarder = tokio::spawn(forward_child(subagents.clone(), parent_id.clone(), rx));
        let outcome = {
            let mut messages = self.history.lock().await;
            messages.push(Message::user(params.prompt.clone()));
            claurst_query::run_query_loop(
                client.as_ref(),
                &mut messages,
                &tools,
                &child_ctx,
                &query,
                child_ctx.cost_tracker.clone(),
                Some(tx),
                cancel,
                None,
            )
            .await
        };
        let _ = forwarder.await;
        if let Ok(messages) = self.history.try_lock() {
            self.persist(&messages);
        }

        guard.armed = false;
        subagents.untrack(&parent_id);
        let stopped = stopped_by_user.load(Ordering::Acquire);
        let progress = subagents.take_progress(&parent_id);
        let escalations = std::mem::take(&mut *self.escalations.lock());
        let status = if stopped {
            SubagentStatus::Stopped
        } else if !escalations.is_empty() {
            SubagentStatus::Failed
        } else {
            match &outcome {
                QueryOutcome::EndTurn { .. } | QueryOutcome::MaxTokens { .. } => {
                    SubagentStatus::Completed
                }
                QueryOutcome::Cancelled => SubagentStatus::Stopped,
                QueryOutcome::Error(_) | QueryOutcome::BudgetExceeded { .. } => {
                    SubagentStatus::Failed
                }
            }
        };
        let report = match &outcome {
            QueryOutcome::EndTurn { message, .. } => Some(message.get_all_text()),
            QueryOutcome::MaxTokens {
                partial_message, ..
            } => Some(partial_message.get_all_text()),
            _ => None,
        }
        .filter(|text| !text.trim().is_empty());
        subagents.emit(
            &parent_id,
            SubagentEvent::Finished {
                status,
                summary: (!escalations.is_empty())
                    .then(|| "Approval required; returned to the lead".to_owned()),
                result: report,
                duration_ms: elapsed_ms(started),
            },
        );

        let mut result = tool_result(outcome, stopped, progress.as_ref());
        if !escalations.is_empty() {
            let requests = escalations
                .iter()
                .map(|request| {
                    let path = request
                        .path
                        .as_deref()
                        .map(|path| format!("; path: {path}"))
                        .unwrap_or_default();
                    format!(
                        "- {}: {} (reason: {}{path})",
                        request.tool_name, request.description, request.reason
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            result.is_error = true;
            result.content = format!(
                "SIDEKICK_ESCALATION_REQUIRED\nThe sidekick did not perform the following \
                 undecided action(s). You are the lead: review them and either perform the \
                 appropriate action yourself through the normal permission flow or explain why \
                 it must not run. Do not delegate the same action unchanged.\n{requests}\n\n\
                 Sidekick progress and report:\n{}",
                result.content
            );
        } else if result.is_error {
            result.content = format!(
                "SIDEKICK_FAILED\nThe sidekick could not complete the brief. Take control back, \
                 diagnose the report, then fix it yourself or send a corrected brief.\n{}",
                result.content
            );
        } else {
            result.content = format!(
                "SIDEKICK_REPORT\n{}\n\nReview the actual changes and verification before you \
                 report completion to the user.",
                result.content
            );
        }
        subagents.remember_answer(&parent_id, &result);
        result
    }
}

pub(crate) struct SidekickBriefTool {
    fusion: Arc<FusionHost>,
    subagents: Arc<SubagentHost>,
}

impl SidekickBriefTool {
    pub fn new(fusion: Arc<FusionHost>, subagents: Arc<SubagentHost>) -> Self {
        Self { fusion, subagents }
    }
}

#[async_trait]
impl Tool for SidekickBriefTool {
    fn name(&self) -> &str {
        SIDEKICK_TOOL_NAME
    }

    fn description(&self) -> &str {
        TOOL_DESCRIPTION
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short description of the execution brief (3-5 words)"
                },
                "prompt": {
                    "type": "string",
                    "description": "Self-contained goal, files, constraints and success criteria; or feedback on the previous brief"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tool allowlist for this brief"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional additional execution constraints"
                },
                "max_turns": {
                    "type": "number",
                    "description": "Maximum sidekick steps (default: no limit)"
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
        if params.run_in_background {
            return ToolResult::error(
                "Fusion sidekick briefs run in the foreground so the lead can review them.",
            );
        }
        if !self.subagents.in_turn() {
            return ToolResult::error("The sidekick can only run during a lead turn.");
        }
        if self.fusion.running.swap(true, Ordering::AcqRel) {
            return ToolResult::error(
                "The persistent sidekick is already running a brief. Wait for its report before sending feedback.",
            );
        }
        struct RunningGuard<'a>(&'a AtomicBool);
        impl Drop for RunningGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _running = RunningGuard(&self.fusion.running);

        let parent_id = match self
            .subagents
            .claim_for(
                SIDEKICK_TOOL_NAME,
                &params.description,
                &params.prompt,
                CLAIM_TIMEOUT,
            )
            .await
        {
            Some(id) => id,
            None => format!("sidekick-{}", uuid::Uuid::new_v4()),
        };
        let cancel = ctx.cancel_token.child_token();
        self.fusion
            .execute(&self.subagents, params, ctx, cancel, parent_id)
            .await
    }
}
