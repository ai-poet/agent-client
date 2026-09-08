//! What a running session reports, and how the engine's stream becomes it.
//!
//! These events are deliberately *not* `DriverEvent`. The engine emits raw
//! Anthropic stream frames and JSON tool payloads; turning those into the
//! transcript's `ActivityItem`s needs `waku-core`'s activity normalizer, which
//! this crate cannot reach without closing a dependency cycle. So the seam is
//! here: the bridge decodes the stream and hands over typed, still-raw pieces,
//! and `waku-core/src/driver/native.rs` does the presentation half.

use serde_json::Value;

/// One thing that happened inside a session.
#[derive(Clone, Debug)]
pub enum AgentEvent {
    /// A prompt was accepted and the first API call is going out.
    TurnStarted,
    /// Assistant text.
    Text(String),
    /// Extended-thinking text. Kept separate all the way to the transcript.
    Reasoning(String),
    /// A tool call is about to run.
    ToolStarted {
        id: String,
        name: String,
        input: Value,
    },
    /// A tool call finished. `failed` is the engine's own verdict, not an
    /// inference from the output text.
    ToolFinished {
        id: String,
        name: String,
        output: Value,
        failed: bool,
    },
    /// Context-window occupancy after a turn settled.
    Usage {
        context_tokens: Option<u64>,
        context_window: Option<u64>,
    },
    /// The engine wants a decision before running a tool. Answered with
    /// [`crate::AgentSession::respond`].
    Permission {
        request_id: String,
        tool_name: String,
        title: String,
        detail: String,
        options: Vec<PermissionChoice>,
    },
    /// A steering message reached the conversation.
    SteerAccepted { message: String },
    /// A steering message could not be delivered — the turn ended first.
    SteerRejected { message: String, reason: String },
    /// The conversation after a turn settled, serialized for persistence.
    /// Emitted once per turn, after `TurnFinished`, so a crash between turns
    /// costs at most the turn that was running.
    HistoryCommitted(Vec<u8>),
    /// Something went wrong. A turn may still settle afterwards.
    Error(String),
    /// The turn is over. `success` is false for cancellation, an unrecoverable
    /// error, or a spend cap — `summary` carries the reason in those cases.
    TurnFinished {
        success: bool,
        summary: Option<String>,
    },
}

/// One answer offered on a [`AgentEvent::Permission`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionChoice {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl PermissionChoice {
    /// Stable id used on the wire and in `respond`.
    pub fn id(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
            Self::RejectOnce => "reject_once",
            Self::RejectAlways => "reject_always",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "allow_once" => Some(Self::AllowOnce),
            "allow_always" => Some(Self::AllowAlways),
            "reject_once" => Some(Self::RejectOnce),
            "reject_always" => Some(Self::RejectAlways),
            _ => None,
        }
    }

    pub fn is_allow(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }
}

/// Where a session delivers its events.
///
/// A newtype rather than a bare `Arc<dyn Fn>` for one practical reason: an
/// `Arc` is not itself callable, so every call site would have to spell out
/// the deref. `emit` does it once.
///
/// Called from engine worker threads, so an implementation must be cheap and
/// must not block — the transcript is updated by whoever receives this, not
/// here.
#[derive(Clone)]
pub struct EventSink(std::sync::Arc<dyn Fn(AgentEvent) + Send + Sync>);

impl EventSink {
    pub fn new(deliver: impl Fn(AgentEvent) + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(deliver))
    }

    pub fn emit(&self, event: AgentEvent) {
        (*self.0)(event);
    }
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

/// Incremental decoder turning the engine's `QueryEvent`s into [`AgentEvent`]s.
///
/// One per turn, so it needs no reset. Stateful for two reasons: the engine
/// reports a tool *start* with the tool name and an *end* that may carry only
/// an id, and the transcript needs the name on both; and a single prompt
/// produces many `message_start` frames as tools are called, of which only the
/// first is the user's turn beginning.
#[derive(Default)]
pub struct StreamDecoder {
    tool_names: std::collections::HashMap<String, String>,
    /// Set once per prompt so `TurnStarted` is reported for the user's turn
    /// rather than for each of the model's internal steps — a single prompt
    /// can produce many `message_start` frames as tools are called.
    turn_open: bool,
}

impl StreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate one engine event. Returns the events to forward, in order.
    pub fn push(&mut self, event: claurst_query::QueryEvent) -> Vec<AgentEvent> {
        use claurst_api::AnthropicStreamEvent as Frame;
        use claurst_query::QueryEvent as Q;

        match event {
            Q::Stream(frame) => match frame {
                Frame::MessageStart { .. } => {
                    if self.turn_open {
                        Vec::new()
                    } else {
                        self.turn_open = true;
                        vec![AgentEvent::TurnStarted]
                    }
                }
                Frame::ContentBlockDelta { delta, .. } => decode_delta(delta),
                _ => Vec::new(),
            },
            Q::ToolStart {
                tool_name,
                tool_id,
                input_json,
            } => {
                self.tool_names.insert(tool_id.clone(), tool_name.clone());
                // The engine hands the arguments over as the raw JSON text it
                // sent to the model. Parsing failure is not fatal — the
                // transcript can still show the call, just without a
                // structured argument view.
                let input = serde_json::from_str(&input_json).unwrap_or(Value::Null);
                vec![AgentEvent::ToolStarted {
                    id: tool_id,
                    name: tool_name,
                    input,
                }]
            }
            Q::ToolEnd {
                tool_name,
                tool_id,
                result,
                is_error,
            } => {
                let name = self
                    .tool_names
                    .remove(&tool_id)
                    .unwrap_or(tool_name);
                // Tool results are text by contract, but tools that return
                // JSON produce a far better detail view when it is kept
                // structured rather than shown as an escaped string.
                let output = serde_json::from_str(&result)
                    .unwrap_or_else(|_| Value::String(result));
                vec![AgentEvent::ToolFinished {
                    id: tool_id,
                    name,
                    output,
                    failed: is_error,
                }]
            }
            Q::TurnComplete { usage, .. } => {
                let context_tokens = usage.as_ref().map(|usage| {
                    (usage.total_input() as u64).saturating_add(usage.output_tokens as u64)
                });
                vec![AgentEvent::Usage {
                    context_tokens,
                    context_window: None,
                }]
            }
            Q::TokenWarning { .. } => Vec::new(),
            Q::Status(_) => {
                // The engine's status line is TUI furniture ("Thinking…",
                // spinner verbs). It is not assistant content and must never
                // reach the transcript.
                Vec::new()
            }
            Q::Error(message) => vec![AgentEvent::Error(message)],
        }
    }
}

fn decode_delta(delta: claurst_api::streaming::ContentDelta) -> Vec<AgentEvent> {
    use claurst_api::streaming::ContentDelta as D;
    match delta {
        D::TextDelta { text } => vec![AgentEvent::Text(text)],
        D::ThinkingDelta { thinking } => vec![AgentEvent::Reasoning(thinking)],
        // `input_json_delta` streams a tool call's arguments as they are
        // generated. The complete arguments arrive with `ToolStart`, so
        // forwarding the partial JSON would only produce a flickering,
        // unparseable argument view.
        _ => Vec::new(),
    }
}
