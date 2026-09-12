//! One question, one answer, no tools, no session.
//!
//! Waku generates commit messages by asking the session's provider a single
//! prompt. Every CLI answers that with a `--print`-style invocation; the
//! built-in agent has no command line, so this is its equivalent — the same
//! loop a session runs, with an empty tool set and nothing persisted.
//!
//! Runs on the shared runtime and blocks the caller, which is the daemon's
//! workspace thread — never a UI frame.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use claurst_api::AnthropicClient;
use claurst_api::client::ClientConfig;
use claurst_core::types::{ContentBlock, Message, MessageContent};
use claurst_core::{AutoPermissionHandler, CostTracker, PermissionMode};
use claurst_query::QueryOutcome;
use claurst_tools::{Tool, ToolContext};
use tokio_util::sync::CancellationToken;

use crate::config::{AccessMode, AgentStartOptions, build_config, build_query_config};
use crate::runtime;

/// Ask the model `prompt` and return what it said, trimmed.
///
/// `model` overrides the engine's configured default. An empty answer is an
/// error rather than an empty string: a caller writing it into a commit would
/// otherwise commit with no message.
pub fn one_shot(cwd: &Path, model: Option<&str>, prompt: &str) -> anyhow::Result<String> {
    let rt = runtime::shared()?;
    let options = AgentStartOptions {
        cwd: cwd.to_path_buf(),
        // Irrelevant with no tools, but the most restrictive reading is the
        // right default for a call that should never touch anything.
        access_mode: AccessMode::Ask,
        plan_mode: false,
        model: model.map(str::to_owned),
        reasoning_effort: None,
        history: Vec::new(),
    };
    let config = build_config(&options);
    let mut query = build_query_config(&config, &options);
    // One reply is the whole job.
    query.max_turns = 1;

    let (api_key, use_bearer_auth) = rt
        .block_on(config.resolve_anthropic_auth_async())
        .unwrap_or_default();
    let client_config = ClientConfig {
        api_key,
        api_base: config.resolve_anthropic_api_base(),
        use_bearer_auth,
        ..Default::default()
    };
    let client = AnthropicClient::new(client_config.clone())?;
    query.provider_registry = Some(Arc::new(claurst_api::ProviderRegistry::from_config(
        &config,
        client_config,
    )));

    let cost_tracker = CostTracker::new();
    let tool_ctx = ToolContext {
        working_dir: cwd.to_path_buf(),
        permission_mode: PermissionMode::Plan,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Plan,
        }),
        cost_tracker: cost_tracker.clone(),
        session_id: format!("one-shot-{}", uuid::Uuid::new_v4()),
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
        cancel_token: CancellationToken::new(),
    };

    let tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut messages = vec![Message::user(prompt)];
    let outcome = rt.block_on(claurst_query::run_query_loop(
        &client,
        &mut messages,
        &tools,
        &tool_ctx,
        &query,
        cost_tracker,
        None,
        CancellationToken::new(),
        None,
    ));

    let message = match outcome {
        QueryOutcome::EndTurn { message, .. } | QueryOutcome::MaxTokens {
            partial_message: message,
            ..
        } => message,
        QueryOutcome::Cancelled => anyhow::bail!("the request was cancelled"),
        QueryOutcome::BudgetExceeded { .. } => anyhow::bail!("the session's spend cap is reached"),
        QueryOutcome::Error(error) => return Err(error.into()),
    };
    let text = text_of(&message);
    if text.is_empty() {
        anyhow::bail!("the model returned no text");
    }
    Ok(text)
}

/// The visible text of a message — every text block, thinking excluded.
fn text_of(message: &Message) -> String {
    match &message.content {
        MessageContent::Text(text) => text.trim().to_owned(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_blocks_never_reach_the_answer() {
        let message = Message::assistant_blocks(vec![
            ContentBlock::Thinking {
                thinking: "let me consider".into(),
                signature: String::new(),
            },
            ContentBlock::Text {
                text: "  fix: handle empty input  ".into(),
            },
        ]);
        assert_eq!(text_of(&message), "fix: handle empty input");
    }

    #[test]
    fn a_plain_text_message_is_trimmed() {
        assert_eq!(text_of(&Message::assistant("\nhello\n")), "hello");
    }
}
