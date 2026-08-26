//! Minimal HTTP over a `curl` subprocess.
//!
//! The fork deliberately adds no HTTP crate. Upstream already reaches the
//! network this way (`waku-core`'s usage probe), and reusing the pattern keeps
//! three properties that matter here:
//!
//! * **Secrets stay out of the process table.** Headers and request bodies
//!   travel to curl as a config on stdin, never on argv, so a bearer token is
//!   not visible to other users via `ps`.
//! * **No TLS backend to build.** A Rust HTTP client pulls in rustls plus a
//!   crypto backend, which on Windows means another native build prerequisite
//!   for every contributor and CI runner.
//! * **Nothing new to merge.** No dependency lines in shared manifests means
//!   no conflicts when merging upstream.
//!
//! curl is resolved by absolute path so a shadowed `curl` earlier on `PATH`
//! cannot intercept credentials.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};

#[cfg(target_os = "windows")]
pub(crate) const CURL_PATH: &str = r"C:\Windows\System32\curl.exe";
#[cfg(not(target_os = "windows"))]
pub(crate) const CURL_PATH: &str = "/usr/bin/curl";

const DEFAULT_TIMEOUT_SECONDS: u32 = 20;

/// A parsed HTTP response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    /// True for 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Deserialize a successful JSON body, or surface the server's error text.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        if !self.is_success() {
            return Err(anyhow!(
                "request failed with status {}: {}",
                self.status,
                error_summary(&self.body)
            ));
        }
        serde_json::from_str(&self.body)
            .with_context(|| format!("could not parse response body: {}", truncate(&self.body, 200)))
    }
}

/// An outgoing request. Headers and body are passed to curl via stdin config.
#[derive(Clone, Debug, Default)]
pub struct Request {
    method: Option<String>,
    headers: Vec<String>,
    body: Option<String>,
    timeout_seconds: Option<u32>,
}

impl Request {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push(format!("{name}: {value}"));
        self
    }

    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    /// Set a JSON body and the matching content type. Implies POST.
    pub fn json_body(mut self, body: String) -> Self {
        self.body = Some(body);
        self.method.get_or_insert_with(|| "POST".to_owned());
        self.header("Content-Type", "application/json")
    }

    pub fn method(mut self, method: &str) -> Self {
        self.method = Some(method.to_owned());
        self
    }

    pub fn timeout_seconds(mut self, seconds: u32) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// Render the curl config passed on stdin.
    ///
    /// Separated from [`Self::send`] so the escaping is unit-testable without
    /// touching the network.
    fn config(&self) -> String {
        let mut config = String::new();
        for header in &self.headers {
            config.push_str(&format!("header = {}\n", quote(header)));
        }
        if let Some(method) = &self.method {
            config.push_str(&format!("request = {}\n", quote(method)));
        }
        if let Some(body) = &self.body {
            config.push_str(&format!("data-binary = {}\n", quote(body)));
        }
        config
    }

    /// Perform the request. Blocks; callers run it off the UI thread.
    pub fn send(&self, url: &str) -> Result<Response> {
        let timeout = self
            .timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .to_string();
        let mut child = Command::new(CURL_PATH)
            .args(["-sS", "--max-time", &timeout, "-D", "-", "-K", "-", url])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not run {CURL_PATH}"))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| anyhow!("curl stdin was unavailable"))?;
            stdin
                .write_all(self.config().as_bytes())
                .context("could not write the curl configuration")?;
        }
        let output = child
            .wait_with_output()
            .context("curl did not finish cleanly")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .next_back()
                .unwrap_or("unknown error");
            return Err(anyhow!("network request failed: {detail}"));
        }
        parse(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Quote a value for a curl config line.
///
/// curl's parser understands `\\`, `\"`, `\t`, `\n`, `\r` and `\v` inside a
/// double-quoted value. A JSON body is full of `"` and `\`, so this must escape
/// rather than merely wrap.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str(r"\\"),
            '"' => quoted.push_str("\\\""),
            '\t' => quoted.push_str(r"\t"),
            '\n' => quoted.push_str(r"\n"),
            '\r' => quoted.push_str(r"\r"),
            '\u{0b}' => quoted.push_str(r"\v"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// `-D -` prefixes the body with the response headers: the status code sits on
/// the first line and the body follows the blank separator line.
fn parse(raw: &str) -> Result<Response> {
    let status = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("curl returned no status line"))?;
    let body = raw
        .find("\r\n\r\n")
        .map(|index| &raw[index + 4..])
        .or_else(|| raw.find("\n\n").map(|index| &raw[index + 2..]))
        .unwrap_or("")
        .to_owned();
    Ok(Response { status, body })
}

/// Pull a human-readable message out of an error body, falling back to the
/// raw text. The managed service reports errors as `{"message": "..."}` or
/// `{"error": "..."}` depending on the endpoint.
fn error_summary(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    parsed
        .as_ref()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_owned)
        .unwrap_or_else(|| truncate(body, 200))
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    value.chars().take(limit).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_body_with_crlf_headers() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}";
        let response = parse(raw).expect("parse");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "{\"ok\":true}");
        assert!(response.is_success());
    }

    #[test]
    fn parses_lf_only_headers() {
        let raw = "HTTP/1.1 404 Not Found\nContent-Length: 2\n\n{}";
        let response = parse(raw).expect("parse");
        assert_eq!(response.status, 404);
        assert_eq!(response.body, "{}");
        assert!(!response.is_success());
    }

    #[test]
    fn parses_empty_body() {
        let response = parse("HTTP/1.1 204 No Content\r\n\r\n").expect("parse");
        assert_eq!(response.status, 204);
        assert_eq!(response.body, "");
    }

    #[test]
    fn missing_status_line_is_an_error() {
        assert!(parse("").is_err());
        assert!(parse("garbage\r\n\r\nbody").is_err());
    }

    #[test]
    fn quotes_escape_json_bodies() {
        // A JSON body reaching curl unescaped would terminate the config value
        // early and silently truncate the request.
        assert_eq!(quote(r#"{"a":"b"}"#), r#""{\"a\":\"b\"}""#);
        assert_eq!(quote(r"back\slash"), r#""back\\slash""#);
        assert_eq!(quote("line\nbreak"), r#""line\nbreak""#);
    }

    #[test]
    fn config_carries_headers_method_and_body() {
        let config = Request::new()
            .bearer("secret-token")
            .json_body(r#"{"name":"desktop"}"#.to_owned())
            .config();
        assert!(config.contains(r#"header = "Authorization: Bearer secret-token""#));
        assert!(config.contains(r#"header = "Content-Type: application/json""#));
        assert!(config.contains(r#"request = "POST""#));
        assert!(config.contains(r#"data-binary = "{\"name\":\"desktop\"}""#));
    }

    #[test]
    fn json_body_defaults_to_post_but_does_not_override_an_explicit_method() {
        let config = Request::new()
            .method("PUT")
            .json_body("{}".to_owned())
            .config();
        assert!(config.contains(r#"request = "PUT""#));
        assert!(!config.contains(r#"request = "POST""#));
    }

    #[test]
    fn json_surfaces_server_error_message() {
        let response = Response {
            status: 402,
            body: r#"{"message":"insufficient balance"}"#.to_owned(),
        };
        let error = response
            .json::<serde_json::Value>()
            .expect_err("should reject non-2xx");
        assert!(error.to_string().contains("insufficient balance"));
        assert!(error.to_string().contains("402"));
    }

    #[test]
    fn error_summary_falls_back_to_raw_text() {
        assert_eq!(error_summary("upstream exploded"), "upstream exploded");
        assert_eq!(error_summary(r#"{"error":"nope"}"#), "nope");
    }
}
