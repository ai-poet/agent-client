//! Sailry execution core behind the existing query-loop contract.
mod history;
mod model;
mod tools;

use agent_core::cost::CostTracker;
use agent_core::error::ClaudeError;
use agent_core::types::{Message, MessageContent, UsageInfo};
use agent_core_api::AnthropicClient;
use agent_core_engine::agent::engine::{self, EngineEvent, RunStatus};
use agent_core_engine::agent::model as core;
use agent_core_engine::store::agent::{Invocation, RuntimeSession, Sessions};
use agent_core_tools::{Tool, ToolContext};
use sailry_protocol::{
    conversation::RunKind, Effort, Permission, ProviderId, RequestId, SessionId, TurnId, WorkMode,
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{QueryConfig, QueryEvent, QueryOutcome};

pub async fn run(
    client: &AnthropicClient,
    messages: &mut Vec<Message>,
    tools: Arc<Vec<Box<dyn Tool>>>,
    tool_ctx: &ToolContext,
    config: &QueryConfig,
    cost_tracker: Arc<CostTracker>,
    event_tx: Option<mpsc::UnboundedSender<QueryEvent>>,
    cancel: CancellationToken,
    pending_messages: Option<&mut Vec<String>>,
) -> QueryOutcome {
    if config.provider_registry.is_none() {
        return crate::run_query_loop(
            client,
            messages,
            tools.as_slice(),
            tool_ctx,
            config,
            cost_tracker,
            event_tx,
            cancel,
            pending_messages,
        )
        .await;
    }
    if let Some(pending) = pending_messages {
        messages.extend(pending.drain(..).map(Message::user));
    }
    let mut query = config.clone();
    query
        .enabled_tools
        .get_or_insert_with(|| tools.iter().map(|tool| tool.name().to_string()).collect());
    let main = match model::ExistingModel::new(
        &query,
        &tool_ctx.config,
        &tool_ctx.session_id,
        event_tx.clone(),
    ) {
        Ok(model) => Arc::new(model),
        Err(error) => return QueryOutcome::Error(ClaudeError::Other(error.message)),
    };
    let summary = match model::ExistingModel::new(
        &query,
        &tool_ctx.config,
        &tool_ctx.session_id,
        event_tx.clone(),
    ) {
        Ok(model) => Arc::new(model),
        Err(error) => return QueryOutcome::Error(ClaudeError::Other(error.message)),
    };
    let invocation = invocation(&query);
    let session = RuntimeSession::new(
        invocation.turn.session,
        messages.iter().map(history::to_engine).collect(),
    );
    let sessions = Sessions::new(session);
    let steering = engine::SteerQueue::default();
    let forwarder = query
        .command_queue
        .clone()
        .map(|queue| forward_commands(queue, steering.clone(), cancel.clone()));
    let system = crate::runner::prompt::build_system_prompt(&query);
    let instruction = match system {
        agent_core_api::SystemPrompt::Text(text) => text,
        agent_core_api::SystemPrompt::Blocks(blocks) => blocks
            .into_iter()
            .map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    let main_config = core::Config {
        model_name: query.model.clone(),
        context_limit: query.context_window.map(|value| value as usize),
        max_tokens: Some(query.max_tokens as u64),
        temperature: query.temperature.map(f64::from),
        thinking_effort: engine_effort(query.effort_level),
        ..core::Config::default()
    };
    let summary_config = core::Config {
        max_tokens: Some(core::summary::output_limit(query.max_tokens) as u64),
        thinking_effort: Effort::Low,
        ..main_config.clone()
    };
    let tools = Arc::new(tools::ExistingTools::new(
        tools,
        tool_ctx.clone(),
        event_tx.clone(),
    ));
    let run = engine::Run {
        sessions: sessions.clone(),
        invocation,
        stop: cancel.clone(),
        binding: core::Binding {
            provider: main,
            config: main_config,
            owner: "waku".into(),
        },
        summary: core::Binding {
            provider: summary,
            config: summary_config,
            owner: "waku".into(),
        },
        instruction,
        context: String::new(),
        parts: Vec::new(),
        catalog: tools,
        callbacks: Vec::new(),
        deferred: BTreeSet::new(),
        context_limit: query.context_window.unwrap_or(0) as usize,
        max_turns: Some(query.max_turns),
        steering,
    };
    let seen_usage = Arc::new(Mutex::new(BTreeSet::new()));
    let checkpoint = query.checkpoint_history;
    let events = event_tx.clone();
    let listener_seen = seen_usage.clone();
    let listener = move |event: EngineEvent| {
        let Some(events) = &events else { return };
        match event {
            EngineEvent::Message(message) => forward_message(events, message),
            EngineEvent::Checkpoint(session) => {
                if checkpoint {
                    let _ = events.send(QueryEvent::Checkpoint(
                        session
                            .conversation
                            .iter()
                            .map(history::from_engine)
                            .collect(),
                    ));
                }
                for message in &session.conversation {
                    let Some(usage) = message.metadata.usage.as_deref() else {
                        continue;
                    };
                    let Some(id) = &message.id else { continue };
                    if !listener_seen.lock().unwrap().insert(id.clone()) {
                        continue;
                    }
                    let _ = events.send(QueryEvent::TurnComplete {
                        turn: 0,
                        stop_reason: if message.metadata.output_token_limit_reached {
                            "max_tokens".into()
                        } else {
                            "end_turn".into()
                        },
                        usage: Some(usage_current(usage)),
                    });
                }
            }
        }
    };
    let outcome = engine::run(run, &listener).await;
    if let Some(forwarder) = forwarder {
        forwarder.cancel.cancel();
        let _ = forwarder.task.await;
    }
    let session = sessions.snapshot().await;
    *messages = session
        .conversation
        .iter()
        .map(history::from_engine)
        .collect();
    let last = session
        .conversation
        .iter()
        .rev()
        .find(|message| message.role == core::Role::Assistant)
        .cloned();
    let usage = last
        .as_ref()
        .and_then(|message| message.metadata.usage.as_deref().map(usage_current))
        .unwrap_or_default();
    match outcome {
        Ok(outcome) if outcome.status == RunStatus::Cancelled => QueryOutcome::Cancelled,
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return QueryOutcome::Error(error_current(error));
            }
            let message = last.unwrap_or_else(|| {
                core::Message::assistant().with_text(outcome.output.unwrap_or_default())
            });
            if message.metadata.output_token_limit_reached {
                QueryOutcome::MaxTokens {
                    partial_message: history::from_engine(&message),
                    usage,
                }
            } else {
                QueryOutcome::EndTurn {
                    message: history::from_engine(&message),
                    usage,
                }
            }
        }
        Err(error) => QueryOutcome::Error(ClaudeError::Other(error.message)),
    }
}

fn invocation(config: &QueryConfig) -> Arc<Invocation> {
    Arc::new(Invocation {
        turn: sailry_protocol::QueuedTurn {
            id: TurnId::new(),
            kind: RunKind::Task,
            session: SessionId::new(),
            request: RequestId::new(),
            revision: 0,
            config: sailry_protocol::SessionConfig {
                assistant: None,
                resource: None,
                provider: ProviderId::new(),
                model: config.model.clone(),
                effort: engine_effort(config.effort_level),
                features: Default::default(),
                mode: WorkMode::Code,
                options: Default::default(),
                permission: Permission::Project,
                credential: None,
            },
            roles: Default::default(),
            plugins: Vec::new(),
        },
        compaction_percent: 80,
        child: None,
    })
}

fn engine_effort(effort: Option<agent_core::effort::EffortLevel>) -> Effort {
    use agent_core::effort::EffortLevel;
    match effort {
        None => Effort::Default,
        Some(EffortLevel::None) => Effort::Disabled,
        Some(EffortLevel::Minimal) => Effort::Minimal,
        Some(EffortLevel::Low) => Effort::Low,
        Some(EffortLevel::Medium) => Effort::Medium,
        Some(EffortLevel::High) => Effort::High,
        Some(EffortLevel::XHigh) => Effort::XHigh,
        Some(EffortLevel::Max | EffortLevel::Ultracode) => Effort::Max,
    }
}

fn usage_current(usage: &core::Usage) -> UsageInfo {
    UsageInfo {
        input_tokens: usage.input.unwrap_or(0),
        output_tokens: usage.output.unwrap_or(0),
        cache_creation_input_tokens: usage.cache_write.unwrap_or(0),
        cache_read_input_tokens: usage.cache_read.unwrap_or(0),
    }
}

fn error_current(error: core::Error) -> ClaudeError {
    match error.kind {
        core::ErrorKind::Authentication => ClaudeError::Auth(error.message),
        core::ErrorKind::ContextOverflow => ClaudeError::ContextWindowExceeded,
        core::ErrorKind::RateLimited => ClaudeError::RateLimit,
        core::ErrorKind::Cancelled => ClaudeError::Cancelled,
        _ => ClaudeError::Other(error.message),
    }
}

fn forward_message(events: &mpsc::UnboundedSender<QueryEvent>, message: core::Message) {
    for content in message.content {
        let delta = match content {
            core::Content::Text { text } => {
                Some(agent_core_api::streaming::ContentDelta::TextDelta { text })
            }
            core::Content::Thinking { text, .. } => {
                Some(agent_core_api::streaming::ContentDelta::ThinkingDelta { thinking: text })
            }
            _ => None,
        };
        if let Some(delta) = delta {
            let _ = events.send(QueryEvent::Stream(
                agent_core_api::AnthropicStreamEvent::ContentBlockDelta { index: 0, delta },
            ));
        }
    }
}

struct Forwarder {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

fn forward_commands(
    queue: crate::CommandQueue,
    steering: engine::SteerQueue,
    cancel: CancellationToken,
) -> Forwarder {
    let local = CancellationToken::new();
    let task_cancel = local.clone();
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                _ = task_cancel.cancelled() => break,
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    for message in crate::drain_command_queue(&queue) {
                        let text = match message.content {
                            MessageContent::Text(text) => text,
                            MessageContent::Blocks(blocks) => blocks.into_iter().filter_map(|block| match block {
                                agent_core::types::ContentBlock::Text { text } => Some(text),
                                _ => None,
                            }).collect::<Vec<_>>().join("\n"),
                        };
                        if !text.is_empty() {
                            steering.push(text);
                        }
                    }
                }
            }
        }
    });
    Forwarder {
        cancel: local,
        task,
    }
}
