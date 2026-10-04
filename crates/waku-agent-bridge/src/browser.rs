//! The built-in agent drives the app's in-app browser.
//!
//! Fork addition. The browser lives in the desktop app, beside the session's
//! other right-panel tabs; the agent runs in the daemon. A tool here sends
//! [`AgentEvent::BrowserRequest`] — `{"op": …, …}` — which reaches the
//! desktop as a driver event, and waits for the desktop's answer, delivered
//! through [`crate::AgentSession::browser_result`]. The same request/answer
//! shape the permission prompt uses, over the same channel, so it works for
//! whichever desktop owns the session.
//!
//! Requests run one at a time: a click and a read racing each other would
//! read a page halfway through changing. Reading tools (snapshot, text,
//! screenshot, console, waiting) are `ReadOnly`, so plan mode allows them
//! and they never ask; acting ones are `Execute` and go through the session's
//! permission rules like a shell command.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::events::{AgentEvent, EventSink};

/// How long one request may take before the agent is told the app did not
/// answer — the desktop may be closed, or another client attached.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(150);

/// The desktop end of the in-app browser, shared by a session's tools.
pub struct BrowserHost {
    events: EventSink,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>,
    /// Unique to this host, so ids never repeat across sessions or daemon
    /// restarts — the desktop skips a request id it has already handled.
    prefix: String,
    next: AtomicU64,
    /// One request at a time.
    serial: tokio::sync::Mutex<()>,
}

impl BrowserHost {
    pub fn new(events: EventSink) -> Arc<Self> {
        Arc::new(Self {
            events,
            pending: Mutex::new(HashMap::new()),
            prefix: uuid::Uuid::new_v4().simple().to_string()[..12].to_owned(),
            next: AtomicU64::new(1),
            serial: tokio::sync::Mutex::new(()),
        })
    }

    /// Ask the desktop to do `operation`, and wait for what it did.
    pub async fn request(&self, operation: Value) -> Result<Value, String> {
        let _turn = self.serial.lock().await;
        let request_id = format!(
            "browser-{}-{}",
            self.prefix,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().insert(request_id.clone(), sender);
        self.events.emit(AgentEvent::BrowserRequest {
            request_id: request_id.clone(),
            operation,
        });
        match tokio::time::timeout(REQUEST_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("the browser request was cancelled".to_owned()),
            Err(_) => {
                self.pending.lock().remove(&request_id);
                Err("the app did not answer the browser request — the in-app browser is \
                     only available while the desktop app is open on this task"
                    .to_owned())
            }
        }
    }

    /// The desktop's answer: `{"ok": true, "value": …}` or
    /// `{"ok": false, "error": "…"}`. An answer nobody waits for — a late
    /// one, or a second client's — is dropped.
    pub fn resolve(&self, request_id: &str, result: Value) {
        let Some(sender) = self.pending.lock().remove(request_id) else {
            return;
        };
        let outcome = if result.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(result.get("value").cloned().unwrap_or(Value::Null))
        } else {
            Err(result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the browser request failed")
                .to_owned())
        };
        let _ = sender.send(outcome);
    }

    /// Cancel: every waiting tool returns at once.
    pub fn release_all(&self) {
        self.pending.lock().clear();
    }
}

/// One browser tool: a name, what the model reads about it, its input, and
/// how its result reads back.
struct Spec {
    name: &'static str,
    op: &'static str,
    description: &'static str,
    read_only: bool,
    schema: fn() -> Value,
}

#[cfg(test)]
pub const TOOL_PREFIX: &str = "browser_";

fn no_input() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn target_properties() -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "ref".to_owned(),
        json!({
            "type": "string",
            "description": "The element's ref from browser_snapshot, like \"e12\". Preferred."
        }),
    );
    properties.insert(
        "selector".to_owned(),
        json!({ "type": "string", "description": "A CSS selector, when there is no ref." }),
    );
    properties.insert(
        "text".to_owned(),
        json!({
            "type": "string",
            "description": "Visible text of the element to act on, when there is neither a ref nor a selector."
        }),
    );
    properties
}

const SPECS: &[Spec] = &[
    Spec {
        name: "browser_navigate",
        op: "navigate",
        description: "Open a URL in the in-app browser (the browser tab of this task in the app's right panel), and wait for the page to load. Use it to test a web app — a dev server on localhost, a staging site. Then call browser_snapshot to see what is on the page.",
        read_only: false,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The address to open. A bare host such as localhost:3000 gets http://." }
                },
                "required": ["url"],
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_snapshot",
        op: "snapshot",
        description: "Describe the page in the in-app browser: its URL and title, every visible interactive element with a ref (like [e12] button \"Save\") to pass to browser_click and browser_type, and the page's visible text. Take a fresh snapshot after anything that changes the page; refs from an older one may be gone.",
        read_only: true,
        schema: no_input,
    },
    Spec {
        name: "browser_click",
        op: "click",
        description: "Click an element in the in-app browser, named by its ref from browser_snapshot (or a CSS selector, or its visible text). Waits for a navigation it starts to finish.",
        read_only: false,
        schema: || {
            json!({
                "type": "object",
                "properties": target_properties(),
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_type",
        op: "type",
        description: "Type text into a field in the in-app browser — an input, a textarea or an editable element — named by its ref from browser_snapshot (or a selector, or its label text). Replaces what the field held unless append is true; submit presses Enter afterwards.",
        read_only: false,
        schema: || {
            let mut properties = target_properties();
            properties.insert("value".to_owned(), json!({ "type": "string", "description": "The text to type." }));
            properties.insert("append".to_owned(), json!({ "type": "boolean", "description": "Add to the field's current text instead of replacing it." }));
            properties.insert("submit".to_owned(), json!({ "type": "boolean", "description": "Press Enter after typing, submitting the field's form." }));
            json!({
                "type": "object",
                "properties": properties,
                "required": ["value"],
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_select",
        op: "select",
        description: "Choose an option of a <select> element in the in-app browser, by the option's value or visible label.",
        read_only: false,
        schema: || {
            let mut properties = target_properties();
            properties.insert("option".to_owned(), json!({ "type": "string", "description": "The option's value or its visible label." }));
            json!({
                "type": "object",
                "properties": properties,
                "required": ["option"],
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_press_key",
        op: "press_key",
        description: "Press a key in the in-app browser, on the focused element: Enter, Escape, Tab, ArrowDown, Backspace, a letter. Enter in a form field submits its form.",
        read_only: false,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The key, as KeyboardEvent.key spells it." }
                },
                "required": ["key"],
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_evaluate",
        op: "evaluate",
        description: "Run JavaScript in the page open in the in-app browser. The script is the body of an async function: use await, and return a JSON-serializable value to get it back. For checks a snapshot cannot express — computed styles, app state, network results.",
        read_only: false,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string", "description": "The function body, e.g. `return document.querySelectorAll('li').length`." }
                },
                "required": ["script"],
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_wait_for",
        op: "wait_for",
        description: "Wait until text appears on the page (or disappears, with gone=true), or an element matching a selector exists, in the in-app browser. Returns whether it happened before the timeout.",
        read_only: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to wait for in the page's visible text." },
                    "selector": { "type": "string", "description": "A CSS selector to wait for." },
                    "gone": { "type": "boolean", "description": "Wait for it to disappear instead." },
                    "timeout_ms": { "type": "integer", "minimum": 100, "maximum": 60000, "description": "How long to wait. Defaults to 10000." }
                },
                "additionalProperties": false
            })
        },
    },
    Spec {
        name: "browser_screenshot",
        op: "screenshot",
        description: "Take a screenshot of the page in the in-app browser. It is saved as a PNG file whose path is returned, and shown to the user in the conversation.",
        read_only: true,
        schema: no_input,
    },
    Spec {
        name: "browser_console",
        op: "console",
        description: "Read the page's recent console messages and uncaught errors in the in-app browser — what a test should check after an action that may have failed silently. Messages are recorded from the first browser tool call on a page onwards.",
        read_only: true,
        schema: no_input,
    },
    Spec {
        name: "browser_navigate_back",
        op: "back",
        description: "Go back to the previous page in the in-app browser.",
        read_only: false,
        schema: no_input,
    },
    Spec {
        name: "browser_reload",
        op: "reload",
        description: "Reload the page in the in-app browser and wait for it to load.",
        read_only: false,
        schema: no_input,
    },
    Spec {
        name: "browser_tabs",
        op: "tabs",
        description: "List this task's in-app browser tabs, open a new one (action=new, with an optional url), switch to one (action=select, index), or close one (action=close, index). Every other browser tool acts on the selected tab.",
        read_only: false,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "new", "select", "close"], "description": "Defaults to list." },
                    "index": { "type": "integer", "minimum": 0, "description": "The tab, as list numbers it." },
                    "url": { "type": "string", "description": "For action=new: the address to open." }
                },
                "additionalProperties": false
            })
        },
    },
];

/// The session's browser tools, all sharing `host`.
pub fn tools(host: &Arc<BrowserHost>) -> Vec<Box<dyn Tool>> {
    SPECS
        .iter()
        .map(|spec| {
            Box::new(BrowserTool {
                spec,
                host: host.clone(),
            }) as Box<dyn Tool>
        })
        .collect()
}

struct BrowserTool {
    spec: &'static Spec,
    host: Arc<BrowserHost>,
}

impl BrowserTool {
    /// `browser_tabs` reads unless it is asked to change something.
    fn reads(&self, input: &Value) -> bool {
        if self.spec.op == "tabs" {
            return matches!(input.get("action").and_then(Value::as_str), None | Some("list"));
        }
        self.spec.read_only
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        self.spec.name
    }

    fn description(&self) -> &str {
        self.spec.description
    }

    fn permission_level(&self) -> PermissionLevel {
        if self.spec.read_only {
            PermissionLevel::ReadOnly
        } else {
            PermissionLevel::Execute
        }
    }

    /// The prompt below names what the call does to the page.
    fn self_gates(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        (self.spec.schema)()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let reads = self.reads(&input);
        if let Err(error) = ctx.check_permission(self.name(), &describe(self.spec.op, &input), reads) {
            return ToolResult::error(error.to_string());
        }
        let mut operation = match input {
            Value::Object(object) => object,
            _ => serde_json::Map::new(),
        };
        operation.insert("op".to_owned(), Value::String(self.spec.op.to_owned()));
        match self.host.request(Value::Object(operation)).await {
            Ok(value) => render(self.spec.op, value),
            Err(error) => ToolResult::error(error),
        }
    }
}

/// The permission prompt's one line.
fn describe(op: &str, input: &Value) -> String {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default();
    let target = [text("ref"), text("selector"), text("text")]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or("the focused element");
    match op {
        "navigate" => format!("Open {} in the in-app browser", text("url")),
        "click" => format!("Click {target} in the in-app browser"),
        "type" => format!("Type into {target} in the in-app browser"),
        "select" => format!("Choose \"{}\" in {target} in the in-app browser", text("option")),
        "press_key" => format!("Press {} in the in-app browser", text("key")),
        "evaluate" => "Run JavaScript in the in-app browser's page".to_owned(),
        "back" => "Go back in the in-app browser".to_owned(),
        "reload" => "Reload the in-app browser's page".to_owned(),
        "tabs" => format!("{} an in-app browser tab", match text("action") {
            "new" => "Open",
            "select" => "Switch to",
            "close" => "Close",
            _ => "List",
        }),
        _ => "Read the in-app browser's page".to_owned(),
    }
}

/// What the model reads back. Most answers carry a `text` field written for
/// it by the desktop; a screenshot also carries its picture, which goes to
/// the transcript and not to the model.
fn render(op: &str, value: Value) -> ToolResult {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| match &value {
            Value::String(text) => text.clone(),
            Value::Null => "Done.".to_owned(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        });
    let result = ToolResult::success(text);
    if op != "screenshot" {
        return result;
    }
    match value.get("png_base64").and_then(Value::as_str) {
        Some(data) => result.with_metadata(json!({
            "content": [{ "type": "image", "mime": "image/png", "data": data }]
        })),
        None => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> (Arc<BrowserHost>, Arc<Mutex<Vec<AgentEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            EventSink::new(move |event| seen.lock().push(event))
        };
        (BrowserHost::new(sink), seen)
    }

    fn requested(seen: &Mutex<Vec<AgentEvent>>) -> Option<(String, Value)> {
        seen.lock().iter().find_map(|event| match event {
            AgentEvent::BrowserRequest {
                request_id,
                operation,
            } => Some((request_id.clone(), operation.clone())),
            _ => None,
        })
    }

    #[tokio::test]
    async fn a_request_waits_for_the_desktops_answer() {
        let (host, seen) = host();
        let waiting = {
            let host = host.clone();
            tokio::spawn(async move { host.request(json!({"op": "snapshot"})).await })
        };
        let (request_id, operation) = loop {
            if let Some(request) = requested(&seen) {
                break request;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(operation["op"], "snapshot");
        host.resolve(&request_id, json!({"ok": true, "value": {"text": "page"}}));
        assert_eq!(waiting.await.unwrap(), Ok(json!({"text": "page"})));
    }

    #[tokio::test]
    async fn a_refusal_and_a_cancel_both_end_the_wait() {
        let (host, seen) = host();
        let refused = {
            let host = host.clone();
            tokio::spawn(async move { host.request(json!({"op": "click"})).await })
        };
        let (request_id, _) = loop {
            if let Some(request) = requested(&seen) {
                break request;
            }
            tokio::task::yield_now().await;
        };
        host.resolve(&request_id, json!({"ok": false, "error": "no element e9"}));
        assert_eq!(refused.await.unwrap(), Err("no element e9".to_owned()));

        seen.lock().clear();
        let cancelled = {
            let host = host.clone();
            tokio::spawn(async move { host.request(json!({"op": "navigate"})).await })
        };
        while requested(&seen).is_none() {
            tokio::task::yield_now().await;
        }
        host.release_all();
        assert!(cancelled.await.unwrap().is_err());
        // A late answer for a request nobody waits on is ignored.
        host.resolve("browser-404", json!({"ok": true}));
    }

    #[test]
    fn reading_tools_never_ask_and_acting_ones_do() {
        let (host, _) = host();
        let tools = tools(&host);
        let level = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.name() == name)
                .map(|tool| tool.permission_level())
                .unwrap()
        };
        for name in ["browser_snapshot", "browser_screenshot", "browser_console", "browser_wait_for"] {
            assert_eq!(level(name), PermissionLevel::ReadOnly, "{name}");
        }
        for name in ["browser_navigate", "browser_click", "browser_type", "browser_evaluate"] {
            assert_eq!(level(name), PermissionLevel::Execute, "{name}");
        }
        assert!(tools.iter().all(|tool| tool.name().starts_with(TOOL_PREFIX)));
        let tabs = BrowserTool {
            spec: SPECS.iter().find(|spec| spec.op == "tabs").unwrap(),
            host,
        };
        assert!(tabs.reads(&json!({})));
        assert!(tabs.reads(&json!({"action": "list"})));
        assert!(!tabs.reads(&json!({"action": "close", "index": 0})));
    }

    #[test]
    fn a_screenshot_shows_its_picture_to_the_person() {
        let result = render(
            "screenshot",
            json!({"text": "Saved shot.png", "png_base64": "iVBORw0KGgo="}),
        );
        assert_eq!(result.content, "Saved shot.png");
        assert_eq!(
            result.metadata.as_ref().unwrap()["content"][0]["mime"],
            "image/png"
        );
        assert!(render("snapshot", json!({"text": "x"})).metadata.is_none());
    }
}
