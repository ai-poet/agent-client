//! Existing provider registry adapted to Sailry's Model and stream contract.
use agent_core::config::Config as ClientConfig;
use agent_core::types::{ToolDefinition, UsageInfo};
use agent_core_api::provider::LlmProvider;
use agent_core_api::provider_types::{ProviderRequest, StopReason, StreamEvent};
use agent_core_api::{SystemPrompt, ThinkingConfig};
use agent_core_engine::agent::model as engine;
use async_trait::async_trait;
use futures::Stream;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{QueryConfig, QueryEvent};

pub struct ExistingModel {
    provider: Arc<dyn LlmProvider>,
    provider_id: String,
    model_id: String,
    query: QueryConfig,
    session_id: String,
    events: Option<mpsc::UnboundedSender<QueryEvent>>,
}

impl ExistingModel {
    pub fn new(
        query: &QueryConfig,
        config: &ClientConfig,
        session_id: &str,
        events: Option<mpsc::UnboundedSender<QueryEvent>>,
    ) -> Result<Self, engine::Error> {
        let registry = query.provider_registry.as_ref().ok_or_else(|| {
            engine::Error::new(
                engine::ErrorKind::NotConfigured,
                "provider registry is required",
            )
        })?;
        let provider_id = config
            .provider
            .as_deref()
            .unwrap_or("anthropic")
            .to_string();
        let prefix = format!("{provider_id}/");
        let model_id = query
            .model
            .strip_prefix(&prefix)
            .unwrap_or(&query.model)
            .to_string();
        let pid = agent_core::provider_id::ProviderId::new(&provider_id);
        let runtime = agent_core_api::registry::runtime_provider_for(&provider_id);
        let registered = if runtime.is_some() {
            None
        } else {
            registry.get(&pid).cloned()
        };
        let mut provider = runtime.or(registered);
        if agent_core_api::registry::resolve_provider_api_base(config, &provider_id).is_some() {
            if let Some(overridden) =
                agent_core_api::registry::provider_from_config(config, &provider_id)
            {
                provider = Some(overridden);
            }
        }
        let provider = provider.ok_or_else(|| {
            engine::Error::new(
                engine::ErrorKind::NotConfigured,
                format!("provider is not configured: {provider_id}"),
            )
        })?;
        Ok(Self {
            provider,
            provider_id,
            model_id,
            query: query.clone(),
            session_id: session_id.into(),
            events,
        })
    }
}

#[async_trait]
impl engine::Model for ExistingModel {
    async fn call(
        &self,
        request: engine::Request,
        cancel: CancellationToken,
    ) -> Result<engine::ModelStream, engine::Error> {
        let effort = current_effort(request.effort, self.query.effort_level);
        let thinking_budget = self
            .query
            .thinking_budget
            .or_else(|| effort.and_then(|effort| effort.thinking_budget_tokens()));
        let capabilities = self.provider.capabilities();
        let request = ProviderRequest {
            model: request
                .model
                .strip_prefix(&format!("{}/", self.provider_id))
                .unwrap_or(&request.model)
                .to_string(),
            messages: request
                .messages
                .iter()
                .map(super::history::from_engine)
                .collect(),
            system_prompt: Some(SystemPrompt::Text(request.system)),
            tools: request
                .tools
                .into_iter()
                .map(|tool| ToolDefinition {
                    name: tool.name,
                    description: tool.description.unwrap_or_default(),
                    input_schema: tool.parameters,
                })
                .collect(),
            max_tokens: request
                .max_tokens
                .unwrap_or(self.query.max_tokens as u64)
                .min(u32::MAX as u64) as u32,
            temperature: request.temperature,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
            thinking: if capabilities.thinking {
                thinking_budget.map(ThinkingConfig::enabled)
            } else {
                None
            },
            provider_options: crate::runner::provider_options::with_prompt_cache_key(
                crate::runner::provider_options::build_provider_options(
                    &self.provider_id,
                    &self.model_id,
                    effort,
                    thinking_budget,
                ),
                &self.provider_id,
                &self.session_id,
            ),
        };
        let opened = tokio::select! {
            _ = cancel.cancelled() => return Err(engine::Error::new(engine::ErrorKind::Cancelled, "model request was cancelled")),
            opened = self.provider.create_message_stream(request) => opened,
        };
        let stream = opened.map_err(|error| {
            self.failure(agent_core_api::retry_policy::from_provider_error(&error))
        })?;
        Ok(Box::pin(ConvertStream::new(
            stream,
            cancel,
            self.events.clone(),
        )))
    }
}

impl ExistingModel {
    fn failure(&self, failure: agent_core_api::retry_policy::ModelFailure) -> engine::Error {
        if let Some(events) = &self.events {
            let _ = events.send(QueryEvent::RequestFailed(failure.clone()));
        }
        use agent_core_api::retry_policy::{FailureClass, RetryReason};
        let kind = match failure.class {
            FailureClass::ContextOverflow => engine::ErrorKind::ContextOverflow,
            FailureClass::Retryable if failure.reason == RetryReason::RateLimited => {
                engine::ErrorKind::RateLimited
            }
            FailureClass::Retryable => engine::ErrorKind::Network,
            FailureClass::Fatal if matches!(failure.status, Some(401 | 403)) => {
                engine::ErrorKind::Authentication
            }
            FailureClass::Fatal => engine::ErrorKind::Other,
        };
        engine::Error::new(kind, failure.message)
    }
}

fn current_effort(
    effort: sailry_protocol::Effort,
    fallback: Option<agent_core::effort::EffortLevel>,
) -> Option<agent_core::effort::EffortLevel> {
    use agent_core::effort::EffortLevel;
    use sailry_protocol::Effort;
    match effort {
        Effort::Default => fallback,
        Effort::Disabled => Some(EffortLevel::None),
        Effort::Minimal => Some(EffortLevel::Minimal),
        Effort::Low => Some(EffortLevel::Low),
        Effort::Medium => Some(EffortLevel::Medium),
        Effort::High => Some(EffortLevel::High),
        Effort::XHigh => Some(EffortLevel::XHigh),
        Effort::Max => Some(EffortLevel::Max),
        Effort::Budget(_) => fallback,
    }
}

struct PartialTool {
    id: String,
    name: String,
    json: String,
    signature: Option<String>,
}

struct ConvertStream {
    stream: Pin<Box<dyn Stream<Item = Result<StreamEvent, agent_core_api::ProviderError>> + Send>>,
    cancel: CancellationToken,
    events: Option<mpsc::UnboundedSender<QueryEvent>>,
    tools: BTreeMap<usize, PartialTool>,
    pending: VecDeque<Result<engine::Event, engine::Error>>,
    stop: StopReason,
    done: bool,
}

impl ConvertStream {
    fn new(
        stream: Pin<
            Box<dyn Stream<Item = Result<StreamEvent, agent_core_api::ProviderError>> + Send>,
        >,
        cancel: CancellationToken,
        events: Option<mpsc::UnboundedSender<QueryEvent>>,
    ) -> Self {
        Self {
            stream,
            cancel,
            events,
            tools: BTreeMap::new(),
            pending: VecDeque::new(),
            stop: StopReason::EndTurn,
            done: false,
        }
    }

    fn finish(&self) -> engine::Event {
        engine::Event::Finish(match &self.stop {
            StopReason::EndTurn | StopReason::StopSequence => engine::Finish::Stop,
            StopReason::ToolUse => engine::Finish::ToolCalls,
            StopReason::MaxTokens => engine::Finish::Length,
            StopReason::ContentFiltered => engine::Finish::ContentFilter,
            StopReason::Other(reason) => engine::Finish::Other(reason.clone()),
        })
    }
}

impl Stream for ConvertStream {
    type Item = Result<engine::Event, engine::Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return std::task::Poll::Ready(Some(event));
            }
            if self.done {
                return std::task::Poll::Ready(None);
            }
            if self.cancel.is_cancelled() {
                self.done = true;
                return std::task::Poll::Ready(Some(Err(engine::Error::new(
                    engine::ErrorKind::Cancelled,
                    "model request was cancelled",
                ))));
            }
            let event = match self.stream.as_mut().poll_next(context) {
                std::task::Poll::Pending => return std::task::Poll::Pending,
                std::task::Poll::Ready(event) => event,
            };
            let Some(event) = event else {
                self.done = true;
                return std::task::Poll::Ready(Some(Err(engine::Error::new(
                    engine::ErrorKind::Unavailable,
                    "the response stream ended before its terminal event",
                ))));
            };
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    self.done = true;
                    let failure = agent_core_api::retry_policy::from_provider_error(&error);
                    if let Some(events) = &self.events {
                        let _ = events.send(QueryEvent::RequestFailed(failure.clone()));
                    }
                    let kind = match failure.class {
                        agent_core_api::retry_policy::FailureClass::Retryable => {
                            engine::ErrorKind::Network
                        }
                        agent_core_api::retry_policy::FailureClass::ContextOverflow => {
                            engine::ErrorKind::ContextOverflow
                        }
                        agent_core_api::retry_policy::FailureClass::Fatal => {
                            engine::ErrorKind::Other
                        }
                    };
                    return std::task::Poll::Ready(Some(Err(engine::Error::new(
                        kind,
                        failure.message,
                    ))));
                }
            };
            match event {
                StreamEvent::MessageStart { usage, .. } => {
                    self.pending
                        .push_back(Ok(engine::Event::Usage(usage_from_current(&usage))));
                }
                StreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => match content_block {
                    agent_core::types::ContentBlock::ToolUse {
                        id,
                        name,
                        input,
                        thought_signature,
                    } => {
                        self.tools.insert(
                            index,
                            PartialTool {
                                id,
                                name,
                                json: if input.is_null()
                                    || input.as_object().is_some_and(|object| object.is_empty())
                                {
                                    String::new()
                                } else {
                                    input.to_string()
                                },
                                signature: thought_signature,
                            },
                        );
                    }
                    agent_core::types::ContentBlock::Text { text } if !text.is_empty() => {
                        self.pending
                            .push_back(Ok(engine::Event::TextDelta { index, text }));
                    }
                    agent_core::types::ContentBlock::Thinking {
                        thinking,
                        signature,
                    } => {
                        self.pending.push_back(Ok(engine::Event::ReasoningDelta {
                            index,
                            text: (!thinking.is_empty()).then_some(thinking),
                            signature: Some(signature),
                        }));
                    }
                    agent_core::types::ContentBlock::RedactedThinking { data } => {
                        self.pending
                            .push_back(Ok(engine::Event::RedactedThinking { index, data }));
                    }
                    _ => {}
                },
                StreamEvent::TextDelta { index, text } => {
                    self.pending
                        .push_back(Ok(engine::Event::TextDelta { index, text }));
                }
                StreamEvent::ThinkingDelta { index, thinking } => {
                    self.pending.push_back(Ok(engine::Event::ReasoningDelta {
                        index,
                        text: Some(thinking),
                        signature: None,
                    }));
                }
                StreamEvent::ReasoningDelta { index, reasoning } => {
                    self.pending.push_back(Ok(engine::Event::ReasoningDelta {
                        index,
                        text: Some(reasoning),
                        signature: None,
                    }));
                }
                StreamEvent::SignatureDelta { index, signature } => {
                    self.pending.push_back(Ok(engine::Event::ReasoningDelta {
                        index,
                        text: None,
                        signature: Some(signature),
                    }));
                }
                StreamEvent::InputJsonDelta {
                    index,
                    partial_json,
                } => {
                    if let Some(tool) = self.tools.get_mut(&index) {
                        tool.json.push_str(&partial_json);
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    if let Some(tool) = self.tools.remove(&index) {
                        let arguments = if tool.json.trim().is_empty() {
                            Value::Object(Map::new())
                        } else {
                            match serde_json::from_str(&tool.json) {
                                Ok(arguments) => arguments,
                                Err(error) => {
                                    self.done = true;
                                    return std::task::Poll::Ready(Some(Err(engine::Error::new(
                                        engine::ErrorKind::InvalidResponse,
                                        format!("invalid tool arguments: {error}"),
                                    ))));
                                }
                            }
                        };
                        self.pending.push_back(Ok(engine::Event::ToolCall {
                            index,
                            call: engine::ToolCall {
                                id: tool.id,
                                name: tool.name,
                                arguments,
                                metadata: tool.signature.map(|signature| {
                                    Map::from_iter([("thought_signature".into(), signature.into())])
                                }),
                                tool_meta: None,
                                error: None,
                            },
                        }));
                    }
                }
                StreamEvent::MessageDelta { stop_reason, usage } => {
                    if let Some(reason) = stop_reason {
                        self.stop = reason;
                    }
                    if let Some(usage) = usage {
                        self.pending
                            .push_back(Ok(engine::Event::Usage(usage_from_current(&usage))));
                    }
                }
                StreamEvent::MessageStop => {
                    self.done = true;
                    let finish = self.finish();
                    self.pending.push_back(Ok(finish));
                }
                StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    self.done = true;
                    let failure =
                        agent_core_api::retry_policy::classify_stream_error(&error_type, &message);
                    if let Some(events) = &self.events {
                        let _ = events.send(QueryEvent::RequestFailed(failure.clone()));
                    }
                    let kind = match failure.class {
                        agent_core_api::retry_policy::FailureClass::Retryable => {
                            engine::ErrorKind::Network
                        }
                        agent_core_api::retry_policy::FailureClass::ContextOverflow => {
                            engine::ErrorKind::ContextOverflow
                        }
                        agent_core_api::retry_policy::FailureClass::Fatal => {
                            engine::ErrorKind::Other
                        }
                    };
                    self.pending
                        .push_back(Err(engine::Error::new(kind, failure.message)));
                }
            }
        }
    }
}

fn usage_from_current(usage: &UsageInfo) -> engine::Usage {
    engine::Usage {
        input: Some(usage.input_tokens),
        output: Some(usage.output_tokens),
        cache_read: Some(usage.cache_read_input_tokens),
        cache_write: Some(usage.cache_creation_input_tokens),
        total: Some(usage.total()),
        ..engine::Usage::default()
    }
}
