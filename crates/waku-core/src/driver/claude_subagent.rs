//! Claude Code's sub-agents, as records in the background-work panel.
//!
//! A sub-agent's own messages arrive on the main stream with
//! `parent_tool_use_id` set to the `Task` / `Agent` call that started it.
//! They used to be flattened into a text log — narrative, then one
//! `› tool · subject` line per call, results dropped. Now each becomes an
//! entry of the sub-agent's record (`driver::subagent::SubagentFeed`): its
//! text, and its tool calls as the same rows the transcript shows, completed
//! by their results.
//!
//! The record hangs off the sub-agent's panel entry. That entry is normally
//! created by the CLI's `task_started` and keyed by the task id; when a
//! sub-agent's message comes first — or a CLI announces no task at all — the
//! entry is created from that message, keyed by the call's id, and a later
//! `task_started` for the same call is filed under that key rather than
//! opening a second entry.
//!
//! A child module of `claude` so it can keep its state in
//! `ClaudeStreamState` beside the rest.

use serde_json::Value;

use super::ClaudeStreamState;
use crate::driver::DriverEventSink;
use crate::driver::activity;
use crate::driver::subagent::{ChildCall, SubagentFeed};
use crate::model::{
    ActivityKind, BackgroundWorkEvent, BackgroundWorkItem, BackgroundWorkKey, BackgroundWorkKind,
    BackgroundWorkStatus, DriverEvent, SubagentCall,
};

/// What the main stream's `Task` / `Agent` call said, for an entry its own
/// messages have to create.
pub(super) fn note_parent_call(
    id: &str,
    name: &str,
    input: Option<&Value>,
    state: &mut ClaudeStreamState,
) {
    let Some(call) = SubagentCall::from_tool(name, input) else {
        return;
    };
    let prompt = input
        .and_then(|input| input.get("prompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .map(str::to_owned);
    state.subagent_calls.insert(id.to_owned(), (call, prompt));
}

/// `task_started` named the call a task serves. The task's events go under
/// the key the call's record already has, if it has one.
pub(super) fn link_task(state: &mut ClaudeStreamState, task_id: &str, tool_use_id: &str) {
    match state.subagent_tasks.get(tool_use_id) {
        Some(key) if key != task_id => {
            state.task_keys.insert(task_id.to_owned(), key.clone());
        }
        _ => {
            state
                .subagent_tasks
                .insert(tool_use_id.to_owned(), task_id.to_owned());
        }
    }
}

/// File a task's entry under the key its record already has. The control id
/// stays the task id: that is what a stop request names.
pub(super) fn rekey(state: &ClaudeStreamState, item: &mut BackgroundWorkItem) {
    if let Some(key) = state.task_keys.get(&item.key.provider_id) {
        item.key.provider_id = key.clone();
    }
}

/// A sub-agent's assistant message: its text, and its tool calls as running
/// rows.
pub(super) fn forward_assistant(
    parent: &str,
    value: &Value,
    events: &impl DriverEventSink,
    state: &mut ClaudeStreamState,
) {
    let key = record_key(parent, events, state);
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    let feed = feed(state, &key);
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let Some(text) = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                else {
                    continue;
                };
                // Each block is complete: one entry apiece.
                feed.close_text();
                if let Some(event) = feed.text(text) {
                    let _ = events.send(DriverEvent::BackgroundWork(event));
                }
            }
            Some("tool_use") => {
                let Some(id) = block.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Tool")
                    .to_owned();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                let kind = crate::driver::support::classify_tool(&name);
                let title = activity::input_title(Some(&input))
                    .or_else(|| {
                        SubagentCall::from_tool(&name, Some(&input)).map(|call| call.description)
                    })
                    .unwrap_or_else(|| name.clone());
                let row = activity::tool_activity(
                    Some(id.to_owned()),
                    kind,
                    title.clone(),
                    Some(&input),
                    None,
                    None,
                    false,
                    false,
                );
                feed.remember(id, ChildCall { kind, title, input });
                let _ = events.send(DriverEvent::BackgroundWork(feed.activity(row)));
            }
            _ => {}
        }
    }
}

/// A sub-agent's tool results: each completes the row its call started.
pub(super) fn forward_tool_results(
    parent: &str,
    value: &Value,
    events: &impl DriverEventSink,
    state: &mut ClaudeStreamState,
) {
    let Some(key) = state.subagent_tasks.get(parent).cloned() else {
        return;
    };
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    let feed = feed(state, &key);
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
            continue;
        };
        let call = feed.take(id).unwrap_or_else(|| ChildCall {
            kind: ActivityKind::Tool,
            title: "Tool".to_owned(),
            input: Value::Null,
        });
        let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
        let row = activity::tool_activity(
            Some(id.to_owned()),
            call.kind,
            call.title,
            Some(&call.input),
            block.get("content"),
            block.get("content"),
            failed,
            true,
        );
        let _ = events.send(DriverEvent::BackgroundWork(feed.activity(row)));
    }
}

/// The call that started a sub-agent returned on the main stream. An entry
/// this module created has no task events to settle it, so this does; one
/// the CLI announced is settled by its own `task_notification`.
pub(super) fn settle_parent(
    id: &str,
    failed: bool,
    output: Option<&Value>,
    events: &impl DriverEventSink,
    state: &mut ClaudeStreamState,
) {
    if state.subagent_tasks.get(id).map(String::as_str) != Some(id) {
        return;
    }
    let status = if failed {
        BackgroundWorkStatus::Failed
    } else {
        BackgroundWorkStatus::Completed
    };
    let mut item = BackgroundWorkItem::new(BackgroundWorkKind::Subagent, id, "", status);
    item.output = output.and_then(result_text);
    let _ = events.send(DriverEvent::BackgroundWork(BackgroundWorkEvent::Upsert(item)));
}

/// The key a sub-agent's record goes under, creating its entry when this is
/// the first anyone has heard of it.
fn record_key(parent: &str, events: &impl DriverEventSink, state: &mut ClaudeStreamState) -> String {
    if let Some(key) = state.subagent_tasks.get(parent) {
        return key.clone();
    }
    let (call, prompt) = match state.subagent_calls.get(parent).cloned() {
        Some((call, prompt)) => (Some(call), prompt),
        None => (None, None),
    };
    let title = call
        .as_ref()
        .map(|call| call.description.clone())
        .unwrap_or_else(|| tr!("background.subagent"));
    let mut item = BackgroundWorkItem::new(
        BackgroundWorkKind::Subagent,
        parent,
        title,
        BackgroundWorkStatus::Running,
    );
    item.origin_activity_id = Some(parent.to_owned());
    item.role = call.and_then(|call| call.agent_type);
    item.command = prompt;
    let _ = events.send(DriverEvent::BackgroundWork(BackgroundWorkEvent::Upsert(item)));
    state
        .subagent_tasks
        .insert(parent.to_owned(), parent.to_owned());
    parent.to_owned()
}

fn feed<'a>(state: &'a mut ClaudeStreamState, key: &str) -> &'a mut SubagentFeed {
    state.subagent_feeds.entry(key.to_owned()).or_insert_with(|| {
        SubagentFeed::new(BackgroundWorkKey::new(BackgroundWorkKind::Subagent, key))
    })
}

/// A tool result's text: a plain string, or the text blocks of a content
/// array.
fn result_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
