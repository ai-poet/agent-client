//! What a running session reports, and how the engine's stream becomes it.
//!
//! These events are deliberately *not* `DriverEvent`. The engine emits raw
//! Anthropic stream frames and JSON tool payloads; turning those into the
//! transcript's `ActivityItem`s needs `waku-core`'s activity normalizer, which
//! this crate cannot reach without closing a dependency cycle. So the seam is
//! here: the bridge decodes the stream and hands over typed, still-raw pieces,
//! and `waku-core/src/driver/native.rs` does the presentation half.

use serde_json::Value;

use crate::background::BackgroundEntry;

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
        /// Images the tool produced, kept apart from `output` because that
        /// text also goes back to the model and pixels there would flood
        /// the context. Shaped for `activity::collect_image_urls`.
        image_source: Option<Value>,
    },
    /// The model entered or left plan mode through the EnterPlanMode /
    /// ExitPlanMode tools. Carried as an event (not folded into the tool
    /// row) because both the engine's permission policy and the client's
    /// mode badge have to move with it.
    PlanModeChanged(bool),
    /// Context-window occupancy after a model step settled.
    Usage {
        context_tokens: Option<u64>,
        context_window: Option<u64>,
    },
    /// The engine is compacting the conversation, or has.
    Compaction {
        phase: CompactionPhase,
        /// `false` when the person asked for it (`/compact`).
        automatic: bool,
        tokens_before: u64,
        /// Estimated, once `phase` is `Finished`.
        tokens_after: Option<u64>,
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
    /// The model asked the user something through `AskUserQuestion` and the
    /// turn is parked until [`crate::AgentSession::answer`] arrives. Unlike a
    /// permission this is never auto-answered: the content has to come from
    /// the user.
    UserInput {
        request_id: String,
        question: String,
        /// Predefined choices, when the model offered any. Free text is
        /// always acceptable too.
        options: Vec<String>,
    },
    /// Everything the engine's background registry holds — a level signal,
    /// sent on demand and after each turn. Stop failures are not an event:
    /// [`crate::AgentSession::stop_background_work`] answers synchronously,
    /// because only the caller holds the key the panel filed the entry under.
    BackgroundWork(Vec<BackgroundEntry>),
    /// A steering message reached the conversation.
    SteerAccepted { message: String },
    /// A steering message could not be delivered — the turn ended first.
    SteerRejected { message: String, reason: String },
    /// The conversation after a turn settled, serialized for persistence.
    /// Emitted once per turn, after `TurnFinished`, so a crash between turns
    /// costs at most the turn that was running.
    HistoryCommitted(Vec<u8>),
    /// The turn ended cleanly and said nothing.
    ///
    /// Structured rather than a sentence because the desktop writes this one
    /// in the user's language, and because the route is the only handle the
    /// user has on a failure that happened upstream of the engine.
    ProducedNothing {
        /// The engine provider the wire format selected.
        provider: String,
        model: String,
        /// Where the request went.
        api_base: String,
    },
    /// Something went wrong. A turn may still settle afterwards.
    Error(String),
    /// The turn is over. `success` is false for cancellation, an unrecoverable
    /// error, or a spend cap — `summary` carries the reason in those cases.
    TurnFinished {
        success: bool,
        summary: Option<String>,
    },
}

/// Where a compaction is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompactionPhase {
    Started,
    Finished,
    Failed,
}

impl From<claurst_query::CompactionPhase> for CompactionPhase {
    fn from(phase: claurst_query::CompactionPhase) -> Self {
        match phase {
            claurst_query::CompactionPhase::Started => Self::Started,
            claurst_query::CompactionPhase::Finished => Self::Finished,
            claurst_query::CompactionPhase::Failed => Self::Failed,
        }
    }
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
pub struct StreamDecoder {
    tool_names: std::collections::HashMap<String, String>,
    turn_open: bool,
    /// The window of the model this turn runs on, so the context gauge has
    /// a denominator. The engine reports occupancy but not capacity.
    context_window: Option<u64>,
    /// Set by a finished compaction until the next request goes out. The
    /// Messages branch reports the step's usage after compacting, and that
    /// figure describes the conversation before it shrank.
    compacted: bool,
}

impl StreamDecoder {
    pub fn new(context_window: Option<u64>) -> Self {
        Self {
            tool_names: std::collections::HashMap::new(),
            turn_open: false,
            context_window,
            compacted: false,
        }
    }

    /// Translate one engine event. Returns the events to forward, in order.
    pub fn push(&mut self, event: claurst_query::QueryEvent) -> Vec<AgentEvent> {
        use claurst_api::AnthropicStreamEvent as Frame;
        use claurst_query::QueryEvent as Q;

        match event {
            Q::Stream(frame) => match frame {
                Frame::MessageStart { .. } => {
                    self.compacted = false;
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
                metadata,
            } => {
                let name = self.tool_names.remove(&tool_id).unwrap_or(tool_name);
                // Tool results are text by contract, but tools that return
                // JSON produce a far better detail view when it is kept
                // structured rather than shown as an escaped string.
                let output =
                    serde_json::from_str(&result).unwrap_or_else(|_| Value::String(result));
                // A metadata document with a `content` array is the image
                // sideband (`mcp_tool::image_metadata`). The plan-mode
                // sideband below carries a `type` and no `content`, so the
                // two never collide.
                let image_source = metadata
                    .as_ref()
                    .filter(|m| m.get("content").is_some_and(Value::is_array))
                    .cloned();
                let mut events = vec![AgentEvent::ToolFinished {
                    id: tool_id,
                    name,
                    output,
                    failed: is_error,
                    image_source,
                }];
                // EnterPlanMode / ExitPlanMode report the switch through
                // their metadata sideband; a failed call changed nothing.
                if !is_error {
                    match metadata
                        .as_ref()
                        .and_then(|m| m.get("type"))
                        .and_then(Value::as_str)
                    {
                        Some("enter_plan_mode") => events.push(AgentEvent::PlanModeChanged(true)),
                        Some("exit_plan_mode") => events.push(AgentEvent::PlanModeChanged(false)),
                        _ => {}
                    }
                }
                events
            }
            Q::TurnComplete { .. } if self.compacted => Vec::new(),
            Q::TurnComplete { usage, .. } => {
                let context_tokens = usage.as_ref().map(|usage| {
                    (usage.total_input() as u64).saturating_add(usage.output_tokens as u64)
                });
                vec![AgentEvent::Usage {
                    context_tokens,
                    context_window: self.context_window,
                }]
            }
            Q::TokenWarning { .. } => Vec::new(),
            Q::Compaction {
                phase,
                automatic,
                tokens_before,
                tokens_after,
            } => {
                let phase = CompactionPhase::from(phase);
                let mut events = vec![AgentEvent::Compaction {
                    phase,
                    automatic,
                    tokens_before,
                    tokens_after,
                }];
                if phase == CompactionPhase::Finished {
                    self.compacted = true;
                    events.push(AgentEvent::Usage {
                        context_tokens: tokens_after,
                        context_window: self.context_window,
                    });
                }
                events
            }
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

#[cfg(test)]
mod tests {

    #[test]
    fn an_image_sideband_rides_on_tool_finished() {
        let mut decoder = StreamDecoder::new(None);
        let meta = serde_json::json!({
            "content": [{"type": "image", "mime": "image/png", "data": "aGVsbG8="}]
        });
        let events = decoder.push(tool_end("waku_js_repl_generate_image", false, Some(meta.clone())));
        let AgentEvent::ToolFinished { image_source, .. } = &events[0] else {
            panic!("expected ToolFinished");
        };
        assert_eq!(image_source.as_ref(), Some(&meta));
    }

    /// The plan-mode sideband has a `type` and no `content`, so it must not
    /// be mistaken for images.
    #[test]
    fn the_plan_mode_sideband_is_not_an_image() {
        let mut decoder = StreamDecoder::new(None);
        let events = decoder.push(tool_end(
            "ExitPlanMode",
            false,
            Some(serde_json::json!({ "type": "exit_plan_mode" })),
        ));
        let AgentEvent::ToolFinished { image_source, .. } = &events[0] else {
            panic!("expected ToolFinished");
        };
        assert!(image_source.is_none());
    }
    use super::*;
    use claurst_query::QueryEvent;

    fn tool_end(name: &str, is_error: bool, metadata: Option<Value>) -> QueryEvent {
        QueryEvent::ToolEnd {
            tool_name: name.to_string(),
            tool_id: format!("id-{name}"),
            result: String::new(),
            is_error,
            metadata,
        }
    }

    #[test]
    fn plan_mode_tools_surface_a_mode_change_event() {
        let mut decoder = StreamDecoder::new(None);

        let events = decoder.push(tool_end(
            "EnterPlanMode",
            false,
            Some(serde_json::json!({ "type": "enter_plan_mode" })),
        ));
        assert!(matches!(events[0], AgentEvent::ToolFinished { .. }));
        assert!(matches!(events[1], AgentEvent::PlanModeChanged(true)));

        let events = decoder.push(tool_end(
            "ExitPlanMode",
            false,
            Some(serde_json::json!({ "type": "exit_plan_mode" })),
        ));
        assert!(matches!(events[1], AgentEvent::PlanModeChanged(false)));
    }

    #[test]
    fn a_failed_plan_mode_tool_changed_nothing() {
        let mut decoder = StreamDecoder::new(None);
        let events = decoder.push(tool_end(
            "EnterPlanMode",
            true,
            Some(serde_json::json!({ "type": "enter_plan_mode" })),
        ));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AgentEvent::ToolFinished { .. }));
    }

    /// The Messages branch reports a step's usage after compacting it away;
    /// the meter shows the compacted size until the next request reports.
    #[test]
    fn the_meter_follows_a_compaction_not_the_usage_reported_after_it() {
        let mut decoder = StreamDecoder::new(Some(200_000));
        let events = decoder.push(QueryEvent::Compaction {
            phase: claurst_query::CompactionPhase::Finished,
            automatic: true,
            tokens_before: 170_000,
            tokens_after: Some(20_000),
        });
        assert!(matches!(
            events[0],
            AgentEvent::Compaction { phase: CompactionPhase::Finished, .. }
        ));
        assert!(matches!(
            events[1],
            AgentEvent::Usage { context_tokens: Some(20_000), .. }
        ));
        let stale = decoder.push(QueryEvent::TurnComplete {
            turn: 3,
            stop_reason: "tool_use".into(),
            usage: Some(claurst_core::types::UsageInfo {
                input_tokens: 170_000,
                ..Default::default()
            }),
        });
        assert!(stale.is_empty());
    }

    #[test]
    fn tools_without_mode_metadata_stay_plain_tool_rows() {
        let mut decoder = StreamDecoder::new(None);
        let events = decoder.push(tool_end("Bash", false, None));
        assert_eq!(events.len(), 1);
    }
}
