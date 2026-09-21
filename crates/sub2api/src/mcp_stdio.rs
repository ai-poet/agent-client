//! A minimal MCP client over a child process's stdio.
//!
//! Waku's Computer Use helpers all speak the same thing: newline-delimited
//! JSON-RPC on stdin/stdout, `initialize` first, then `tools/call`. The
//! JavaScript REPL uses this to reach the native helper (the macOS Swift
//! bundle, or `cua-driver` elsewhere), and the daemon uses it to probe that
//! helper's permissions. Both live in crates that depend on this one, so the
//! client lives here rather than in either of them.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, anyhow, bail};
use serde_json::{Value, json};

/// The MCP revision both ends of every Waku helper conversation speak.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// One MCP server, spawned as a child and driven synchronously.
///
/// Requests are strictly sequential: a call blocks until the matching
/// response (or the deadline) arrives. Notifications from the server are
/// skipped. That is enough for every helper Waku drives, none of which
/// initiates traffic of its own.
pub struct McpStdioClient {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
    label: String,
    server_info: Value,
}

impl McpStdioClient {
    /// Spawn `command args…`, forward its stderr to ours under `label`, and
    /// complete the MCP handshake before returning.
    ///
    /// `deadline` bounds the handshake; a server that never answers
    /// `initialize` is killed rather than waited on.
    pub fn spawn(
        command: &Path,
        args: &[&str],
        envs: &[(&str, &str)],
        client_name: &str,
        label: &str,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Self> {
        let mut process = Command::new(command);
        process
            .args(args)
            .envs(envs.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::cli_detect::detach_console(&mut process);
        let mut child = process
            .spawn()
            .with_context(|| format!("failed to start {}", command.display()))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("{label} stdin is unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("{label} stdout is unavailable"))?;
        if let Some(stderr) = child.stderr.take() {
            let prefix = label.to_owned();
            std::thread::Builder::new()
                .name(format!("{label}-stderr"))
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        eprintln!("{prefix}: {line}");
                    }
                })?;
        }
        let mut client = Self {
            child,
            input: BufWriter::new(input),
            output: BufReader::new(output),
            next_id: 1,
            label: label.to_owned(),
            server_info: Value::Null,
        };
        let initialized = client.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": client_name, "version": env!("CARGO_PKG_VERSION")}
            }),
            deadline,
        )?;
        client.server_info = initialized
            .get("serverInfo")
            .cloned()
            .unwrap_or(Value::Null);
        client.notify("notifications/initialized", json!({}))?;
        Ok(client)
    }

    /// The server process id, for registration files and reapers.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The `serverInfo` object the server answered `initialize` with —
    /// typically `{name, version}` — or `null` if it sent none.
    pub fn server_info(&self) -> &Value {
        &self.server_info
    }

    /// `tools/call`. An `isError` result becomes an `Err` carrying the
    /// server's text content, so callers never have to inspect it.
    pub fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Value> {
        let result = self.call_tool_raw(name, arguments, deadline)?;
        if is_tool_error(&result) {
            bail!("{}", text_content(&result));
        }
        Ok(result)
    }

    /// `tools/call`, returning the result whether or not the tool flagged
    /// `isError`. An `Err` here is a transport failure — the server is gone
    /// or unreadable — which callers may want to treat differently from a
    /// tool that answered with a refusal.
    pub fn call_tool_raw(
        &mut self,
        name: &str,
        arguments: Value,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Value> {
        self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
            deadline,
        )
    }

    /// Send one request and block for its response.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let watchdog = RequestWatchdog::start(self.child.id(), deadline)?;
        let label = self.label.clone();
        let result = (|| -> anyhow::Result<Value> {
            write_message(
                &mut self.input,
                &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
            )?;
            loop {
                let mut line = String::new();
                let bytes = self.output.read_line(&mut line)?;
                if bytes == 0 {
                    let status = self.child.try_wait()?;
                    bail!(
                        "{label} closed its session{}",
                        status
                            .map(|status| format!(" ({status})"))
                            .unwrap_or_default()
                    );
                }
                let message: Value = serde_json::from_str(line.trim())
                    .with_context(|| format!("{label} returned invalid JSON"))?;
                if message.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = message.get("error") {
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{label} request failed"));
                    bail!("{detail}");
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| anyhow!("{label} response has no result"));
            }
        })();
        if watchdog.finish() {
            bail!("{} request timed out", self.label);
        }
        result
    }

    /// Send one notification; nothing is awaited.
    pub fn notify(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        write_message(
            &mut self.input,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        // The session is over: take the helper down rather than trust it to
        // notice stdin closing. On Windows that has to reach the worker
        // `cua-driver` spawns too, which `Child::kill` alone would not.
        #[cfg(windows)]
        kill_process_tree(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Serialize one JSON-RPC message as a single line and flush it.
pub fn write_message(output: &mut impl Write, message: &Value) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *output, message)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

/// Whether a tool result carries the MCP `isError` flag.
pub fn is_tool_error(result: &Value) -> bool {
    result.get("isError").and_then(Value::as_bool) == Some(true)
}

/// Every `text` content block of a tool result, joined by blank lines.
pub fn text_content(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Terminate `pid` and, on Windows, everything it spawned.
///
/// `cua-driver` starts a UIAccess worker of its own; killing only the parent
/// would leave that worker holding the desktop.
pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/PID", &pid.to_string(), "/T", "/F"]);
        crate::cli_detect::detach_console(&mut taskkill);
        let _ = taskkill
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // SAFETY: a plain signal to a pid we spawned; a stale id fails
        // harmlessly with ESRCH.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    #[cfg(not(any(windows, unix)))]
    let _ = pid;
}

/// Kills the server when a request outlives its deadline, so a hung helper
/// surfaces as an error instead of a stuck REPL.
struct RequestWatchdog {
    completed: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<bool>>,
}

impl RequestWatchdog {
    fn start(pid: u32, deadline: Option<Instant>) -> anyhow::Result<Self> {
        let Some(deadline) = deadline else {
            return Ok(Self {
                completed: None,
                thread: None,
            });
        };
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| anyhow!("request timed out"))?;
        let (completed, completion) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("mcp-stdio-timeout".into())
            .spawn(move || {
                if completion.recv_timeout(remaining).is_ok() {
                    return false;
                }
                kill_process_tree(pid);
                true
            })?;
        Ok(Self {
            completed: Some(completed),
            thread: Some(thread),
        })
    }

    fn finish(mut self) -> bool {
        self.complete()
    }

    fn complete(&mut self) -> bool {
        if let Some(completed) = self.completed.take() {
            let _ = completed.send(());
        }
        self.thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or(false)
    }
}

impl Drop for RequestWatchdog {
    fn drop(&mut self) {
        let _ = self.complete();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_content_joins_only_text_blocks() {
        let result = json!({
            "content": [
                {"type": "text", "text": "first"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png"},
                {"type": "text", "text": "second"}
            ]
        });
        assert_eq!(text_content(&result), "first\n\nsecond");
        assert_eq!(text_content(&json!({})), "");
    }

    /// What matters here is the framing — one compact line with one trailing
    /// newline. Asserting the exact bytes also asserted a key order, which
    /// `serde_json` only fixes when its `preserve_order` feature is off:
    /// building this crate alongside one that enables it flipped the order
    /// and failed a test about newlines.
    #[test]
    fn write_message_emits_one_line() {
        let mut buffer = Vec::new();
        write_message(&mut buffer, &json!({"jsonrpc": "2.0", "id": 1})).unwrap();

        let text = String::from_utf8(buffer).expect("UTF-8");
        assert!(text.ends_with('\n'), "{text:?}");
        assert_eq!(text.matches('\n').count(), 1, "one line, not several");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text.trim_end()).unwrap(),
            json!({"jsonrpc": "2.0", "id": 1})
        );
    }
}
