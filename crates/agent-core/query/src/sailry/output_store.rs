//! Session-scoped original-output storage and UTF-8-safe readback.
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_PAGE_BYTES: usize = 32 * 1024;
const MAX_RECORDS: usize = 512;
static SHARED: OnceLock<Arc<OutputStore>> = OnceLock::new();

#[derive(Clone)]
struct Record {
    revision: String,
    result: String,
    stdout: Option<String>,
    stderr: Option<String>,
    command_id: Option<String>,
}

#[derive(Default)]
pub struct OutputStore {
    sessions: Mutex<HashMap<String, HashMap<String, Record>>>,
    order: Mutex<VecDeque<(String, String)>>,
}

impl OutputStore {
    pub fn shared() -> Arc<Self> {
        SHARED.get_or_init(|| Arc::new(Self::default())).clone()
    }

    pub fn record(&self, session: &str, call: &str, result: &CallToolResult) -> String {
        let original = serde_json::to_value(result)
            .expect("tool result is JSON")
            .to_string();
        let revision = agent_core_engine::agent::output::result_revision(result);
        let data = result.structured_content.as_ref().unwrap_or(&Value::Null);
        let data = data.get("data").unwrap_or(data);
        let alias = command_id(result);
        let record = Record {
            revision: revision.clone(),
            result: original,
            stdout: data["stdout"]["text"].as_str().map(str::to_owned),
            stderr: data["stderr"]["text"].as_str().map(str::to_owned),
            command_id: alias.clone(),
        };
        let mut sessions = self.sessions.lock().unwrap();
        let outputs = sessions.entry(session.to_owned()).or_default();
        let is_new = !outputs.contains_key(call);
        outputs.insert(call.to_owned(), record.clone());
        if let Some(id) = alias {
            outputs.insert(id, record);
        }
        if is_new {
            let mut order = self.order.lock().unwrap();
            order.push_back((session.to_owned(), call.to_owned()));
            while order.len() > MAX_RECORDS {
                let Some((session, call)) = order.pop_front() else {
                    break;
                };
                if let Some(record) = sessions
                    .get_mut(&session)
                    .and_then(|outputs| outputs.remove(&call))
                {
                    if let Some(alias) = record.command_id {
                        if let Some(outputs) = sessions.get_mut(&session) {
                            outputs.remove(&alias);
                        }
                    }
                }
            }
        }
        revision
    }

    pub fn read(
        &self,
        session: &str,
        call: &str,
        revision: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        let record = self.get(session, call)?;
        if record.revision != revision {
            return Err("output revision does not match the saved result".into());
        }
        page(&record.result, offset, limit).map(|page| {
            json!({
                "output":{"call":call,"revision":record.revision},
                "offset":page.offset,
                "total_bytes":page.total,
                "next_offset":page.next,
                "text":page.text,
            })
        })
    }

    pub fn read_command(
        &self,
        session: &str,
        id: &str,
        stream: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        let record = self.get(session, id)?;
        let text = match stream {
            "stdout" => record.stdout.as_deref(),
            "stderr" => record.stderr.as_deref(),
            _ => return Err("stream must be stdout or stderr".into()),
        }
        .ok_or_else(|| "the command did not retain this stream".to_owned())?;
        page(text, offset, limit).map(|page| {
            json!({
                "id":id,
                "stream":stream,
                "offset":page.offset,
                "total_bytes":page.total,
                "next_offset":page.next,
                "text":page.text,
                "truncated":false,
                "invalid_utf8":false,
            })
        })
    }

    fn get(&self, session: &str, call: &str) -> Result<Record, String> {
        self.sessions
            .lock()
            .unwrap()
            .get(session)
            .and_then(|outputs| outputs.get(call))
            .cloned()
            .ok_or_else(|| "output is not available in this session".into())
    }
}

fn command_id(result: &CallToolResult) -> Option<String> {
    let value = result.structured_content.as_ref()?;
    value
        .pointer("/sailry_command/id")
        .or_else(|| value.pointer("/data/info/id"))
        .or_else(|| value.pointer("/info/id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

struct Page {
    offset: usize,
    total: usize,
    next: Option<usize>,
    text: String,
}

fn page(text: &str, offset: usize, limit: usize) -> Result<Page, String> {
    if limit == 0 || limit > MAX_PAGE_BYTES {
        return Err(format!(
            "limit must be between 1 and {MAX_PAGE_BYTES} bytes"
        ));
    }
    if offset > text.len() {
        return Err("offset exceeds the saved output".into());
    }
    let range = agent_core_engine::process::output::utf8_page(text.as_bytes(), offset, limit)
        .map_err(|error| error.message)?;
    let next = (range.end < text.len()).then_some(range.end);
    Ok(Page {
        offset,
        total: text.len(),
        next,
        text: text[range].to_owned(),
    })
}

/// Tests for the session-scoped original-output store.

#[cfg(test)]
mod tests {
    use super::OutputStore;
    use rmcp::model::{CallToolResult, ContentBlock};

    #[test]
    fn pages_original_results_and_rejects_wrong_revisions() {
        let store = OutputStore::default();
        let mut result = CallToolResult::success(vec![ContentBlock::text("hello")]);
        result.structured_content = Some(serde_json::json!({"kind":"file_lines"}));
        let reference = store.record("session", "call", &result);
        let first = store.read("session", "call", &reference, 0, 8).unwrap();
        assert_eq!(first["offset"], 0);
        assert_eq!(first["text"].as_str().unwrap().len(), 8);
        assert!(first["next_offset"].as_u64().is_some());
        assert!(store.read("session", "call", "wrong", 0, 8).is_err());
        assert!(store.read("other", "call", &reference, 0, 8).is_err());
    }

    #[test]
    fn reads_saved_command_streams_without_running_commands() {
        let store = OutputStore::default();
        let mut result = CallToolResult::success(vec![ContentBlock::text("hello")]);
        result.structured_content = Some(serde_json::json!({
            "kind":"command_result",
            "data":{"stdout":{"text":"hello"},"stderr":{"text":""}}
        }));
        store.record("session", "command", &result);
        let page = store
            .read_command("session", "command", "stdout", 1, 3)
            .unwrap();
        assert_eq!(page["text"], "ell");
        assert_eq!(page["next_offset"], 4);
        let page = store
            .read_command("session", "command", "stderr", 0, 3)
            .unwrap();
        assert_eq!(page["text"], "");
        assert_eq!(page["next_offset"], serde_json::Value::Null);
    }

    #[test]
    fn bounds_retained_output_inventory() {
        let store = OutputStore::default();
        for index in 0..=super::MAX_RECORDS {
            let result = CallToolResult::success(vec![ContentBlock::text("output")]);
            store.record("session", &format!("call-{index}"), &result);
        }
        assert!(store.get("session", "call-0").is_err());
        assert!(store
            .get("session", &format!("call-{}", super::MAX_RECORDS))
            .is_ok());
    }
}
