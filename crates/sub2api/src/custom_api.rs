//! Per-CLI custom API endpoints — bring your own endpoint.
//!
//! The managed gateway is one way to route an agent; this is the other: the
//! user pastes a base URL and an API key per CLI, and `global_config` writes
//! them into that CLI's own configuration file, exactly like the cloud
//! routing. OpenCode and Pi additionally take an optional model list, since
//! their native configs declare models explicitly.
//!
//! Stored in `~/.cheaprouter/custom-api.json`, desktop-local: routing no longer
//! involves the daemon at all. Earlier builds carried this configuration in
//! `DaemonSettings.extra`; [`migrate_from_extra`] adopts that once.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brand;
use crate::global_config::atomic_write_private;

/// Key the legacy daemon-settings transport used; read once for migration.
pub const LEGACY_SETTINGS_KEY: &str = "sub2apiCustomApi";

/// The CLIs a custom endpoint can be set for, in display order — the
/// intersection of what this app runs and what cc-switch manages.
pub const CUSTOM_API_PROVIDERS: [&str; 5] = ["claude", "codex", "grok", "opencode", "pi"];

/// One CLI's endpoint override.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CustomEndpoint {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// Model ids to declare, for the CLIs whose config lists models
    /// (OpenCode, Pi; Grok falls back to its stock pair when empty).
    #[serde(default)]
    pub models: Vec<String>,
}

impl CustomEndpoint {
    pub fn is_usable(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

/// Custom routing for every CLI that supports it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CustomApiConfig {
    #[serde(default)]
    pub claude: Option<CustomEndpoint>,
    #[serde(default)]
    pub codex: Option<CustomEndpoint>,
    #[serde(default)]
    pub grok: Option<CustomEndpoint>,
    #[serde(default)]
    pub opencode: Option<CustomEndpoint>,
    #[serde(default)]
    pub pi: Option<CustomEndpoint>,
}

impl CustomApiConfig {
    pub fn is_empty(&self) -> bool {
        CUSTOM_API_PROVIDERS
            .into_iter()
            .all(|provider| self.get(provider).is_none())
    }

    pub fn get(&self, provider_id: &str) -> Option<&CustomEndpoint> {
        match provider_id {
            "claude" => self.claude.as_ref(),
            "codex" => self.codex.as_ref(),
            "grok" => self.grok.as_ref(),
            "opencode" => self.opencode.as_ref(),
            "pi" => self.pi.as_ref(),
            _ => None,
        }
    }

    /// Set or clear one CLI's endpoint. Unknown ids are ignored.
    pub fn set(&mut self, provider_id: &str, endpoint: Option<CustomEndpoint>) {
        match provider_id {
            "claude" => self.claude = endpoint,
            "codex" => self.codex = endpoint,
            "grok" => self.grok = endpoint,
            "opencode" => self.opencode = endpoint,
            "pi" => self.pi = endpoint,
            _ => {}
        }
    }

    /// The endpoint that should route `provider_id`, if a usable one is set.
    pub fn endpoint_for(&self, provider_id: &str) -> Option<&CustomEndpoint> {
        self.get(provider_id).filter(|endpoint| endpoint.is_usable())
    }
}

/// Where the configuration lives.
pub fn config_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("custom-api.json"))
}

/// Load the stored configuration; absent or unreadable means "none set".
pub fn load() -> CustomApiConfig {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Persist the configuration (atomically, private).
pub fn save(config: &CustomApiConfig) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut encoded =
        serde_json::to_string_pretty(config).context("could not encode custom API settings")?;
    encoded.push('\n');
    atomic_write_private(&path, encoded.as_bytes())
}

/// Drain configuration left in `DaemonSettings.extra` by the injection-era
/// builds. Returns the parsed configuration when the key was present and
/// valid; the caller decides whether to save it (a newer local file wins).
/// `extra` is always cleaned of the legacy key.
pub fn migrate_from_extra(extra: &mut BTreeMap<String, Value>) -> Option<CustomApiConfig> {
    let value = extra.remove(LEGACY_SETTINGS_KEY)?;
    serde_json::from_value(value).ok()
}

// --- validation and connectivity -------------------------------------------

/// Why a typed base URL was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    /// Spaces or line breaks inside — a paste that picked up extra text.
    Whitespace,
    /// A scheme other than `http` / `https`.
    Scheme(String),
    NoHost,
}

/// Turn what the user typed into the origin the config writers expect.
///
/// Adds `https://` when no scheme was given, lowercases the scheme, strips
/// trailing slashes and a trailing `/v1` (each CLI's writer appends its own
/// version path, so a pasted `/v1` would double up), and refuses anything
/// that is not an `http(s)` URL with a host. Validation, not correction:
/// a typo in the host is still the user's to notice.
pub fn normalize_base_url(raw: &str) -> Result<String, UrlError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(UrlError::Empty);
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err(UrlError::Whitespace);
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let (scheme, rest) = with_scheme
        .split_once("://")
        .expect("a scheme separator was just ensured");
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(UrlError::Scheme(scheme));
    }
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() {
        return Err(UrlError::NoHost);
    }
    let mut path = rest.trim_end_matches('/').to_owned();
    if let Some(stripped) = path
        .strip_suffix("/v1")
        .or_else(|| path.strip_suffix("/V1"))
    {
        path = stripped.trim_end_matches('/').to_owned();
    }
    Ok(format!("{scheme}://{path}"))
}

/// How the connectivity test read the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// The models listing answered 2xx: the endpoint speaks the protocol
    /// and accepted the key.
    Ok,
    /// Reachable, but the key was refused (401 / 403).
    Unauthorized,
    /// Reachable, but some other HTTP error — a wrong path is the usual one.
    HttpError,
    /// No HTTP answer at all: DNS, TLS, proxy, or a dead host.
    Unreachable,
}

/// What the connectivity test found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub latency_ms: u128,
    pub status: Option<u16>,
    pub verdict: ProbeVerdict,
    /// The server's error text or the transport error, shortened.
    pub detail: String,
}

/// The request the connectivity test sends: the models listing, which every
/// API family serves and which costs nothing.
///
/// The path follows the CLI's protocol exactly as the config writers do —
/// Anthropic's SDK appends `/v1` to the root, OpenAI-style clients expect it
/// in the base — so a green test means the *routed* endpoint answers, not
/// merely that the host is up.
pub fn probe_request(
    provider_id: &str,
    base_url: &str,
    api_key: &str,
) -> (String, crate::http::Request) {
    let key = api_key.trim();
    let mut request = crate::http::Request::new().timeout_seconds(10);
    if provider_id == "claude" {
        let url = format!(
            "{}/v1/models",
            crate::gateway::anthropic_base_url(base_url)
        );
        request = request.header("anthropic-version", "2023-06-01");
        if !key.is_empty() {
            request = request.header("x-api-key", key);
        }
        (url, request)
    } else {
        let url = format!("{}/models", crate::gateway::openai_base_url(base_url));
        if !key.is_empty() {
            request = request.bearer(key);
        }
        (url, request)
    }
}

/// Run the connectivity test. Blocks for up to the request timeout; callers
/// run it off the UI thread.
pub fn probe_endpoint(provider_id: &str, base_url: &str, api_key: &str) -> ProbeResult {
    let (url, request) = probe_request(provider_id, base_url, api_key);
    let started = std::time::Instant::now();
    match request.send(&url) {
        Ok(response) => {
            let latency_ms = started.elapsed().as_millis();
            let verdict = match response.status {
                200..=299 => ProbeVerdict::Ok,
                401 | 403 => ProbeVerdict::Unauthorized,
                _ => ProbeVerdict::HttpError,
            };
            let detail = if verdict == ProbeVerdict::Ok {
                String::new()
            } else {
                crate::http::error_summary(&response.body)
            };
            ProbeResult {
                latency_ms,
                status: Some(response.status),
                verdict,
                detail,
            }
        }
        Err(error) => ProbeResult {
            latency_ms: started.elapsed().as_millis(),
            status: None,
            verdict: ProbeVerdict::Unreachable,
            detail: format!("{error:#}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_url_cases() {
        assert_eq!(
            normalize_base_url("api.example.com"),
            Ok("https://api.example.com".to_owned())
        );
        assert_eq!(
            normalize_base_url("  HTTPS://api.example.com/  "),
            Ok("https://api.example.com".to_owned())
        );
        assert_eq!(
            normalize_base_url("http://gw.local:8080/api/v1/"),
            Ok("http://gw.local:8080/api".to_owned())
        );
        assert_eq!(
            normalize_base_url("https://gw.example.org/V1"),
            Ok("https://gw.example.org".to_owned())
        );
        assert_eq!(normalize_base_url(""), Err(UrlError::Empty));
        assert_eq!(normalize_base_url("   "), Err(UrlError::Empty));
        assert_eq!(
            normalize_base_url("https://a.example.org key"),
            Err(UrlError::Whitespace)
        );
        assert_eq!(
            normalize_base_url("ftp://files.example.org"),
            Err(UrlError::Scheme("ftp".to_owned()))
        );
        assert_eq!(normalize_base_url("https:///path"), Err(UrlError::NoHost));
    }

    #[test]
    fn probe_endpoint_builds_per_cli_request() {
        let (url, request) = probe_request("claude", "https://gw.example.org/v1/", "sk-ant-key");
        assert_eq!(url, "https://gw.example.org/v1/models");
        assert!(
            request
                .header_lines()
                .iter()
                .any(|line| line == "x-api-key: sk-ant-key")
        );
        assert!(
            request
                .header_lines()
                .iter()
                .any(|line| line.starts_with("anthropic-version: "))
        );

        let (url, request) = probe_request("codex", "https://gw.example.org", "sk-openai");
        assert_eq!(url, "https://gw.example.org/v1/models");
        assert!(
            request
                .header_lines()
                .iter()
                .any(|line| line == "Authorization: Bearer sk-openai")
        );

        // Grok, OpenCode and Pi speak the OpenAI shape too.
        let (url, request) = probe_request("grok", "https://api.x.ai", "");
        assert_eq!(url, "https://api.x.ai/v1/models");
        assert!(request.header_lines().is_empty());
        assert_eq!(request.timeout(), Some(10));
    }

    fn endpoint(url: &str, key: &str) -> CustomEndpoint {
        CustomEndpoint {
            base_url: url.to_owned(),
            api_key: key.to_owned(),
            models: Vec::new(),
        }
    }

    #[test]
    fn set_get_and_usability() {
        let mut config = CustomApiConfig::default();
        assert!(config.is_empty());
        for provider in CUSTOM_API_PROVIDERS {
            config.set(provider, Some(endpoint("https://x.example.org", "sk")));
            assert!(config.endpoint_for(provider).is_some(), "{provider}");
        }
        assert!(!config.is_empty());
        for provider in CUSTOM_API_PROVIDERS {
            config.set(provider, None);
        }
        assert!(config.is_empty());
        // Unknown ids are ignored rather than panicking.
        config.set("gemini", Some(endpoint("https://x", "k")));
        assert!(config.is_empty());
        assert!(config.get("gemini").is_none());

        // Half-filled entries are readable (for the form) but not usable.
        let mut config = CustomApiConfig::default();
        config.set("pi", Some(endpoint("https://x.example.org", "")));
        assert!(config.get("pi").is_some());
        assert!(config.endpoint_for("pi").is_none());
    }

    #[test]
    fn serialization_round_trips_with_models() {
        let mut config = CustomApiConfig::default();
        config.set(
            "opencode",
            Some(CustomEndpoint {
                base_url: "https://x.example.org".into(),
                api_key: "sk".into(),
                models: vec!["m1".into(), "m2".into()],
            }),
        );
        let encoded = serde_json::to_string(&config).expect("encode");
        let decoded: CustomApiConfig = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, config);
        // Legacy payloads without `models` still parse.
        let legacy: CustomApiConfig = serde_json::from_str(
            r#"{"claude":{"base_url":"https://a.org","api_key":"k"}}"#,
        )
        .expect("legacy decode");
        assert_eq!(legacy.claude.as_ref().unwrap().models, Vec::<String>::new());
    }

    #[test]
    fn migration_drains_the_legacy_key() {
        let mut extra = BTreeMap::new();
        assert!(migrate_from_extra(&mut extra).is_none());
        extra.insert(
            LEGACY_SETTINGS_KEY.to_owned(),
            serde_json::json!({"claude": {"base_url": "https://a.org", "api_key": "k"}}),
        );
        let migrated = migrate_from_extra(&mut extra).expect("parse legacy payload");
        assert_eq!(migrated.claude.as_ref().unwrap().api_key, "k");
        assert!(!extra.contains_key(LEGACY_SETTINGS_KEY));
        // Garbage payloads still drain the key.
        extra.insert(LEGACY_SETTINGS_KEY.to_owned(), serde_json::json!("junk"));
        assert!(migrate_from_extra(&mut extra).is_none());
        assert!(!extra.contains_key(LEGACY_SETTINGS_KEY));
    }
}
