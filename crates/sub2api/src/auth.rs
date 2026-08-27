//! Managed cloud sign-in over a loopback redirect, and credential storage.
//!
//! # Why loopback
//!
//! The desktop opens the system browser at the service's login bridge and
//! listens on `127.0.0.1:<ephemeral>` for the redirect. Compared with
//! registering a custom URL scheme this needs no per-platform installer work
//! and no OS registration, and it behaves identically on macOS, Windows, and
//! Linux.
//!
//! # Why a relay page
//!
//! The bridge returns the session in the URL *fragment*
//! (`#access_token=…&refresh_token=…`). Browsers never send a fragment to the
//! server, so the loopback listener cannot read it directly. The callback
//! therefore serves a small page that copies `location.hash` back to the same
//! local server with a `POST`. The tokens stay on the loopback interface and
//! never traverse the network.
//!
//! # What is stored where
//!
//! OAuth access and refresh tokens stay in this process' own credential file
//! and are never handed to the daemon. Only the derived gateway API keys are
//! published into daemon settings, so a compromised daemon settings file cannot
//! be used to mint new credentials or read the account.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;

/// Path on the login bridge that starts the browser flow.
const LOGIN_BRIDGE_PATH: &str = "/auth/paseo";

/// Refresh this long before expiry so a request never races the deadline.
const REFRESH_SKEW_SECONDS: i64 = 120;

/// Stored session. Serialized to `~/.cheaprouter/cloud-account.json`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds at which `access_token` stops being valid.
    pub expires_at: i64,
    /// Service origin this session belongs to.
    pub endpoint: String,
    /// Gateway keys minted by the bridge at sign-in.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub claude_api_key: Option<String>,
    #[serde(default)]
    pub codex_api_key: Option<String>,
    /// Model group `api_key` is bound to, if the user picked one.
    ///
    /// Persisted alongside the key because the key alone does not say which
    /// group it belongs to: without this the settings page would show "account
    /// default" as selected after a restart while requests kept routing
    /// through the group.
    #[serde(default)]
    pub group_id: Option<i64>,
    /// Group `claude_api_key` is bound to, when the user picked one per CLI.
    #[serde(default)]
    pub claude_group_id: Option<i64>,
    /// Group `codex_api_key` is bound to, when the user picked one per CLI.
    #[serde(default)]
    pub codex_group_id: Option<i64>,
    /// The user turned gateway routing off without signing out. Stored
    /// inverted so the serde default (false) means the common case: signing
    /// in routes.
    #[serde(default)]
    pub routing_disabled: bool,
}

impl Credentials {
    pub fn path() -> Option<PathBuf> {
        brand::data_dir().map(|dir| dir.join("cloud-account.json"))
    }

    /// Load the stored session, or `None` when signed out.
    pub fn load() -> Option<Self> {
        let path = Self::path()?;
        let raw = std::fs::read_to_string(path).ok()?;
        let parsed: Self = serde_json::from_str(&raw).ok()?;
        if parsed.access_token.is_empty() || parsed.endpoint.is_empty() {
            return None;
        }
        Some(parsed)
    }

    /// Persist the session with owner-only permissions.
    pub fn save(&self) -> Result<()> {
        let path = Self::path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, body).with_context(|| format!("could not write {}", path.display()))?;
        restrict_to_owner(&path);
        Ok(())
    }

    /// Remove the stored session.
    pub fn clear() -> Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(anyhow::Error::from(error).context(format!("could not remove {}", path.display())))
            }
        }
    }

    /// True when the access token is expired or close enough that it should be
    /// refreshed before the next request.
    pub fn needs_refresh(&self, now_unix: i64) -> bool {
        self.expires_at - REFRESH_SKEW_SECONDS <= now_unix
    }

    /// Apply a refreshed token pair, keeping the old refresh token when the
    /// service does not rotate it.
    pub fn apply_refresh(&mut self, pair: &crate::client::TokenPair, now_unix: i64) {
        self.access_token = pair.access_token.clone();
        if !pair.refresh_token.is_empty() {
            self.refresh_token = pair.refresh_token.clone();
        }
        self.expires_at = now_unix + pair.expires_in;
    }
}

/// Current unix time in seconds.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(unix)]
fn restrict_to_owner(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &std::path::Path) {
    // Windows inherits the user profile ACL, which is already owner-only.
}

/// An in-progress browser sign-in.
pub struct LoginFlow {
    listener: TcpListener,
    endpoint: String,
    port: u16,
}

impl LoginFlow {
    /// Bind the loopback listener. Binding before opening the browser means the
    /// redirect can never arrive at a closed port.
    pub fn start(endpoint: &str) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint)?;
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .context("could not open a loopback port for the sign-in redirect")?;
        let port = listener.local_addr()?.port();
        listener
            .set_nonblocking(true)
            .context("could not configure the loopback listener")?;
        Ok(Self {
            listener,
            endpoint,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// URL to open in the user's browser.
    pub fn login_url(&self) -> String {
        build_login_url(&self.endpoint, self.port)
    }

    /// Block until the browser delivers a session, or the deadline passes.
    pub fn wait(&self, timeout: Duration) -> Result<Credentials> {
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for the browser sign-in"));
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(credentials) = self.serve(stream)? {
                        return Ok(credentials);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    return Err(anyhow::Error::from(error)
                        .context("the loopback listener failed while awaiting sign-in"));
                }
            }
        }
    }

    /// Handle one connection. Returns `Some` once the relay delivers a session.
    fn serve(&self, mut stream: TcpStream) -> Result<Option<Credentials>> {
        stream.set_nonblocking(false).ok();
        stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let Some(request) = read_request(&mut stream)? else {
            return Ok(None);
        };
        if request.method == "POST" && request.path.starts_with("/deliver") {
            match credentials_from_fragment(&request.body, &self.endpoint) {
                Ok(credentials) => {
                    respond(&mut stream, "200 OK", "text/plain; charset=utf-8", "ok");
                    return Ok(Some(credentials));
                }
                Err(error) => {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "text/plain; charset=utf-8",
                        &error.to_string(),
                    );
                    return Ok(None);
                }
            }
        }
        respond(&mut stream, "200 OK", "text/html; charset=utf-8", RELAY_PAGE);
        Ok(None)
    }
}

/// Build the bridge URL the browser opens.
///
/// `endpoint` is echoed back in the callback, which lets the relay verify that
/// the session belongs to the service the user actually chose.
pub fn build_login_url(endpoint: &str, port: u16) -> String {
    let redirect = format!("http://127.0.0.1:{port}/callback");
    format!(
        "{endpoint}{LOGIN_BRIDGE_PATH}?endpoint={}&redirect_to={}",
        percent_encode(endpoint),
        percent_encode(&redirect)
    )
}

/// Turn a callback fragment into credentials.
///
/// Accepts the fragment with or without its leading `#`.
pub fn credentials_from_fragment(fragment: &str, expected_endpoint: &str) -> Result<Credentials> {
    let params = parse_query(fragment.trim().trim_start_matches('#'));
    let take = |key: &str| params.get(key).map(String::as_str).unwrap_or("").trim();

    let access_token = take("access_token");
    let refresh_token = take("refresh_token");
    let expires_in: i64 = take("expires_in").parse().unwrap_or_default();
    let endpoint = take("endpoint");

    if access_token.is_empty() || refresh_token.is_empty() || expires_in <= 0 {
        return Err(anyhow!("the sign-in callback did not include a session"));
    }

    let endpoint = if endpoint.is_empty() {
        expected_endpoint.to_owned()
    } else {
        let normalized = normalize_endpoint(endpoint)?;
        // A callback naming a different service would mean the browser was
        // redirected somewhere we did not send it.
        if normalized != expected_endpoint {
            return Err(anyhow!(
                "the sign-in callback came from {normalized}, not {expected_endpoint}"
            ));
        }
        normalized
    };

    let optional = |key: &str| {
        let value = take(key);
        (!value.is_empty()).then(|| value.to_owned())
    };

    Ok(Credentials {
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
        expires_at: now_unix() + expires_in,
        endpoint,
        api_key: optional("api_key"),
        claude_api_key: optional("claude_api_key"),
        codex_api_key: optional("codex_api_key"),
        // The bridge hands out account-level keys; group bindings are a
        // later, explicit choice in the app.
        group_id: None,
        claude_group_id: None,
        codex_group_id: None,
        // Signing in is an explicit request to route through the service.
        routing_disabled: false,
    })
}

/// Normalize a service origin: absolute http(s) URL, no trailing slash.
pub fn normalize_endpoint(endpoint: &str) -> Result<String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(anyhow!("the service address is required"));
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(anyhow!("the service address must start with http:// or https://"));
    }
    let without_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    if without_scheme.is_empty() || without_scheme.starts_with('/') {
        return Err(anyhow!("the service address is missing a host"));
    }
    Ok(trimmed.to_owned())
}

/// The page served at the loopback callback.
///
/// It copies the fragment — which the browser withheld from the request — back
/// to this server, then tells the user they can return to the app.
const RELAY_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Signing in…</title>
<style>
body{font:15px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif;margin:0;
display:flex;align-items:center;justify-content:center;height:100vh;
background:#0f1115;color:#e6e8eb}
main{text-align:center;max-width:28rem;padding:2rem}
h1{font-size:1.1rem;font-weight:600;margin:0 0 .5rem}
p{margin:0;color:#9aa1ab}
</style></head>
<body><main id="status"><h1>Finishing sign-in…</h1><p>You can close this tab in a moment.</p></main>
<script>
(function () {
  var hash = window.location.hash || "";
  var status = document.getElementById("status");
  function show(title, detail) {
    status.innerHTML = "<h1></h1><p></p>";
    status.firstChild.textContent = title;
    status.lastChild.textContent = detail;
  }
  if (!hash || hash.length < 2) {
    show("Sign-in did not complete", "No session was returned. Start again from the app.");
    return;
  }
  fetch("/deliver", { method: "POST", body: hash })
    .then(function (response) {
      if (response.ok) {
        show("Signed in", "You can close this tab and return to the app.");
      } else {
        return response.text().then(function (text) {
          show("Sign-in failed", text || "The app rejected the session.");
        });
      }
    })
    .catch(function () {
      show("Sign-in failed", "The app is no longer listening. Start again from the app.");
    });
})();
</script></body></html>"#;

struct HttpRequest {
    method: String,
    path: String,
    body: String,
}

/// Read one request. Returns `None` when the peer sends nothing usable.
fn read_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream.try_clone().context("could not read the request")?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return Ok(None);
    };
    let Some(path) = parts.next() else {
        return Ok(None);
    };

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    // Cap the body: the relay posts a fragment, never bulk data.
    let mut body = vec![0u8; content_length.min(64 * 1024)];
    if !body.is_empty() {
        reader.read_exact(&mut body).context("could not read the request body")?;
    }

    Ok(Some(HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        body: String::from_utf8_lossy(&body).into_owned(),
    }))
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Parse `a=1&b=2` into a map, percent-decoding both sides.
fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((percent_decode(key), percent_decode(value)))
        })
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(decoded) => {
                        out.push(decoded);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode everything outside the unreserved set.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = "https://cloud.example.org";

    #[test]
    fn normalizes_endpoints() {
        assert_eq!(normalize_endpoint("https://a.org/").unwrap(), "https://a.org");
        assert_eq!(normalize_endpoint("  http://a.org  ").unwrap(), "http://a.org");
        assert!(normalize_endpoint("").is_err());
        assert!(normalize_endpoint("a.org").is_err());
        assert!(normalize_endpoint("ftp://a.org").is_err());
        assert!(normalize_endpoint("https://").is_err());
    }

    #[test]
    fn login_url_carries_an_encoded_loopback_redirect() {
        let url = build_login_url(SERVICE, 51789);
        assert!(url.starts_with("https://cloud.example.org/auth/paseo?"));
        assert!(url.contains("endpoint=https%3A%2F%2Fcloud.example.org"));
        assert!(url.contains("redirect_to=http%3A%2F%2F127.0.0.1%3A51789%2Fcallback"));
    }

    #[test]
    fn parses_a_complete_callback_fragment() {
        let fragment = "#access_token=at&refresh_token=rt&expires_in=3600\
                        &endpoint=https%3A%2F%2Fcloud.example.org\
                        &api_key=sk-gateway&claude_api_key=sk-claude&codex_api_key=sk-codex";
        let credentials = credentials_from_fragment(fragment, SERVICE).expect("parse");
        assert_eq!(credentials.access_token, "at");
        assert_eq!(credentials.refresh_token, "rt");
        assert_eq!(credentials.endpoint, SERVICE);
        assert_eq!(credentials.api_key.as_deref(), Some("sk-gateway"));
        assert_eq!(credentials.claude_api_key.as_deref(), Some("sk-claude"));
        assert_eq!(credentials.codex_api_key.as_deref(), Some("sk-codex"));
        assert!(credentials.expires_at > now_unix());
    }

    #[test]
    fn fragment_without_leading_hash_is_accepted() {
        let credentials =
            credentials_from_fragment("access_token=at&refresh_token=rt&expires_in=60", SERVICE)
                .expect("parse");
        assert_eq!(credentials.endpoint, SERVICE);
        assert!(credentials.api_key.is_none());
    }

    #[test]
    fn incomplete_callbacks_are_rejected() {
        for fragment in [
            "",
            "#access_token=at",
            "#access_token=at&refresh_token=rt",
            "#access_token=at&refresh_token=rt&expires_in=0",
            "#access_token=&refresh_token=rt&expires_in=60",
        ] {
            assert!(
                credentials_from_fragment(fragment, SERVICE).is_err(),
                "should reject: {fragment}"
            );
        }
    }

    #[test]
    fn a_callback_from_another_service_is_rejected() {
        let fragment = "#access_token=at&refresh_token=rt&expires_in=60\
                        &endpoint=https%3A%2F%2Fevil.example.net";
        let error = credentials_from_fragment(fragment, SERVICE).expect_err("should reject");
        assert!(error.to_string().contains("evil.example.net"));
    }

    #[test]
    fn group_binding_survives_a_round_trip() {
        // Without this the settings page shows "account default" as selected
        // after a restart while requests keep routing through the group.
        let credentials = Credentials {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: 42,
            endpoint: "https://a.org".into(),
            api_key: Some("sk-group".into()),
            group_id: Some(7),
            ..Credentials::default()
        };
        let encoded = serde_json::to_string(&credentials).expect("encode");
        let decoded: Credentials = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, credentials);
        assert_eq!(decoded.group_id, Some(7));
    }

    #[test]
    fn credentials_written_before_group_support_still_load() {
        // Files written by an earlier build have no `group_id`.
        let decoded: Credentials = serde_json::from_str(
            r#"{"access_token":"at","refresh_token":"rt","expires_at":1,"endpoint":"https://a.org"}"#,
        )
        .expect("decode");
        assert_eq!(decoded.group_id, None);
        assert_eq!(decoded.access_token, "at");
    }

    #[test]
    fn refresh_window_opens_before_expiry() {
        let credentials = Credentials {
            expires_at: 1_000_000,
            ..Credentials::default()
        };
        assert!(!credentials.needs_refresh(1_000_000 - REFRESH_SKEW_SECONDS - 1));
        assert!(credentials.needs_refresh(1_000_000 - REFRESH_SKEW_SECONDS));
        assert!(credentials.needs_refresh(1_000_001));
    }

    #[test]
    fn refresh_keeps_the_old_token_when_the_service_does_not_rotate_it() {
        let mut credentials = Credentials {
            access_token: "old".into(),
            refresh_token: "keep-me".into(),
            expires_at: 0,
            ..Credentials::default()
        };
        credentials.apply_refresh(
            &crate::client::TokenPair {
                access_token: "new".into(),
                refresh_token: String::new(),
                expires_in: 3600,
            },
            1_000,
        );
        assert_eq!(credentials.access_token, "new");
        assert_eq!(credentials.refresh_token, "keep-me");
        assert_eq!(credentials.expires_at, 4_600);
    }

    #[test]
    fn percent_round_trip() {
        let raw = "https://a.org/x?y=1 2&z=✓";
        assert_eq!(percent_decode(&percent_encode(raw)), raw);
        assert_eq!(percent_decode("a+b"), "a b");
        // A stray percent must not panic or eat the rest of the string.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn relay_page_posts_the_fragment_back() {
        assert!(RELAY_PAGE.contains("location.hash"));
        assert!(RELAY_PAGE.contains(r#"fetch("/deliver""#));
    }

    #[test]
    fn flow_binds_loopback_and_builds_a_matching_url() {
        let flow = LoginFlow::start(SERVICE).expect("bind");
        assert!(flow.port() > 0);
        assert!(flow.login_url().contains(&format!("127.0.0.1%3A{}", flow.port())));
    }

    #[test]
    fn wait_times_out_without_a_callback() {
        let flow = LoginFlow::start(SERVICE).expect("bind");
        let error = flow
            .wait(Duration::from_millis(120))
            .expect_err("should time out");
        assert!(error.to_string().contains("timed out"));
    }
}
