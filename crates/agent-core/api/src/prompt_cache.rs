//! Prompt-cache breakpoints on a Messages API request.
//!
//! Fork departure (Waku). Anthropic caches a request's prefix only up to a
//! `cache_control` breakpoint, and this engine never set one — the
//! `CacheControl` type existed, but every place that built a request left it
//! empty. A route that goes straight to Anthropic therefore re-read the whole
//! conversation at full price on every call, while Claude Code, which marks
//! its own requests, was cached on the very same route.
//!
//! The breakpoints follow Claude Code's: the last tool definition, the end of
//! the system prompt, the last message, and the user message before it. Each
//! request's last-message breakpoint is where the next request's
//! previous-user breakpoint lands, so every call reads the conversation up to
//! the turn before it from the cache and writes only what is new. Anthropic
//! allows four breakpoints; ones already on the request count toward that,
//! and a prefix too short to cache is simply not cached — marking is never an
//! error.

use serde_json::{Map, Value};

use crate::types::{CacheControl, CreateMessageRequest, SystemBlock, SystemPrompt};

/// Anthropic rejects a request with more breakpoints than this.
const MAX_BREAKPOINTS: usize = 4;

/// Add Claude Code's breakpoints to `request`, within the limit.
pub fn apply_breakpoints(request: &mut CreateMessageRequest) {
    let mut budget = MAX_BREAKPOINTS.saturating_sub(existing_breakpoints(request));

    // Most valuable first: the conversation, then the system prompt, then the
    // tools — each later one is a prefix of the ones before it.
    for index in message_targets(request) {
        if budget > 0 && mark_message(&mut request.messages[index].content) {
            budget -= 1;
        }
    }
    if budget > 0 && mark_system(&mut request.system) {
        budget -= 1;
    }
    if budget == 0 {
        return;
    }
    if let Some(tool) = request.tools.as_mut().and_then(|tools| tools.last_mut()) {
        if tool.cache_control.is_none() {
            tool.cache_control = Some(CacheControl::ephemeral());
        }
    }
}

/// The last message, and the user message before it.
fn message_targets(request: &CreateMessageRequest) -> Vec<usize> {
    let Some(last) = request.messages.len().checked_sub(1) else {
        return Vec::new();
    };
    let previous_user = request.messages[..last]
        .iter()
        .rposition(|message| message.role == "user");
    std::iter::once(last).chain(previous_user).collect()
}

fn existing_breakpoints(request: &CreateMessageRequest) -> usize {
    let system = match &request.system {
        Some(SystemPrompt::Blocks(blocks)) => blocks
            .iter()
            .filter(|block| block.cache_control.is_some())
            .count(),
        _ => 0,
    };
    let tools = request.tools.as_ref().map_or(0, |tools| {
        tools
            .iter()
            .filter(|tool| tool.cache_control.is_some())
            .count()
    });
    let messages = request
        .messages
        .iter()
        .filter_map(|message| message.content.as_array())
        .flatten()
        .filter(|block| block.get("cache_control").is_some())
        .count();
    system + tools + messages
}

/// Put a breakpoint at the end of the system prompt. Returns whether one was
/// added.
fn mark_system(system: &mut Option<SystemPrompt>) -> bool {
    match system {
        Some(SystemPrompt::Text(text)) if !text.trim().is_empty() => {
            *system = Some(SystemPrompt::Blocks(vec![SystemBlock {
                block_type: "text".to_string(),
                text: std::mem::take(text),
                cache_control: Some(CacheControl::ephemeral()),
            }]));
            true
        }
        Some(SystemPrompt::Blocks(blocks)) => {
            if blocks.iter().any(|block| block.cache_control.is_some()) {
                return false;
            }
            match blocks
                .iter_mut()
                .rev()
                .find(|block| !block.text.trim().is_empty())
            {
                Some(block) => {
                    block.cache_control = Some(CacheControl::ephemeral());
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Put a breakpoint on the last block of a message that can carry one.
/// Thinking blocks cannot, and neither can empty text. Returns whether one
/// was added.
fn mark_message(content: &mut Value) -> bool {
    if let Value::String(text) = content {
        if text.trim().is_empty() {
            return false;
        }
        let mut block = Map::new();
        block.insert("type".into(), Value::String("text".into()));
        block.insert("text".into(), Value::String(std::mem::take(text)));
        block.insert("cache_control".into(), ephemeral());
        *content = Value::Array(vec![Value::Object(block)]);
        return true;
    }
    let Some(blocks) = content.as_array_mut() else {
        return false;
    };
    let target = blocks.iter_mut().rev().find_map(|block| {
        let object = block.as_object_mut()?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let empty_text = kind == "text"
            && object
                .get("text")
                .and_then(Value::as_str)
                .is_none_or(|text| text.trim().is_empty());
        (!matches!(kind, "thinking" | "redacted_thinking") && !empty_text).then_some(object)
    });
    match target {
        Some(object) if !object.contains_key("cache_control") => {
            object.insert("cache_control".into(), ephemeral());
            true
        }
        _ => false,
    }
}

fn ephemeral() -> Value {
    serde_json::to_value(CacheControl::ephemeral()).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApiMessage, ApiToolDefinition};
    use serde_json::json;

    fn message(role: &str, content: Value) -> ApiMessage {
        ApiMessage {
            role: role.into(),
            content,
        }
    }

    fn tool(name: &str) -> ApiToolDefinition {
        ApiToolDefinition {
            name: name.into(),
            description: "A tool.".into(),
            input_schema: json!({"type": "object"}),
            cache_control: None,
        }
    }

    fn request(messages: Vec<ApiMessage>) -> CreateMessageRequest {
        CreateMessageRequest::builder("claude-sonnet-5", 1024)
            .system_text("You are a coding agent.")
            .tools(vec![tool("Read"), tool("Bash")])
            .messages(messages)
            .build()
    }

    fn marked(value: &Value) -> Vec<String> {
        value
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| block.get("cache_control").is_some())
                    .map(|block| block["type"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn an_agent_turn_is_marked_like_claude_code() {
        let mut request = request(vec![
            message("user", json!("Fix the bug")),
            message(
                "assistant",
                json!([{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]),
            ),
            message(
                "user",
                json!([{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]),
            ),
        ]);
        apply_breakpoints(&mut request);

        // The prompt string became a block so it could carry a breakpoint.
        assert_eq!(marked(&request.messages[0].content), ["text"]);
        assert!(marked(&request.messages[1].content).is_empty());
        assert_eq!(marked(&request.messages[2].content), ["tool_result"]);
        match &request.system {
            Some(SystemPrompt::Blocks(blocks)) => assert!(blocks[0].cache_control.is_some()),
            other => panic!("system should be blocks, got {other:?}"),
        }
        let tools = request.tools.as_ref().unwrap();
        assert!(tools[0].cache_control.is_none());
        assert!(tools[1].cache_control.is_some());
        assert_eq!(existing_breakpoints(&request), MAX_BREAKPOINTS);
    }

    #[test]
    fn breakpoints_already_on_the_request_count_toward_the_limit() {
        let mut request = request(vec![
            message(
                "user",
                json!([{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}]),
            ),
            message("assistant", json!("b")),
            message(
                "user",
                json!([{"type": "text", "text": "c", "cache_control": {"type": "ephemeral"}}]),
            ),
            message("assistant", json!("d")),
            message("user", json!("e")),
        ]);
        apply_breakpoints(&mut request);

        // Two were there; the last message and the system prompt take the
        // other two, and the tools go without.
        assert_eq!(existing_breakpoints(&request), MAX_BREAKPOINTS);
        assert_eq!(marked(&request.messages[4].content), ["text"]);
        assert!(request.tools.as_ref().unwrap()[1].cache_control.is_none());
    }

    #[test]
    fn thinking_and_empty_text_never_carry_a_breakpoint() {
        let mut content = json!([
            {"type": "text", "text": "Checking."},
            {"type": "thinking", "thinking": "…", "signature": "s"},
            {"type": "text", "text": "  "}
        ]);
        assert!(mark_message(&mut content));
        assert_eq!(marked(&content), ["text"]);
        assert!(content[0].get("cache_control").is_some());

        let mut only_thinking = json!([{"type": "thinking", "thinking": "…", "signature": "s"}]);
        assert!(!mark_message(&mut only_thinking));
        let mut blank = json!("   ");
        assert!(!mark_message(&mut blank));
    }

    #[test]
    fn a_single_prompt_marks_itself_once() {
        let mut request = request(vec![message("user", json!("Hello"))]);
        apply_breakpoints(&mut request);
        assert_eq!(marked(&request.messages[0].content), ["text"]);
        assert_eq!(existing_breakpoints(&request), 3);
    }

    #[test]
    fn the_serialized_request_carries_the_breakpoints() {
        let mut request = request(vec![message("user", json!("Hello"))]);
        apply_breakpoints(&mut request);
        let body = serde_json::to_value(&request).unwrap();
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }
}
