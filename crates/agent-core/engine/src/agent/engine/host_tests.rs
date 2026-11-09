use super::*;
use crate::agent::catalog::ToolProvider;
use crate::agent::model::{
    Config, Content, Error, Event as ModelEvent, Finish, Message, Model, ModelStream, Request,
    ToolCall, Usage,
};
use crate::store::agent::{Invocation, RuntimeSession, Sessions};
use futures::stream;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Tool};
use sailry_protocol::{
    Effort, Permission, ProviderId, RequestId, SessionId, TurnId, WorkMode, conversation::RunKind,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

struct Script {
    replies: Mutex<VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<Request>>,
}

impl Script {
    fn new(replies: Vec<Vec<ModelEvent>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::default(),
        }
    }
}

#[async_trait::async_trait]
impl Model for Script {
    async fn call(&self, request: Request, _: CancellationToken) -> Result<ModelStream, Error> {
        self.requests.lock().unwrap().push(request);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply");
        Ok(Box::pin(stream::iter(reply.into_iter().map(Ok))))
    }
}

#[derive(Default)]
struct HostTools {
    active: AtomicUsize,
    peak: AtomicUsize,
    cancel: bool,
}

#[async_trait::async_trait]
impl ToolProvider<RuntimeSession> for HostTools {
    async fn tools(&self, _: &RuntimeSession) -> anyhow::Result<Vec<Tool>> {
        Ok(["read-a", "read-b"]
            .map(|name| Tool::new(name, "fixture", serde_json::Map::new()))
            .into())
    }

    async fn call(
        &self,
        _: &RuntimeSession,
        _: &str,
        call: CallToolRequestParams,
        emit: &Emitter,
    ) -> Result<CallToolResult, ErrorData> {
        if self.cancel {
            emit.cancel_token().cancel();
            std::future::pending::<()>().await;
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "done:{}",
            call.name
        ))]))
    }

    fn concurrent(&self, _: &RuntimeSession, _: &CallToolRequestParams) -> bool {
        true
    }
}

fn text(value: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta {
            index: 0,
            text: value.into(),
        },
        ModelEvent::Usage(Usage::default()),
        ModelEvent::Finish(Finish::Stop),
    ]
}

fn tool_calls() -> Vec<ModelEvent> {
    ["a", "b"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| ModelEvent::ToolCall {
            index,
            call: ToolCall {
                id: id.into(),
                name: format!("read-{id}"),
                arguments: serde_json::json!({}),
                metadata: None,
                tool_meta: None,
                error: None,
            },
        })
        .chain([ModelEvent::Finish(Finish::ToolCalls)])
        .collect()
}

fn invocation(kind: RunKind) -> Arc<Invocation> {
    let session = SessionId::new();
    Arc::new(Invocation {
        turn: sailry_protocol::QueuedTurn {
            id: TurnId::new(),
            kind,
            session,
            request: RequestId::new(),
            revision: 0,
            config: sailry_protocol::SessionConfig {
                assistant: None,
                resource: None,
                provider: ProviderId::new(),
                model: "fixture".into(),
                effort: Effort::Default,
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

fn config() -> Config {
    Config {
        model_name: "fixture".into(),
        context_limit: Some(100_000),
        max_tokens: Some(16),
        ..Config::default()
    }
}

fn binding(model: Arc<Script>) -> model::Binding {
    model::Binding {
        provider: model,
        config: config(),
        owner: "fixture".into(),
    }
}

async fn run_with(
    script: Vec<Vec<ModelEvent>>,
    tools: Arc<HostTools>,
    history: Vec<Message>,
    kind: RunKind,
    context_limit: usize,
    steering: SteerQueue,
) -> (RunOutcome, RuntimeSession, Arc<Script>) {
    let invocation = invocation(kind);
    let session = RuntimeSession::new(invocation.turn.session, history);
    let model = Arc::new(Script::new(script));
    let request = Run {
        sessions: Sessions::new(session),
        invocation: invocation.clone(),
        stop: CancellationToken::new(),
        binding: binding(model.clone()),
        summary: binding(model.clone()),
        instruction: "fixture instructions".into(),
        context: String::new(),
        parts: Vec::new(),
        catalog: tools,
        callbacks: Vec::new(),
        deferred: BTreeSet::new(),
        context_limit,
        max_turns: Some(8),
        steering,
    };
    let sessions = request.sessions.clone();
    let events = |_: EngineEvent| {};
    let outcome = run(request, &events).await.unwrap();
    let session = sessions.snapshot().await;
    (outcome, session, model)
}

#[tokio::test(flavor = "current_thread")]
async fn runs_inference_to_a_visible_answer() {
    let (outcome, session, _) = run_with(
        vec![text("done")],
        Arc::new(HostTools::default()),
        vec![Message::user().with_text("work")],
        RunKind::Task,
        100_000,
        SteerQueue::default(),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Yielded);
    assert_eq!(outcome.output.as_deref(), Some("done"));
    assert!(
        session
            .conversation
            .iter()
            .any(|message| message.as_concat_text() == "done")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn pairs_tools_and_runs_read_only_calls_concurrently() {
    let tools = Arc::new(HostTools::default());
    let (outcome, session, _) = run_with(
        vec![tool_calls(), text("done")],
        tools.clone(),
        vec![Message::user().with_text("work")],
        RunKind::Task,
        100_000,
        SteerQueue::default(),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Yielded);
    assert_eq!(tools.peak.load(Ordering::SeqCst), 2);
    let responses = session
        .conversation
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolResult(result) => Some(result.id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(responses, ["a", "b"]);
    for id in ["a", "b"] {
        assert!(
            session
                .conversation
                .iter()
                .any(|message| { message.metadata.operation_note("tool_output", id).is_some() })
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_completes_pending_tool_history() {
    let tools = Arc::new(HostTools {
        cancel: true,
        ..HostTools::default()
    });
    let (outcome, session, _) = run_with(
        vec![tool_calls()],
        tools,
        vec![Message::user().with_text("work")],
        RunKind::Task,
        100_000,
        SteerQueue::default(),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Cancelled);
    let responses = session
        .conversation
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    assert!(responses.iter().all(|result| result.result.is_err()));
}

#[tokio::test(flavor = "current_thread")]
async fn steer_joins_the_next_model_request() {
    let steering = SteerQueue::default();
    steering.push("new constraint");
    let (outcome, _, model) = run_with(
        vec![text("first"), text("second")],
        Arc::new(HostTools::default()),
        vec![Message::user().with_text("work")],
        RunKind::Task,
        100_000,
        steering,
    )
    .await;
    assert_eq!(outcome.output.as_deref(), Some("second"));
    let requests = model.requests.lock().unwrap();
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.as_concat_text() == "new constraint")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compacts_through_the_original_structured_handoff() {
    let summary = vec![
        ModelEvent::TextDelta {
            index: 0,
            text: "```json\n{\"user_intent\":[\"Fix the parser\"]}\n```".into(),
        },
        ModelEvent::Finish(Finish::Stop),
    ];
    let (outcome, session, model) = run_with(
        vec![summary, text("done")],
        Arc::new(HostTools::default()),
        vec![
            Message::user().with_text("older context ".repeat(500)),
            Message::assistant().with_text("older work ".repeat(500)),
            Message::user().with_text("Fix the parser"),
        ],
        RunKind::Task,
        1000,
        SteerQueue::default(),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Yielded);
    assert!(
        session
            .conversation
            .iter()
            .any(|message| message.as_concat_text().contains("Fix the parser"))
    );
    assert!(
        model.requests.lock().unwrap()[1]
            .messages
            .iter()
            .any(|message| message.as_concat_text().contains("Fix the parser"))
    );
}
