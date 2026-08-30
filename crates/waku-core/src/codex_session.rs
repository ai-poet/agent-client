//! Codex CLI session discovery and transcript import.
//!
//! The app-server owns Codex's state database and rollout migration rules, so
//! use its public thread APIs instead of reimplementing `codex resume` by
//! walking private SQLite tables or guessing rollout filenames. These probes
//! are bounded, short-lived, and called only from the daemon's background RPC
//! path — never from a render frame.

use std::io::{BufRead as _, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::model::{
    AgentTurn, Message, MessageRole, ProviderResumeCursor, ProviderSessionHistory,
    ProviderSessionSummary, TurnStatus,
};

const RPC_TIMEOUT: Duration = Duration::from_secs(10);

fn write_json_line(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn app_server_request(binary: &Path, request: Value) -> anyhow::Result<Value> {
    let mut command = crate::command_env::command(binary);
    let command = command
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = crate::command_env::spawn(command)
        .with_context(|| format!("could not start {} app-server", binary.display()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Codex app-server stdin is unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Codex app-server stdout is unavailable"))?;
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && tx.send(value).is_err()
            {
                break;
            }
        }
    });

    let initialized = write_json_line(
        &mut stdin,
        &json!({
            "method": "initialize",
            "id": 0,
            "params": {
                "clientInfo": {
                    "name": "waku",
                    "title": "Waku",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": { "experimentalApi": true }
            }
        }),
    )
    .and_then(|_| write_json_line(&mut stdin, &json!({"method": "initialized", "params": {}})))
    .and_then(|_| write_json_line(&mut stdin, &request));
    if let Err(error) = initialized {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return Err(error).context("could not write to Codex app-server");
    }

    let deadline = Instant::now() + RPC_TIMEOUT;
    let response = (|| -> anyhow::Result<Value> {
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| anyhow!("Codex app-server session request timed out"))?;
            let value = rx
                .recv_timeout(remaining)
                .map_err(|_| anyhow!("Codex app-server session request timed out"))?;
            if value.get("id").and_then(Value::as_u64) == Some(1) {
                return Ok(value);
            }
        }
    })();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    let response = response?;

    if let Some(error) = response.pointer("/error/message").and_then(Value::as_str) {
        bail!("Codex rejected the session request: {error}");
    }
    Ok(response)
}

fn title_from_prompt(prompt: &str) -> Option<String> {
    let mut title = prompt
        .split_whitespace()
        .take(7)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        return None;
    }
    if title.chars().count() > 54 {
        title = format!("{}…", title.chars().take(53).collect::<String>());
    }
    Some(title)
}

fn parse_session_summaries(response: &Value, limit: usize) -> Vec<ProviderSessionSummary> {
    response
        .pointer("/result/data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|thread| {
            thread.get("ephemeral").and_then(Value::as_bool) != Some(true)
                && thread.get("parentThreadId").is_none_or(Value::is_null)
        })
        .filter_map(|thread| {
            let id = thread.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let cwd = PathBuf::from(thread.get("cwd").and_then(Value::as_str)?);
            if !cwd.is_absolute() || !cwd.is_dir() {
                return None;
            }
            let title = thread
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    thread
                        .get("preview")
                        .and_then(Value::as_str)
                        .and_then(title_from_prompt)
                })?;
            let created_at = thread
                .get("createdAt")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let updated_at = thread
                .get("recencyAt")
                .and_then(Value::as_u64)
                .or_else(|| thread.get("updatedAt").and_then(Value::as_u64))
                .unwrap_or(created_at)
                .max(created_at);
            Some(ProviderSessionSummary {
                cursor: ProviderResumeCursor::Codex {
                    thread_id: id.to_owned(),
                },
                title,
                cwd,
                created_at,
                updated_at,
            })
        })
        .take(limit)
        .collect()
}

/// Ask Codex for CLI-owned threads, excluding exec, editor, app-server and
/// sub-agent rollouts. Waku-created app-server threads are filtered again by
/// native cursor in the daemon catalog.
pub fn list_provider_sessions(
    binary: &Path,
    limit: usize,
) -> anyhow::Result<Vec<ProviderSessionSummary>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let response = app_server_request(
        binary,
        json!({
            "method": "thread/list",
            "id": 1,
            "params": {
                "limit": limit,
                "sortKey": "updated_at",
                "sortDirection": "desc",
                "sourceKinds": ["cli"]
            }
        }),
    )?;
    Ok(parse_session_summaries(&response, limit))
}

fn codex_user_text(item: &Value) -> Option<String> {
    let text = item
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

fn parse_session_history(response: &Value) -> ProviderSessionHistory {
    let native_turns = response
        .pointer("/result/thread/turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut history = ProviderSessionHistory::default();

    for native_turn in native_turns {
        let turn_id = native_turn
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok())
            .unwrap_or_else(Uuid::new_v4);
        let started_at = native_turn
            .get("startedAt")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let completed_at = native_turn
            .get("completedAt")
            .and_then(Value::as_u64)
            .or(Some(started_at));
        let status = match native_turn.get("status").and_then(Value::as_str) {
            Some("failed") => TurnStatus::Failed,
            Some("interrupted" | "inProgress") => TurnStatus::Interrupted,
            _ => TurnStatus::Completed,
        };
        history.turns.push(AgentTurn {
            id: turn_id,
            turn_count: history.turns.len() + 1,
            status,
            provider_turn_started: true,
            provider_resume_at: None,
            started_at,
            completed_at,
            checkpoint: None,
        });

        for item in native_turn
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (role, content, created_at) = match item.get("type").and_then(Value::as_str) {
                Some("userMessage") => {
                    let Some(content) = codex_user_text(item) else {
                        continue;
                    };
                    (MessageRole::User, content, started_at)
                }
                Some("agentMessage") => {
                    let Some(content) = item
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty())
                    else {
                        continue;
                    };
                    (
                        MessageRole::Assistant,
                        content.to_owned(),
                        completed_at.unwrap_or(started_at),
                    )
                }
                _ => continue,
            };
            let mut message = Message::new_for_turn(role, content, turn_id);
            message.created_at = created_at;
            history.messages.push(message);
        }
    }
    history
}

fn retain_recent_messages(history: &mut ProviderSessionHistory, visible_turn_limit: usize) {
    let retained = history
        .turns
        .iter()
        .rev()
        .take(visible_turn_limit)
        .map(|turn| turn.id)
        .collect::<std::collections::HashSet<_>>();
    history
        .messages
        .retain(|message| message.turn_id.is_some_and(|id| retained.contains(&id)));
}

/// Load the complete native turn sequence so Waku's provider turn counts stay
/// aligned for later fork and rollback operations. Only recent display text is
/// retained; Codex remains authoritative for the full conversation context.
pub fn provider_session_history(
    binary: &Path,
    thread_id: &str,
    visible_turn_limit: usize,
) -> anyhow::Result<ProviderSessionHistory> {
    if thread_id.trim().is_empty() || visible_turn_limit == 0 {
        return Ok(ProviderSessionHistory::default());
    }
    let response = app_server_request(
        binary,
        json!({
            "method": "thread/read",
            "id": 1,
            "params": {
                "threadId": thread_id,
                "includeTurns": true
            }
        }),
    )?;
    let mut history = parse_session_history(&response);
    retain_recent_messages(&mut history, visible_turn_limit);
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thread_catalog_metadata_and_filters_subagents() {
        let root = std::env::temp_dir();
        let response = json!({"result": {"data": [
            {
                "id": "01900000-0000-7000-8000-000000000001",
                "name": "Resume this thread",
                "preview": "ignored fallback",
                "cwd": root,
                "createdAt": 10,
                "updatedAt": 20,
                "recencyAt": 25,
                "ephemeral": false,
                "parentThreadId": null
            },
            {
                "id": "01900000-0000-7000-8000-000000000002",
                "preview": "sub-agent",
                "cwd": root,
                "createdAt": 10,
                "updatedAt": 20,
                "ephemeral": false,
                "parentThreadId": "01900000-0000-7000-8000-000000000001"
            }
        ]}});

        let sessions = parse_session_summaries(&response, 10);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Resume this thread");
        assert_eq!(sessions[0].updated_at, 25);
    }

    #[test]
    fn imports_recent_turn_text_in_chronological_order() {
        let response = json!({"result": {"thread": {"turns": [
            {
                "id": "01900000-0000-7000-8000-000000000001",
                "status": "completed",
                "startedAt": 10,
                "completedAt": 20,
                "items": [
                    {"type":"userMessage","id":"u1","content":[{"type":"text","text":"first"}]},
                    {"type":"agentMessage","id":"a1","text":"done"}
                ]
            },
            {
                "id": "01900000-0000-7000-8000-000000000002",
                "status": "completed",
                "startedAt": 30,
                "completedAt": 40,
                "items": [
                    {"type":"userMessage","id":"u2","content":[{"type":"text","text":"second"}]},
                    {"type":"agentMessage","id":"a2","text":"later"}
                ]
            }
        ]}}});

        let mut history = parse_session_history(&response);
        retain_recent_messages(&mut history, 1);
        assert_eq!(history.turns.len(), 2);
        assert_eq!(history.messages.len(), 2);
        assert_eq!(history.messages[0].content, "second");
        assert_eq!(history.messages[1].content, "later");
        assert_eq!(history.messages[0].turn_id, Some(history.turns[1].id));
        assert_eq!(history.turns[1].turn_count, 2);
    }
}
