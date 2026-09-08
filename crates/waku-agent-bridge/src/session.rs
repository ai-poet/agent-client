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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use claurst_api::AnthropicClient;
use claurst_api::client::ClientConfig;
use claurst_core::config::{Config, Settings};
use claurst_core::types::Message;
use claurst_core::{CostTracker, PermissionManager};
use claurst_query::{CommandPriority, CommandQueue, QueryConfig, QueryEvent, QueryOutcome, QueuedCommand};
use claurst_tools::{Tool, ToolContext};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::{AgentStartOptions, TurnOptions, build_config, build_query_config};
use crate::events::{AgentEvent, EventSink, PermissionChoice, StreamDecoder};
use crate::history;
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

struct Inner {
    events: EventSink,
    id: String,
    cwd: PathBuf,
    client: Arc<AnthropicClient>,
    tools: Arc<Vec<Box<dyn Tool>>>,
    cost_tracker: Arc<CostTracker>,
    file_history: Arc<Mutex<claurst_core::file_history::FileHistory>>,
    manager: Arc<std::sync::Mutex<PermissionManager>>,
    bridge: Arc<PermissionBridge>,
    /// Filled in the background once the MCP roster has connected. A turn that
    /// starts before then simply runs without MCP resources rather than
    /// waiting for servers that may never come up.
    mcp: Mutex<Option<Arc<claurst_mcp::McpManager>>>,
    config: Mutex<Config>,
    query: Mutex<QueryConfig>,
    options: Mutex<AgentStartOptions>,
    history: Mutex<Vec<Message>>,
    turn: Mutex<Option<Turn>>,
    turn_counter: Arc<AtomicUsize>,
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
        let rt = runtime::shared()?;
        let settings = Arc::new(Mutex::new(Settings::load_sync().unwrap_or_default()));
        let config = build_config(&options);
        let query = build_query_config(&config, &options);

        // Resolving the key may refresh an OAuth token, so it is async. Every
        // other part of construction is not.
        let (api_key, use_bearer_auth) = rt.block_on(config.resolve_anthropic_auth_async()).unwrap_or_default();

        let client_config = ClientConfig {
            api_key,
            api_base: config.resolve_anthropic_api_base(),
            use_bearer_auth,
            ..Default::default()
        };
        let client = Arc::new(AnthropicClient::new(client_config.clone())?);

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

        let mut tools: Vec<Box<dyn Tool>> = claurst_tools::all_tools();
        tools.push(Box::new(claurst_query::AgentTool));

        let mut query = query;
        query.provider_registry = Some(Arc::new(claurst_api::ProviderRegistry::from_config(
            &config,
            client_config,
        )));

        let inner = Arc::new(Inner {
            events,
            id: uuid::Uuid::new_v4().to_string(),
            cwd: options.cwd.clone(),
            client,
            tools: Arc::new(tools),
            cost_tracker: CostTracker::new(),
            file_history: Arc::new(Mutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            manager,
            bridge,
            mcp: Mutex::new(None),
            config: Mutex::new(config),
            query: Mutex::new(query),
            history: Mutex::new(history::deserialize(&options.history)),
            options: Mutex::new(options),
            turn: Mutex::new(None),
            turn_counter: Arc::new(AtomicUsize::new(0)),
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
        if self.is_busy() {
            self.steer(text);
            return;
        }
        let inner = self.inner.clone();
        let Ok(rt) = runtime::shared() else {
            inner.events.emit(AgentEvent::Error(
                "the agent runtime is unavailable".to_string(),
            ));
            return;
        };

        let cancel = CancellationToken::new();
        let queue = CommandQueue::new();
        *inner.turn.lock() = Some(Turn {
            cancel: cancel.clone(),
            queue: queue.clone(),
            steers: Arc::new(Mutex::new(Vec::new())),
            watching: false,
        });

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
        turn.queue.push(
            QueuedCommand::InjectUserMessage(text.clone()),
            CommandPriority::Normal,
        );
        turn.steers.lock().push(text);
        if !turn.watching {
            turn.watching = true;
            spawn_steer_watcher(self.inner.clone(), turn.queue.clone(), turn.steers.clone());
        }
    }

    /// Stop the running turn. Releases anything blocked on an approval so no
    /// tool is left waiting on a dialog that is going away.
    pub fn cancel(&self) {
        let cancel = self.inner.turn.lock().as_ref().map(|turn| turn.cancel.clone());
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        self.inner.bridge.release_all();
    }

    /// Answer a [`AgentEvent::Permission`].
    pub fn respond(&self, request_id: &str, option_id: &str) {
        let Some(choice) = PermissionChoice::from_id(option_id) else {
            tracing::warn!(option_id, "agent: unknown permission option");
            return;
        };
        self.inner.bridge.resolve(request_id, choice);
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
        if changes.model.is_some() {
            options.model = changes.model;
        }
        if changes.reasoning_effort.is_some() {
            options.reasoning_effort = changes.reasoning_effort;
        }

        let config = build_config(&options);
        let query = build_query_config(&config, &options);
        self.inner.bridge.set_auto(options.access_mode.auto_answer());
        if let Ok(mut manager) = self.inner.manager.lock() {
            // The manager caches the mode it evaluates against; rebuild it so
            // a switch to Full Access stops asking immediately rather than at
            // the next session.
            *manager = PermissionManager::new(
                config.permission_mode.clone(),
                &Settings::load_sync().unwrap_or_default(),
            );
        }

        {
            // Held one at a time, and in the same order `run_turn` reads them,
            // so the two can never wait on each other.
            let mut current_query = self.inner.query.lock();
            // The registry owns live provider clients; rebuilding it on every
            // option change would drop connection pools for a model switch.
            let registry = current_query.provider_registry.clone();
            *current_query = query;
            current_query.provider_registry = registry;
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
    /// undone the moment that copy is written back.
    pub fn rollback(&self, turns: usize) -> anyhow::Result<usize> {
        if self.is_busy() {
            anyhow::bail!("cannot rewind while a turn is running");
        }
        let removed = history::rollback(&mut self.inner.history.lock(), turns);
        Ok(removed)
    }

    /// A copy of the conversation with the last `turns_to_remove` turns
    /// dropped, ready to seed a branch. Leaves this session untouched.
    pub fn fork(&self, turns_to_remove: usize) -> anyhow::Result<Vec<u8>> {
        let branched = history::fork(&self.inner.history.lock(), turns_to_remove);
        Ok(history::serialize(&branched))
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(turn) = self.turn.lock().as_ref() {
            turn.cancel.cancel();
        }
        self.bridge.release_all();
    }
}

/// Connect the configured MCP servers without holding up session start.
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
        *inner.mcp.lock() = Some(manager);
    });
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
            // The turn ending is handled by `run_turn`, which reports whatever
            // is still pending as rejected. Stopping here avoids a double
            // report.
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
    messages.push(Message::user(prompt));

    let config = inner.config.lock().clone();
    let mut query = inner.query.lock().clone();
    query.command_queue = Some(queue.clone());

    let handler: Arc<dyn claurst_core::PermissionHandler> =
        Arc::new(GuiPermissionHandler::new(inner.bridge.clone()));

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
        user_question_tx: None,
        cancel_token: cancel.clone(),
    };

    let (tx, rx) = mpsc::unbounded_channel::<QueryEvent>();
    let forwarder = {
        let inner = inner.clone();
        tokio::spawn(forward_events(inner, rx))
    };

    let outcome = claurst_query::run_query_loop(
        inner.client.as_ref(),
        &mut messages,
        inner.tools.as_slice(),
        &tool_ctx,
        &query,
        inner.cost_tracker.clone(),
        Some(tx),
        cancel,
        None,
    )
    .await;

    // The forwarder ends when the loop drops its sender.
    let _ = forwarder.await;

    // Take the turn down before reporting, so a `TurnFinished` handler that
    // immediately prompts again is not treated as steering.
    let leftover = {
        let mut guard = inner.turn.lock();
        let turn = guard.take();
        turn.map(|turn| turn.steers.lock().drain(..).collect::<Vec<_>>())
            .unwrap_or_default()
    };

    *inner.history.lock() = messages;

    let (success, summary) = describe(outcome);
    if !success && let Some(reason) = summary.clone() {
        inner.events.emit(AgentEvent::Error(reason));
    }
    inner.events.emit(AgentEvent::TurnFinished { success, summary });
    inner.events.emit(AgentEvent::HistoryCommitted(history::serialize(
        &inner.history.lock(),
    )));

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
}

async fn forward_events(inner: Arc<Inner>, mut rx: mpsc::UnboundedReceiver<QueryEvent>) {
    let mut decoder = StreamDecoder::new();
    while let Some(event) = rx.recv().await {
        for translated in decoder.push(event) {
            inner.events.emit(translated);
        }
    }
}

/// What to tell the transcript about how the turn ended.
///
/// `MaxTokens` counts as a success: the model produced an answer and the
/// engine's own recovery already ran. Everything else that is not `EndTurn`
/// carries a reason the user can act on — reporting a turn that produced
/// nothing as a success is the failure mode this exists to avoid.
fn describe(outcome: QueryOutcome) -> (bool, Option<String>) {
    match outcome {
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

    #[test]
    fn a_cancelled_turn_is_never_reported_as_a_success() {
        let (success, summary) = describe(QueryOutcome::Cancelled);
        assert!(!success);
        assert!(summary.is_some());
    }

    #[test]
    fn a_spend_cap_names_both_numbers() {
        let (success, summary) = describe(QueryOutcome::BudgetExceeded {
            cost_usd: 12.5,
            limit_usd: 10.0,
        });
        assert!(!success);
        let summary = summary.unwrap();
        assert!(summary.contains("$12.50"), "{summary}");
        assert!(summary.contains("$10.00"), "{summary}");
    }

    #[test]
    fn hitting_the_output_limit_still_counts_as_an_answer() {
        let (success, summary) = describe(QueryOutcome::MaxTokens {
            partial_message: Message::assistant("half an answer"),
            usage: Default::default(),
        });
        assert!(success);
        assert!(summary.is_some(), "the user should still be told why it stopped");
    }
}
