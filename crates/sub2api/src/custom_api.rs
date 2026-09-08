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
/// Providers whose routing the user can point somewhere else.
///
/// `native` is the built-in agent. It is the only entry that is not a CLI -
/// nothing is written to a config file on its behalf; the driver reads the
/// endpoint directly at session start.
pub const CUSTOM_API_PROVIDERS: [&str; 6] =
    ["native", "claude", "codex", "grok", "opencode", "pi"];

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

/// One saved endpoint configuration for a CLI — a "profile". A CLI can keep
/// several (the gateway, an official key, another relay) and route through
/// exactly one at a time.
///
/// The endpoint fields are flattened so a profile serializes as the old
/// single-endpoint object plus `id`/`name`/`candidate_urls`/`auto_select`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct EndpointProfile {
    /// Stable identity, `p-<unix ms>-<n>`; never shown.
    #[serde(default)]
    pub id: String,
    /// Display name; the app fills in a default when empty.
    #[serde(default)]
    pub name: String,
    /// The routed endpoint. `base_url` is the URL currently in use.
    #[serde(flatten)]
    pub endpoint: CustomEndpoint,
    /// Alternate origins for the same service (mirror domains, a direct IP),
    /// normalized. Always contains `base_url` once it is set.
    #[serde(default)]
    pub candidate_urls: Vec<String>,
    /// After a speed test, switch `base_url` to the fastest candidate that
    /// answered successfully.
    #[serde(default)]
    pub auto_select: bool,
}

impl EndpointProfile {
    /// A fresh, empty profile with a new id.
    pub fn new(name: &str) -> Self {
        Self {
            id: next_profile_id(),
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// Add a candidate origin. Returns false when it was already listed.
    pub fn add_candidate(&mut self, url: &str) -> bool {
        let url = url.trim();
        if url.is_empty() || self.candidate_urls.iter().any(|known| known == url) {
            return false;
        }
        self.candidate_urls.push(url.to_owned());
        true
    }

    /// Drop a candidate. Removing the URL in use moves `base_url` to the
    /// first remaining candidate (or clears it when none is left).
    pub fn remove_candidate(&mut self, url: &str) {
        self.candidate_urls.retain(|known| known != url);
        if self.endpoint.base_url == url {
            self.endpoint.base_url = self.candidate_urls.first().cloned().unwrap_or_default();
        }
    }

    /// Route through `url`, listing it as a candidate if it was not yet.
    pub fn select_url(&mut self, url: &str) {
        let url = url.trim();
        self.add_candidate(url);
        self.endpoint.base_url = url.to_owned();
    }

    /// Keep the invariant that the URL in use is one of the candidates.
    fn normalize(&mut self) {
        if self.id.is_empty() {
            self.id = next_profile_id();
        }
        let base_url = self.endpoint.base_url.trim().to_owned();
        if !base_url.is_empty() {
            self.add_candidate(&base_url);
        }
    }
}

/// Monotonic within a process, so two profiles created in the same
/// millisecond still get distinct ids.
fn next_profile_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("p-{millis}-{n}")
}

/// Every profile one CLI has, and which of them routes it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderProfiles {
    #[serde(default)]
    pub profiles: Vec<EndpointProfile>,
    /// Id of the routing profile. `None` only while there are no profiles.
    #[serde(default)]
    pub active: Option<String>,
}

impl ProviderProfiles {
    fn from_legacy(endpoint: CustomEndpoint) -> Self {
        let empty = endpoint.base_url.trim().is_empty()
            && endpoint.api_key.trim().is_empty()
            && endpoint.models.is_empty();
        if empty {
            return Self::default();
        }
        let mut profile = EndpointProfile {
            id: "legacy".to_owned(),
            name: String::new(),
            endpoint,
            candidate_urls: Vec::new(),
            auto_select: false,
        };
        profile.normalize();
        Self {
            active: Some(profile.id.clone()),
            profiles: vec![profile],
        }
    }

    /// Repair what a hand-edited or older file may have left inconsistent:
    /// missing ids, an `active` that points nowhere, a base URL absent from
    /// its own candidates.
    fn normalize(&mut self) {
        for profile in &mut self.profiles {
            profile.normalize();
        }
        let active_exists = self
            .active
            .as_ref()
            .is_some_and(|id| self.profiles.iter().any(|profile| profile.id == *id));
        if !active_exists {
            self.active = self.profiles.first().map(|profile| profile.id.clone());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn active_profile(&self) -> Option<&EndpointProfile> {
        let id = self.active.as_deref()?;
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn active_profile_mut(&mut self) -> Option<&mut EndpointProfile> {
        let id = self.active.clone()?;
        self.profiles.iter_mut().find(|profile| profile.id == id)
    }

    pub fn find(&self, id: &str) -> Option<&EndpointProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut EndpointProfile> {
        self.profiles.iter_mut().find(|profile| profile.id == id)
    }

    /// Add an empty profile and make it the active one. Returns its id.
    pub fn add(&mut self, name: &str) -> String {
        let profile = EndpointProfile::new(name);
        let id = profile.id.clone();
        self.profiles.push(profile);
        self.active = Some(id.clone());
        id
    }

    /// Copy a profile (endpoint, candidates, auto-select) under a new name
    /// and make the copy active. Returns the new id.
    pub fn duplicate(&mut self, id: &str, name: &str) -> Option<String> {
        let mut copy = self.find(id)?.clone();
        copy.id = next_profile_id();
        copy.name = name.to_owned();
        let new_id = copy.id.clone();
        let position = self
            .profiles
            .iter()
            .position(|profile| profile.id == id)
            .map_or(self.profiles.len(), |index| index + 1);
        self.profiles.insert(position, copy);
        self.active = Some(new_id.clone());
        Some(new_id)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        match self.find_mut(id) {
            Some(profile) => {
                profile.name = name.trim().to_owned();
                true
            }
            None => false,
        }
    }

    /// Remove a profile. Removing the active one activates the profile that
    /// followed it (or the last remaining). Returns false for unknown ids.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(index) = self.profiles.iter().position(|profile| profile.id == id) else {
            return false;
        };
        self.profiles.remove(index);
        if self.active.as_deref() == Some(id) {
            self.active = self
                .profiles
                .get(index)
                .or_else(|| self.profiles.last())
                .map(|profile| profile.id.clone());
        }
        true
    }

    /// Route through `id`. Returns false for unknown ids.
    pub fn set_active(&mut self, id: &str) -> bool {
        if self.find(id).is_none() {
            return false;
        }
        self.active = Some(id.to_owned());
        true
    }
}

/// Accept both shapes of a CLI's slot: the current `{profiles, active}`
/// object and the pre-profile single endpoint `{base_url, api_key, models}`
/// (or `null`). Older files upgrade on the first save, since serialization
/// always writes the current shape.
fn deserialize_slot<'de, D>(deserializer: D) -> Result<ProviderProfiles, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Order matters: `CustomEndpoint` has only defaulted fields and would
    // match any object, so the shape that needs `profiles` is tried first.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SlotRepr {
        Profiles {
            profiles: Vec<EndpointProfile>,
            #[serde(default)]
            active: Option<String>,
        },
        Legacy(CustomEndpoint),
    }

    let mut slot = match Option::<SlotRepr>::deserialize(deserializer)? {
        None => ProviderProfiles::default(),
        Some(SlotRepr::Profiles { profiles, active }) => ProviderProfiles { profiles, active },
        Some(SlotRepr::Legacy(endpoint)) => ProviderProfiles::from_legacy(endpoint),
    };
    slot.normalize();
    Ok(slot)
}

/// Custom routing for every CLI that supports it: per CLI, the saved
/// profiles and the one in use.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CustomApiConfig {
    /// The built-in agent. Listed first because it is the default provider,
    /// and the only one whose endpoint is read at session start rather than
    /// written into some CLI's own configuration file.
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub native: ProviderProfiles,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub claude: ProviderProfiles,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub codex: ProviderProfiles,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub grok: ProviderProfiles,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub opencode: ProviderProfiles,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub pi: ProviderProfiles,
}

impl CustomApiConfig {
    pub fn is_empty(&self) -> bool {
        CUSTOM_API_PROVIDERS
            .into_iter()
            .all(|provider| self.get(provider).is_none())
    }

    /// One CLI's profiles. `None` for ids this feature does not cover.
    pub fn profiles(&self, provider_id: &str) -> Option<&ProviderProfiles> {
        match provider_id {
            "native" => Some(&self.native),
            "claude" => Some(&self.claude),
            "codex" => Some(&self.codex),
            "grok" => Some(&self.grok),
            "opencode" => Some(&self.opencode),
            "pi" => Some(&self.pi),
            _ => None,
        }
    }

    pub fn profiles_mut(&mut self, provider_id: &str) -> Option<&mut ProviderProfiles> {
        match provider_id {
            "native" => Some(&mut self.native),
            "claude" => Some(&mut self.claude),
            "codex" => Some(&mut self.codex),
            "grok" => Some(&mut self.grok),
            "opencode" => Some(&mut self.opencode),
            "pi" => Some(&mut self.pi),
            _ => None,
        }
    }

    /// The active profile's endpoint — what the form shows and the writers
    /// consult. `None` while the CLI has no profile at all.
    pub fn get(&self, provider_id: &str) -> Option<&CustomEndpoint> {
        self.profiles(provider_id)?
            .active_profile()
            .map(|profile| &profile.endpoint)
    }

    /// The active profile itself, when there is one.
    pub fn active_profile(&self, provider_id: &str) -> Option<&EndpointProfile> {
        self.profiles(provider_id)?.active_profile()
    }

    /// Write the active profile's endpoint, creating a first profile when
    /// the CLI has none; `None` deletes the active profile (the next one, if
    /// any, takes over). Unknown ids are ignored.
    pub fn set(&mut self, provider_id: &str, endpoint: Option<CustomEndpoint>) {
        let Some(slot) = self.profiles_mut(provider_id) else {
            return;
        };
        match endpoint {
            Some(endpoint) => {
                if slot.active_profile().is_none() {
                    slot.add("");
                }
                let profile = slot
                    .active_profile_mut()
                    .expect("a profile was just added");
                profile.endpoint = endpoint;
                profile.normalize();
            }
            None => {
                if let Some(id) = slot.active.clone() {
                    slot.remove(&id);
                }
            }
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
    /// The models listing itself when the probe succeeded (for filling the
    /// model list), empty otherwise.
    pub body: String,
}

/// Seconds a single probe waits before it is called unreachable.
pub const PROBE_TIMEOUT_SECS: u32 = 10;

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
    probe_request_with_timeout(provider_id, base_url, api_key, PROBE_TIMEOUT_SECS)
}

/// [`probe_request`] with an explicit timeout, for the speed test.
pub fn probe_request_with_timeout(
    provider_id: &str,
    base_url: &str,
    api_key: &str,
    timeout_secs: u32,
) -> (String, crate::http::Request) {
    let key = api_key.trim();
    let mut request = crate::http::Request::new().timeout_seconds(timeout_secs);
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
    probe_endpoint_with_timeout(provider_id, base_url, api_key, PROBE_TIMEOUT_SECS)
}

/// [`probe_endpoint`] with an explicit timeout.
pub fn probe_endpoint_with_timeout(
    provider_id: &str,
    base_url: &str,
    api_key: &str,
    timeout_secs: u32,
) -> ProbeResult {
    let (url, request) = probe_request_with_timeout(provider_id, base_url, api_key, timeout_secs);
    let started = std::time::Instant::now();
    match request.send(&url) {
        Ok(response) => {
            let latency_ms = started.elapsed().as_millis();
            let verdict = match response.status {
                200..=299 => ProbeVerdict::Ok,
                401 | 403 => ProbeVerdict::Unauthorized,
                _ => ProbeVerdict::HttpError,
            };
            let (detail, body) = if verdict == ProbeVerdict::Ok {
                (String::new(), response.body)
            } else {
                (crate::http::error_summary(&response.body), String::new())
            };
            ProbeResult {
                latency_ms,
                status: Some(response.status),
                verdict,
                detail,
                body,
            }
        }
        Err(error) => ProbeResult {
            latency_ms: started.elapsed().as_millis(),
            status: None,
            verdict: ProbeVerdict::Unreachable,
            detail: format!("{error:#}"),
            body: String::new(),
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
        assert_eq!(legacy.get("claude").unwrap().models, Vec::<String>::new());
    }

    #[test]
    fn legacy_file_migrates_to_one_active_profile() {
        // The exact shape the previous release wrote, `null` slots included.
        let raw = r#"{
  "claude": {
    "base_url": "https://a.example.org",
    "api_key": "sk-a",
    "models": []
  },
  "codex": null,
  "grok": null,
  "opencode": {
    "base_url": "https://o.example.org",
    "api_key": "sk-o",
    "models": ["m1"]
  },
  "pi": null
}"#;
        let config: CustomApiConfig = serde_json::from_str(raw).expect("legacy decode");
        let claude = config.profiles("claude").unwrap();
        assert_eq!(claude.profiles.len(), 1);
        assert_eq!(claude.active.as_deref(), Some("legacy"));
        let profile = claude.active_profile().unwrap();
        assert_eq!(profile.endpoint.base_url, "https://a.example.org");
        assert_eq!(profile.endpoint.api_key, "sk-a");
        assert_eq!(profile.candidate_urls, vec!["https://a.example.org".to_owned()]);
        assert!(!profile.auto_select);
        assert!(config.profiles("codex").unwrap().is_empty());
        assert!(config.get("codex").is_none());
        assert_eq!(config.get("opencode").unwrap().models, vec!["m1".to_owned()]);
        // An all-empty legacy entry is not worth a profile.
        let config: CustomApiConfig =
            serde_json::from_str(r#"{"pi":{"base_url":"","api_key":""}}"#).expect("decode");
        assert!(config.get("pi").is_none());
    }

    #[test]
    fn legacy_then_save_writes_new_shape() {
        let config: CustomApiConfig =
            serde_json::from_str(r#"{"claude":{"base_url":"https://a.org","api_key":"k"}}"#)
                .expect("decode");
        let encoded = serde_json::to_string(&config).expect("encode");
        assert!(encoded.contains(r#""profiles":[{"id":"legacy""#), "{encoded}");
        assert!(encoded.contains(r#""active":"legacy""#), "{encoded}");
        let again: CustomApiConfig = serde_json::from_str(&encoded).expect("re-decode");
        assert_eq!(again, config);
    }

    #[test]
    fn new_shape_round_trips_and_repairs_active() {
        let raw = r#"{"codex":{"profiles":[
            {"id":"one","name":"One","base_url":"https://1.org","api_key":"k1"},
            {"id":"two","name":"Two","base_url":"https://2.org","api_key":"k2",
             "candidate_urls":["https://2.org","https://2b.org"],"auto_select":true}
        ],"active":"missing"}}"#;
        let config: CustomApiConfig = serde_json::from_str(raw).expect("decode");
        let codex = config.profiles("codex").unwrap();
        // An `active` that points nowhere falls back to the first profile.
        assert_eq!(codex.active.as_deref(), Some("one"));
        assert_eq!(codex.find("one").unwrap().candidate_urls, vec!["https://1.org".to_owned()]);
        assert!(codex.find("two").unwrap().auto_select);
        let encoded = serde_json::to_string(&config).expect("encode");
        let decoded: CustomApiConfig = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, config);
    }

    #[test]
    fn set_none_deletes_active_and_activates_next() {
        let mut config = CustomApiConfig::default();
        let slot = config.profiles_mut("grok").unwrap();
        let first = slot.add("First");
        slot.find_mut(&first).unwrap().endpoint = endpoint("https://1.org", "k1");
        let second = slot.add("Second");
        slot.find_mut(&second).unwrap().endpoint = endpoint("https://2.org", "k2");
        let third = slot.add("Third");
        slot.find_mut(&third).unwrap().endpoint = endpoint("https://3.org", "k3");
        assert!(slot.set_active(&second));
        assert_eq!(config.get("grok").unwrap().base_url, "https://2.org");

        config.set("grok", None);
        // The profile after the removed one takes over.
        assert_eq!(config.get("grok").unwrap().base_url, "https://3.org");
        config.set("grok", None);
        assert_eq!(config.get("grok").unwrap().base_url, "https://1.org");
        config.set("grok", None);
        assert!(config.get("grok").is_none());
        assert!(config.is_empty());
    }

    #[test]
    fn set_some_on_empty_slot_creates_profile_and_lists_url() {
        let mut config = CustomApiConfig::default();
        config.set("claude", Some(endpoint("https://a.org", "k")));
        let claude = config.profiles("claude").unwrap();
        assert_eq!(claude.profiles.len(), 1);
        let profile = claude.active_profile().unwrap();
        assert!(!profile.id.is_empty());
        assert_eq!(profile.candidate_urls, vec!["https://a.org".to_owned()]);
        // A second set on the same slot edits the active profile in place.
        config.set("claude", Some(endpoint("https://b.org", "k2")));
        let claude = config.profiles("claude").unwrap();
        assert_eq!(claude.profiles.len(), 1);
        assert_eq!(
            claude.active_profile().unwrap().candidate_urls,
            vec!["https://a.org".to_owned(), "https://b.org".to_owned()]
        );
    }

    #[test]
    fn select_url_and_remove_candidate_keep_base_url_consistent() {
        let mut profile = EndpointProfile::new("P");
        profile.endpoint = endpoint("https://a.org", "k");
        profile.normalize();
        profile.select_url("https://b.org");
        assert_eq!(profile.endpoint.base_url, "https://b.org");
        assert_eq!(
            profile.candidate_urls,
            vec!["https://a.org".to_owned(), "https://b.org".to_owned()]
        );
        assert!(!profile.add_candidate("https://b.org"));
        assert!(!profile.add_candidate("  "));
        profile.remove_candidate("https://b.org");
        assert_eq!(profile.endpoint.base_url, "https://a.org");
        profile.remove_candidate("https://a.org");
        assert_eq!(profile.endpoint.base_url, "");
        assert!(profile.candidate_urls.is_empty());
    }

    #[test]
    fn duplicate_rename_and_ids_are_distinct() {
        let mut slot = ProviderProfiles::default();
        let original = slot.add("Gateway");
        {
            let profile = slot.find_mut(&original).unwrap();
            profile.endpoint = endpoint("https://g.org", "k");
            profile.add_candidate("https://g2.org");
            profile.auto_select = true;
        }
        let copy = slot.duplicate(&original, "Gateway copy").expect("copy");
        assert_ne!(copy, original);
        assert_eq!(slot.active.as_deref(), Some(copy.as_str()));
        let copied = slot.find(&copy).unwrap();
        assert_eq!(copied.name, "Gateway copy");
        assert_eq!(copied.endpoint.api_key, "k");
        assert_eq!(copied.candidate_urls, vec!["https://g2.org".to_owned()]);
        assert!(copied.auto_select);
        // The copy sits right after its source.
        assert_eq!(slot.profiles[1].id, copy);
        assert!(slot.rename(&copy, "  Mirror  "));
        assert_eq!(slot.find(&copy).unwrap().name, "Mirror");
        assert!(!slot.rename("nope", "x"));
        assert!(slot.duplicate("nope", "x").is_none());
        assert!(!slot.remove("nope"));
        assert!(!slot.set_active("nope"));
        let a = next_profile_id();
        let b = next_profile_id();
        assert_ne!(a, b);
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
        assert_eq!(migrated.get("claude").unwrap().api_key, "k");
        assert!(!extra.contains_key(LEGACY_SETTINGS_KEY));
        // Garbage payloads still drain the key.
        extra.insert(LEGACY_SETTINGS_KEY.to_owned(), serde_json::json!("junk"));
        assert!(migrate_from_extra(&mut extra).is_none());
        assert!(!extra.contains_key(LEGACY_SETTINGS_KEY));
    }
}
