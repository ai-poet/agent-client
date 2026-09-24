//! An agent's todo list, in one shape whichever agent wrote it.
//!
//! Every agent that plans out loud keeps a checklist, and each one writes it
//! differently: Claude Code's and the built-in agent's `TodoWrite` take
//! `{todos: [{content, status}]}`, OpenCode's `todowrite` the same with a
//! priority, Codex reports `{plan: [{step, status}]}`, and ACP agents send
//! `{entries: [...]}`. [`parse_todo_list`] reads all of them, so the
//! transcript and the status capsule never need to know which agent it was.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Where one item stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    /// Dropped before it was done. Only some agents have it (OpenCode).
    Cancelled,
}

impl TodoStatus {
    /// Read a status as any of the agents spell it.
    pub fn parse(raw: &str) -> Option<Self> {
        let normalized: String = raw
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        match normalized.as_str() {
            "pending" | "todo" | "notstarted" | "open" => Some(Self::Pending),
            "inprogress" | "active" | "running" | "doing" | "started" => Some(Self::InProgress),
            "completed" | "complete" | "done" | "finished" => Some(Self::Completed),
            "cancelled" | "canceled" | "skipped" | "dropped" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Nothing more will happen to it.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

/// One item on an agent's todo list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    /// The present-tense phrasing some agents give the item being worked on
    /// ("Running the tests"), shown in place of `content` while it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
}

impl TodoItem {
    /// What to show for this item: its active phrasing while it is the one
    /// being worked on, otherwise its content.
    pub fn label(&self) -> &str {
        match (&self.active_form, self.status) {
            (Some(active), TodoStatus::InProgress) if !active.trim().is_empty() => active,
            _ => &self.content,
        }
    }
}

/// Read a todo list from a tool's arguments or output, or from a plan
/// notification. `None` when `value` is not a todo list at all; an empty list
/// is a list the agent cleared.
pub fn parse_todo_list(value: &Value) -> Option<Vec<TodoItem>> {
    let entries = match value {
        Value::Array(entries) => entries,
        Value::Object(object) => ["todos", "plan", "entries", "items"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_array))?,
        // Some agents hand back their arguments as the JSON text they sent.
        Value::String(text) => {
            let parsed: Value = serde_json::from_str(text.trim()).ok()?;
            return match parsed {
                Value::String(_) => None,
                other => parse_todo_list(&other),
            };
        }
        _ => return None,
    };
    let items: Vec<TodoItem> = entries.iter().filter_map(parse_item).collect();
    // A list whose every entry was unreadable is not a todo list; an empty
    // one is.
    (entries.is_empty() || !items.is_empty()).then_some(items)
}

fn parse_item(entry: &Value) -> Option<TodoItem> {
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let content = text("content")
        .or_else(|| text("step"))
        .or_else(|| text("text"))
        .or_else(|| text("title"))
        .or_else(|| text("description"))?;
    let status = entry
        .get("status")
        .and_then(Value::as_str)
        .and_then(TodoStatus::parse)
        .or_else(|| {
            entry
                .get("completed")
                .or_else(|| entry.get("done"))
                .and_then(Value::as_bool)
                .map(|done| {
                    if done {
                        TodoStatus::Completed
                    } else {
                        TodoStatus::Pending
                    }
                })
        })
        .unwrap_or(TodoStatus::Pending);
    Some(TodoItem {
        content,
        status,
        active_form: text("activeForm").or_else(|| text("active_form")),
    })
}

/// `(settled, total)`: how far along the list is.
pub fn todo_progress(items: &[TodoItem]) -> (usize, usize) {
    let settled = items.iter().filter(|item| item.status.is_settled()).count();
    (settled, items.len())
}

/// Index of the item being worked on: the first in progress, else the first
/// still pending. `None` once everything is settled.
pub fn current_todo(items: &[TodoItem]) -> Option<usize> {
    items
        .iter()
        .position(|item| item.status == TodoStatus::InProgress)
        .or_else(|| items.iter().position(|item| item.status == TodoStatus::Pending))
}

/// The slice of a long list worth showing: `size` items around the current
/// one, in the agent's own order. Short lists come back whole.
pub fn todo_window(items: &[TodoItem], size: usize) -> std::ops::Range<usize> {
    if items.len() <= size.max(1) {
        return 0..items.len();
    }
    let size = size.max(1);
    let anchor = current_todo(items).unwrap_or(items.len() - 1);
    let start = anchor.saturating_sub(size / 2).min(items.len() - size);
    start..start + size
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_agents_shape_reads_the_same() {
        let claude = json!({"todos": [
            {"content": "Write the parser", "status": "completed", "activeForm": "Writing the parser"},
            {"content": "Run the tests", "status": "in_progress", "activeForm": "Running the tests"}
        ]});
        let codex = json!({"explanation": "", "plan": [
            {"step": "Write the parser", "status": "completed"},
            {"step": "Run the tests", "status": "inProgress"}
        ]});
        let acp = json!({"entries": [
            {"content": "Write the parser", "priority": "high", "status": "completed"},
            {"content": "Run the tests", "priority": "high", "status": "in_progress"}
        ]});
        let bare = json!([
            {"content": "Write the parser", "status": "done"},
            {"content": "Run the tests", "status": "active"}
        ]);
        for shape in [&claude, &codex, &acp, &bare] {
            let items = parse_todo_list(shape).expect("a todo list");
            let statuses: Vec<_> = items.iter().map(|item| item.status).collect();
            assert_eq!(statuses, [TodoStatus::Completed, TodoStatus::InProgress], "{shape}");
            assert_eq!(items[1].content, "Run the tests");
        }
        let claude = parse_todo_list(&claude).unwrap();
        assert_eq!(claude[1].label(), "Running the tests");
        assert_eq!(claude[0].label(), "Write the parser");
    }

    #[test]
    fn arguments_sent_as_text_are_read_too() {
        let text = Value::String(r#"{"todos":[{"content":"a","status":"pending"}]}"#.into());
        assert_eq!(parse_todo_list(&text).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_list_is_a_cleared_list_and_anything_else_is_not_a_list() {
        assert_eq!(parse_todo_list(&json!({"todos": []})), Some(Vec::new()));
        assert_eq!(parse_todo_list(&json!({"command": "ls"})), None);
        assert_eq!(parse_todo_list(&json!([{"unrelated": 1}])), None);
        assert_eq!(parse_todo_list(&json!("plain output")), None);
    }

    #[test]
    fn progress_and_the_current_item() {
        let items = parse_todo_list(&json!([
            {"content": "a", "status": "completed"},
            {"content": "b", "status": "cancelled"},
            {"content": "c", "status": "pending"},
            {"content": "d", "status": "pending"}
        ]))
        .unwrap();
        assert_eq!(todo_progress(&items), (2, 4));
        assert_eq!(current_todo(&items), Some(2));
    }

    #[test]
    fn a_long_list_is_windowed_around_the_current_item() {
        let items: Vec<TodoItem> = (0..10)
            .map(|index| TodoItem {
                content: format!("item {index}"),
                status: match index {
                    0..=5 => TodoStatus::Completed,
                    6 => TodoStatus::InProgress,
                    _ => TodoStatus::Pending,
                },
                active_form: None,
            })
            .collect();
        assert_eq!(todo_window(&items, 3), 5..8);
        assert_eq!(todo_window(&items[..3], 3), 0..3);
        let all_done: Vec<TodoItem> = items
            .iter()
            .cloned()
            .map(|item| TodoItem { status: TodoStatus::Completed, ..item })
            .collect();
        assert_eq!(todo_window(&all_done, 3), 7..10);
    }
}
