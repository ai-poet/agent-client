//! Fork addition (Waku): the loop end to end — a scripted model, every tool
//! real, the workspace checked from outside.
//!
//! The unit tests pin each piece; these pin what the pieces do together. A
//! model's calls run against real files in a temporary workspace, and each
//! test reads the files back rather than trusting the transcript — the
//! "verify the world, not the self-report" rule of DeepSeek Harness's test
//! tiers. Only the model is a double.

use super::*;
use claurst_api::provider_types::StopReason;
use claurst_api::{ProviderError, StreamEvent};
use serde_json::json;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex as StdMutex;

type Step = Vec<Result<StreamEvent, ProviderError>>;

/// A model that answers each request with the next step of a script.
struct Script {
    id: claurst_core::provider_id::ProviderId,
    steps: StdMutex<VecDeque<Step>>,
}

#[async_trait::async_trait]
impl claurst_api::LlmProvider for Script {
    fn id(&self) -> &claurst_core::provider_id::ProviderId {
        &self.id
    }
    fn name(&self) -> &str {
        "e2e-script"
    }
    async fn create_message(
        &self,
        _request: claurst_api::ProviderRequest,
    ) -> Result<claurst_api::ProviderResponse, ProviderError> {
        unimplemented!("the loop streams")
    }
    async fn create_message_stream(
        &self,
        _request: claurst_api::ProviderRequest,
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
    async fn health_check(&self) -> Result<claurst_api::ProviderStatus, ProviderError> {
        Ok(claurst_api::ProviderStatus::Healthy)
    }
    fn capabilities(&self) -> claurst_api::ProviderCapabilities {
        claurst_api::ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: false,
            image_input: false,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: false,
            system_prompt_style: claurst_api::SystemPromptStyle::TopLevel,
        }
    }
}

struct AllowAll;

impl claurst_core::permissions::PermissionHandler for AllowAll {
    fn check_permission(
        &self,
        _request: &claurst_core::permissions::PermissionRequest,
    ) -> claurst_core::permissions::PermissionDecision {
        claurst_core::permissions::PermissionDecision::Allow
    }
    fn request_permission(
        &self,
        request: &claurst_core::permissions::PermissionRequest,
    ) -> claurst_core::permissions::PermissionDecision {
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
}

/// Run the loop over `steps` in `workspace`, with every real tool.
async fn run(workspace: &Path, session: &str, steps: Vec<Step>) -> Run {
    let mut registry = claurst_api::ProviderRegistry::new();
    registry.register(Arc::new(Script {
        id: claurst_core::provider_id::ProviderId::new("scripted"),
        steps: StdMutex::new(steps.into()),
    }));
    let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
        api_key: "test-key".to_string(),
        ..Default::default()
    })
    .expect("build test client");
    let mut settings = claurst_core::config::Config::default();
    settings.provider = Some("scripted".to_string());
    let ctx = ToolContext {
        working_dir: workspace.to_path_buf(),
        permission_mode: claurst_core::config::PermissionMode::Default,
        permission_handler: Arc::new(AllowAll),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: session.to_string(),
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
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
    let tools = claurst_tools::all_tools();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut messages = vec![Message::user("go")];
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_query_loop(
            &client,
            &mut messages,
            &tools,
            &ctx,
            &config,
            claurst_core::cost::CostTracker::new(),
            Some(tx),
            tokio_util::sync::CancellationToken::new(),
            None,
        ),
    )
    .await
    .expect("the loop must not hang");
    let mut checkpoints = 0;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, QueryEvent::Checkpoint(_)) {
            checkpoints += 1;
        }
    }
    Run {
        outcome,
        messages,
        checkpoints,
    }
}

/// Every tool result in the conversation, in order: its text and whether it
/// is an error.
fn results(messages: &[Message]) -> Vec<(String, bool)> {
    messages
        .iter()
        .filter_map(|message| match &message.content {
            claurst_core::types::MessageContent::Blocks(blocks) => Some(blocks),
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
    // Before each step's calls and after their results.
    assert_eq!(run.checkpoints, 4);
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
