//! Fork addition (Waku): the loop end to end — a scripted model, every tool
//! real, the workspace checked from outside.
//!
//! The unit tests pin each piece; these pin what the pieces do together. A
//! model's calls run against real files in a temporary workspace, and each
//! test reads the files back rather than trusting the transcript — the
//! "verify the world, not the self-report" rule of DeepSeek Harness's test
//! tiers. Only the model is a double.

use super::*;
use agent_core_api::provider_types::StopReason;
use agent_core_api::{ProviderError, StreamEvent};
use serde_json::json;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex as StdMutex;

type Step = Vec<Result<StreamEvent, ProviderError>>;

/// A model that answers each request with the next step of a script.
struct Script {
    id: agent_core::provider_id::ProviderId,
    steps: StdMutex<VecDeque<Step>>,
}

#[async_trait::async_trait]
impl agent_core_api::LlmProvider for Script {
    fn id(&self) -> &agent_core::provider_id::ProviderId {
        &self.id
    }
    fn name(&self) -> &str {
        "e2e-script"
    }
    async fn create_message(
        &self,
        _request: agent_core_api::ProviderRequest,
    ) -> Result<agent_core_api::ProviderResponse, ProviderError> {
        unimplemented!("the loop streams")
    }
    async fn create_message_stream(
        &self,
        _request: agent_core_api::ProviderRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("the loop sent more requests than the script has");
        Ok(Box::pin(futures::stream::iter(step)))
    }
    async fn health_check(&self) -> Result<agent_core_api::ProviderStatus, ProviderError> {
        Ok(agent_core_api::ProviderStatus::Healthy)
    }
    fn capabilities(&self) -> agent_core_api::ProviderCapabilities {
        agent_core_api::ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: false,
            image_input: false,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: false,
            system_prompt_style: agent_core_api::SystemPromptStyle::TopLevel,
        }
    }
}

struct AllowAll;

impl agent_core::permissions::PermissionHandler for AllowAll {
    fn check_permission(
        &self,
        _request: &agent_core::permissions::PermissionRequest,
    ) -> agent_core::permissions::PermissionDecision {
        agent_core::permissions::PermissionDecision::Allow
    }
    fn request_permission(
        &self,
        request: &agent_core::permissions::PermissionRequest,
    ) -> agent_core::permissions::PermissionDecision {
        self.check_permission(request)
    }
}

fn opening() -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::MessageStart {
        id: uuid::Uuid::new_v4().to_string(),
        model: "mock-model".to_string(),
        usage: UsageInfo::default(),
    })
}

/// The model calls one tool.
fn call(name: &str, input: Value) -> Step {
    vec![
        opening(),
        Ok(StreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ToolUse {
                id: format!("call-{}", uuid::Uuid::new_v4()),
                name: name.to_string(),
                input: json!({}),
                thought_signature: None,
            },
        }),
        Ok(StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: input.to_string(),
        }),
        Ok(StreamEvent::ContentBlockStop { index: 0 }),
        Ok(StreamEvent::MessageDelta {
            stop_reason: Some(StopReason::ToolUse),
            usage: Some(UsageInfo::default()),
        }),
        Ok(StreamEvent::MessageStop),
    ]
}

/// The model answers and ends its turn.
fn answer(text: &str) -> Step {
    vec![
        opening(),
        Ok(StreamEvent::TextDelta {
            index: 0,
            text: text.to_string(),
        }),
        Ok(StreamEvent::MessageDelta {
            stop_reason: Some(StopReason::EndTurn),
            usage: Some(UsageInfo::default()),
        }),
        Ok(StreamEvent::MessageStop),
    ]
}

struct Run {
    outcome: QueryOutcome,
    messages: Vec<Message>,
    checkpoints: usize,
    checkpoint_messages: Vec<Vec<Message>>,
}

/// Run the loop over `steps` in `workspace`, with every real tool.
async fn run(workspace: &Path, session: &str, steps: Vec<Step>) -> Run {
    run_with_engine(workspace, session, steps, false).await
}

async fn run_core(workspace: &Path, session: &str, steps: Vec<Step>) -> Run {
    run_with_engine(workspace, session, steps, true).await
}

async fn run_with_engine(workspace: &Path, session: &str, steps: Vec<Step>, core: bool) -> Run {
    let mut registry = agent_core_api::ProviderRegistry::new();
    registry.register(Arc::new(Script {
        id: agent_core::provider_id::ProviderId::new("scripted"),
        steps: StdMutex::new(steps.into()),
    }));
    let client = agent_core_api::AnthropicClient::new(agent_core_api::client::ClientConfig {
        api_key: "test-key".to_string(),
        ..Default::default()
    })
    .expect("build test client");
    let mut settings = agent_core::config::Config::default();
    settings.provider = Some("scripted".to_string());
    let ctx = ToolContext {
        working_dir: workspace.to_path_buf(),
        permission_mode: agent_core::config::PermissionMode::Default,
        permission_handler: Arc::new(AllowAll),
        cost_tracker: agent_core::cost::CostTracker::new(),
        session_id: session.to_string(),
        file_history: Arc::new(parking_lot::Mutex::new(
            agent_core::file_history::FileHistory::new(),
        )),
        current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: settings,
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    };
    let config = QueryConfig {
        model: "mock-model".to_string(),
        max_turns: 20,
        provider_registry: Some(Arc::new(registry)),
        checkpoint_history: true,
        ..QueryConfig::default()
    };
    let tools = agent_core_tools::all_tools();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut messages = vec![Message::user("go")];
    let outcome = if core {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            run_agent_core_loop(
                &client,
                &mut messages,
                Arc::new(tools),
                &ctx,
                &config,
                agent_core::cost::CostTracker::new(),
                Some(tx),
                tokio_util::sync::CancellationToken::new(),
                None,
            ),
        )
        .await
    } else {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            run_query_loop(
                &client,
                &mut messages,
                &tools,
                &ctx,
                &config,
                agent_core::cost::CostTracker::new(),
                Some(tx),
                tokio_util::sync::CancellationToken::new(),
                None,
            ),
        )
        .await
    }
    .expect("the loop must not hang");
    let mut checkpoints = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let QueryEvent::Checkpoint(messages) = event {
            checkpoints.push(messages);
        }
    }
    Run {
        outcome,
        messages,
        checkpoints: checkpoints.len(),
        checkpoint_messages: checkpoints,
    }
}

/// Every tool result in the conversation, in order: its text and whether it
/// is an error.
fn results(messages: &[Message]) -> Vec<(String, bool)> {
    messages
        .iter()
        .filter_map(|message| match &message.content {
            agent_core::types::MessageContent::Blocks(blocks) => Some(blocks),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                content: ToolResultContent::Text(text),
                is_error,
                ..
            } => Some((text.clone(), is_error.unwrap_or(false))),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn sailry_core_executes_real_tools_and_checkpoints() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("created.txt");
    let run = run_core(
        workspace.path(),
        "e2e-sailry-core",
        vec![
            call(
                "Write",
                json!({"file_path": path, "content": "from sailry\n"}),
            ),
            answer("Done."),
        ],
    )
    .await;
    assert!(
        matches!(&run.outcome, QueryOutcome::EndTurn { .. }),
        "{:?}",
        run.outcome
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "from sailry\n");
    assert!(run.checkpoints > 0);
}

#[tokio::test]
async fn the_opening_prompt_is_checkpointed_before_any_model_answer() {
    let workspace = tempfile::tempdir().unwrap();
    let run = run(
        workspace.path(),
        "e2e-opening-checkpoint",
        vec![answer("Done.")],
    )
    .await;
    assert!(matches!(run.outcome, QueryOutcome::EndTurn { .. }));
    assert_eq!(run.checkpoints, 1);
    let opening = &run.checkpoint_messages[0];
    assert_eq!(opening.len(), 1);
    assert_eq!(opening[0].role, agent_core::types::Role::User);
    assert_eq!(opening[0].get_all_text(), "go");
}

#[tokio::test]
async fn a_file_is_written_then_edited_on_disk() {
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("notes.txt");
    let path = file.to_string_lossy().to_string();
    let run = run(
        workspace.path(),
        "e2e-write-edit",
        vec![
            call("Write", json!({"file_path": path, "content": "alpha\n"})),
            // The agent's own write counts as read: no Read needed.
            call(
                "Edit",
                json!({"file_path": path, "old_string": "alpha", "new_string": "beta"}),
            ),
            answer("Done."),
        ],
    )
    .await;

    assert!(
        matches!(run.outcome, QueryOutcome::EndTurn { .. }),
        "{:?}",
        run.outcome
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\n");
    let results = results(&run.messages);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(_, error)| !error), "{results:?}");
    // The opening prompt, then before each step's calls and after their results.
    assert_eq!(run.checkpoints, 5);
}

#[tokio::test]
async fn an_unread_file_is_left_alone_until_it_is_read() {
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("config.toml");
    std::fs::write(&file, "mode = \"one\"\n").unwrap();
    let path = file.to_string_lossy().to_string();
    let edit = json!({"file_path": path, "old_string": "one", "new_string": "two"});
    let run = run(
        workspace.path(),
        "e2e-read-before-edit",
        vec![
            call("Edit", edit.clone()),
            call("Read", json!({"file_path": path})),
            call("Edit", edit),
            answer("Changed."),
        ],
    )
    .await;

    assert!(
        matches!(run.outcome, QueryOutcome::EndTurn { .. }),
        "{:?}",
        run.outcome
    );
    let results = results(&run.messages);
    assert_eq!(results.len(), 3);
    assert!(
        results[0].1 && results[0].0.contains("has not been read"),
        "{results:?}"
    );
    assert!(!results[1].1 && !results[2].1, "{results:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "mode = \"two\"\n");
}

#[tokio::test]
async fn the_same_call_a_third_time_draws_a_reminder() {
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("log.txt");
    std::fs::write(&file, "nothing new\n").unwrap();
    let read = json!({"file_path": file.to_string_lossy()});
    let run = run(
        workspace.path(),
        "e2e-repeat",
        vec![
            call("Read", read.clone()),
            call("Read", read.clone()),
            call("Read", read),
            answer("Still nothing."),
        ],
    )
    .await;

    let results = results(&run.messages);
    assert_eq!(results.len(), 3);
    assert!(!results[0].0.contains("<system-reminder>"));
    assert!(!results[1].0.contains("<system-reminder>"));
    assert!(
        results[2].0.contains("<system-reminder>"),
        "{}",
        results[2].0
    );
    // The file itself is untouched.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "nothing new\n");
}
