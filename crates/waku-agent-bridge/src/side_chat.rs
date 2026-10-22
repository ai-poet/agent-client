//! A side question about a task, answered while the task goes on.
//!
//! Fork addition, after Claude Desktop's side chat (`/btw`, ⌘;): the person
//! asks something about what the agent is doing without steering it. The
//! desktop sends a digest of the task's transcript and the side thread so
//! far; this answers with the engine's own loop — any of its three wire
//! formats, the model's own gateway key — with no tools, no session id and
//! nothing written anywhere. The answer streams back through `on_text`.
//!
//! Like [`crate::one_shot`] it blocks the caller, which is a daemon thread
//! of its own, never a UI frame.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use claurst_core::types::Message;
use claurst_core::{AutoPermissionHandler, CostTracker, PermissionMode};
use claurst_query::QueryOutcome;
use claurst_tools::{Tool, ToolContext};
use tokio::sync::mpsc;
pub use tokio_util::sync::CancellationToken;

use crate::config::{AccessMode, AgentStartOptions, build_config, build_query_config, split_model};
use crate::events::{AgentEvent, StreamDecoder};
use crate::oneshot::text_of;
use crate::runtime;
use crate::session::build_clients;

/// One message of the side thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideMessage {
    pub assistant: bool,
    pub text: String,
}

/// Answer the side thread's last question. `model` is the picker's id
/// (`platform::model`); `None` takes the engine's default. `system` is laid
/// over the engine's prompt as the session's own instructions would be.
pub fn ask_streaming(
    cwd: &Path,
    model: Option<&str>,
    system: &str,
    thread: &[SideMessage],
    on_text: &mut dyn FnMut(&str),
    cancel: CancellationToken,
) -> anyhow::Result<String> {
    if thread.last().is_none_or(|message| message.assistant) {
        anyhow::bail!("the side thread does not end with a question");
    }
    let rt = runtime::shared()?;
    let (platform, model) = model.map(split_model).map_or((None, None), |(platform, model)| {
        (platform, Some(model))
    });
    let options = AgentStartOptions {
        cwd: cwd.to_path_buf(),
        access_mode: AccessMode::Ask,
        plan_mode: false,
        model,
        platform,
        // The model's family decides, as for a session on it.
        wire_format: None,
        reasoning_effort: None,
        narration_language: None,
        history: Vec::new(),
        computer_use: None,
        session_id: None,
        browser_tools: false,
        trajectory_dir: None,
    };
    let config = build_config(&options)?;
    let mut query = build_query_config(&config, &options);
    query.max_turns = 1;
    query.retry_budget = claurst_api::retry_policy::RetryBudget::Light;
    query.system_prompt = Some(match query.system_prompt.take() {
        Some(house) if !house.trim().is_empty() => format!("{house}\n\n{system}"),
        _ => system.to_owned(),
    });
    let (client, registry) = build_clients(&config, options.platform.as_deref())?;
    query.provider_registry = Some(registry);

    let cost_tracker = CostTracker::new();
    let tool_ctx = ToolContext {
        working_dir: cwd.to_path_buf(),
        permission_mode: PermissionMode::Plan,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Plan,
        }),
        cost_tracker: cost_tracker.clone(),
        session_id: format!("side-chat-{}", uuid::Uuid::new_v4()),
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
        )),
        current_turn: Arc::new(AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        managed_agent_config: None,
        config,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
        cancel_token: cancel.clone(),
    };
    let mut messages = thread_messages(thread);
    let tools: Vec<Box<dyn Tool>> = Vec::new();

    rt.block_on(async {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let run = claurst_query::run_query_loop(
            client.as_ref(),
            &mut messages,
            &tools,
            &tool_ctx,
            &query,
            cost_tracker,
            Some(tx),
            cancel,
            None,
        );
        tokio::pin!(run);
        let mut decoder = StreamDecoder::new(None);
        let outcome = loop {
            tokio::select! {
                outcome = &mut run => break outcome,
                Some(event) = rx.recv() => forward(&mut decoder, event, on_text),
            }
        };
        while let Ok(event) = rx.try_recv() {
            forward(&mut decoder, event, on_text);
        }
        let message = match outcome {
            QueryOutcome::EndTurn { message, .. }
            | QueryOutcome::MaxTokens {
                partial_message: message,
                ..
            } => message,
            QueryOutcome::Cancelled => anyhow::bail!("the request was cancelled"),
            QueryOutcome::BudgetExceeded { .. } => {
                anyhow::bail!("the session's spend cap is reached")
            }
            QueryOutcome::Error(error) => return Err(error.into()),
        };
        let text = text_of(&message);
        if text.is_empty() {
            anyhow::bail!("the model returned no text");
        }
        Ok(text)
    })
}

fn forward(decoder: &mut StreamDecoder, event: claurst_query::QueryEvent, on_text: &mut dyn FnMut(&str)) {
    for event in decoder.push(event) {
        if let AgentEvent::Text(text) = event {
            on_text(&text);
        }
    }
}

/// The thread as the engine's messages: the person's and the model's turns
/// in order.
fn thread_messages(thread: &[SideMessage]) -> Vec<Message> {
    thread
        .iter()
        .map(|message| {
            if message.assistant {
                Message::assistant(message.text.as_str())
            } else {
                Message::user(message.text.as_str())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_that_does_not_end_with_a_question_is_refused() {
        let thread = [SideMessage {
            assistant: true,
            text: "hi".into(),
        }];
        let error = ask_streaming(
            Path::new("."),
            None,
            "",
            &thread,
            &mut |_| {},
            CancellationToken::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not end with a question"));
        assert!(ask_streaming(Path::new("."), None, "", &[], &mut |_| {}, CancellationToken::new()).is_err());
    }

    #[test]
    fn the_thread_keeps_its_order_and_roles() {
        let messages = thread_messages(&[
            SideMessage { assistant: false, text: "q1".into() },
            SideMessage { assistant: true, text: "a1".into() },
            SideMessage { assistant: false, text: "q2".into() },
        ]);
        assert_eq!(messages.len(), 3);
        assert_eq!(text_of(&messages[1]), "a1");
        assert_eq!(messages[2].role, claurst_core::types::Role::User);
    }
}
