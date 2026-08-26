//! Routing agent CLIs through the managed gateway.
//!
//! # Transport
//!
//! The desktop signs in; the daemon spawns agents. The gateway keys therefore
//! have to cross the process boundary. They travel inside
//! `DaemonSettings.extra`, upstream's `#[serde(flatten)]` escape hatch for
//! unknown keys, under a single namespaced key. That means the wire contract in
//! `waku-protocol` needs no change at all: an upstream daemon round-trips our
//! key untouched, and an upstream desktop ignores it.
//!
//! # Isolation
//!
//! Routing is applied as process environment at spawn time. The user's own
//! `~/.claude` and `~/.codex` are never rewritten, so signing out — or running
//! an agent outside this app — restores their personal configuration with no
//! cleanup step. Codex, which has no base-URL environment variable, gets a
//! generated `CODEX_HOME` directory instead of an edit to the real one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brand;

/// Key under which routing configuration lives in `DaemonSettings.extra`.
///
/// Namespaced so it cannot collide with a future upstream setting.
pub const SETTINGS_KEY: &str = "sub2apiCloudGateway";

/// Codex model used when the account does not pin one.
const DEFAULT_CODEX_MODEL: &str = "gpt-5.4";

/// What the daemon needs in order to route agents through the gateway.
///
/// Deliberately holds gateway API keys only — never the OAuth access or refresh
/// token, which stay in the desktop's credential file.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GatewayConfig {
    /// Routing is off while false, even when keys are present.
    #[serde(default)]
    pub enabled: bool,
    /// Service origin, e.g. `https://cloud.example.org`.
    #[serde(default)]
    pub endpoint: String,
    /// Fallback key used when a provider has no dedicated one.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub claude_api_key: Option<String>,
    #[serde(default)]
    pub codex_api_key: Option<String>,
    /// Codex model to pin. Falls back to [`DEFAULT_CODEX_MODEL`].
    #[serde(default)]
    pub codex_model: Option<String>,
}

impl GatewayConfig {
    /// Read the configuration out of daemon settings, if present and usable.
    pub fn from_extra(extra: &BTreeMap<String, Value>) -> Option<Self> {
        let value = extra.get(SETTINGS_KEY)?;
        let config: Self = serde_json::from_value(value.clone()).ok()?;
        config.is_usable().then_some(config)
    }

    /// Publish the configuration into daemon settings.
    pub fn write_into(&self, extra: &mut BTreeMap<String, Value>) -> Result<()> {
        let value = serde_json::to_value(self).context("could not encode gateway settings")?;
        extra.insert(SETTINGS_KEY.to_owned(), value);
        Ok(())
    }

    /// Remove routing configuration, e.g. on sign-out.
    pub fn remove_from(extra: &mut BTreeMap<String, Value>) {
        extra.remove(SETTINGS_KEY);
    }

    /// True when routing is on and there is at least one key to route with.
    pub fn is_usable(&self) -> bool {
        self.enabled
            && !self.endpoint.is_empty()
            && [&self.api_key, &self.claude_api_key, &self.codex_api_key]
                .into_iter()
                .flatten()
                .any(|key| !key.is_empty())
    }

    /// Key for a provider, falling back to the general gateway key.
    fn key_for(&self, provider_id: &str) -> Option<&str> {
        let specific = match provider_id {
            "claude" => self.claude_api_key.as_deref(),
            "codex" => self.codex_api_key.as_deref(),
            _ => None,
        };
        specific
            .or(self.api_key.as_deref())
            .filter(|key| !key.is_empty())
    }

    /// Environment overrides for a provider, as `(name, value)` pairs.
    ///
    /// An empty result means "leave this provider alone" — the agent then runs
    /// on the user's own credentials exactly as it would upstream.
    pub fn env_for(&self, provider_id: &str) -> Vec<(String, String)> {
        if !self.is_usable() {
            return Vec::new();
        }
        let Some(key) = self.key_for(provider_id) else {
            return Vec::new();
        };
        match provider_id {
            "claude" => vec![
                ("ANTHROPIC_BASE_URL".to_owned(), anthropic_base_url(&self.endpoint)),
                ("ANTHROPIC_AUTH_TOKEN".to_owned(), key.to_owned()),
            ],
            // Codex reads its base URL from config.toml, not the environment,
            // so routing happens through a generated CODEX_HOME instead. The
            // key is still exported: Codex reads it for the OpenAI provider.
            "codex" => vec![("OPENAI_API_KEY".to_owned(), key.to_owned())],
            _ => Vec::new(),
        }
    }

    /// Create (or refresh) an isolated `CODEX_HOME` and return its path.
    ///
    /// Callers export `CODEX_HOME=<returned path>` alongside [`Self::env_for`].
    /// The directory lives beside our other state, so the user's real
    /// `~/.codex` is never read or written.
    pub fn prepare_codex_home(&self) -> Result<PathBuf> {
        let key = self
            .key_for("codex")
            .ok_or_else(|| anyhow!("no gateway key is available for Codex"))?;
        let home = brand::data_dir()
            .ok_or_else(|| anyhow!("could not locate the home directory"))?
            .join("gateway")
            .join("codex");
        std::fs::create_dir_all(&home)
            .with_context(|| format!("could not create {}", home.display()))?;
        write_private(
            &home.join("config.toml"),
            &codex_config_toml(&self.endpoint, self.codex_model.as_deref()),
        )?;
        write_private(
            &home.join("auth.json"),
            &serde_json::to_string_pretty(&serde_json::json!({ "OPENAI_API_KEY": key }))?,
        )?;
        Ok(home)
    }
}

/// Path of the daemon settings document the desktop publishes into.
///
/// Matches upstream's `DaemonSettings::default_path()`. Recomputed here rather
/// than imported so this crate stays free of the upstream crates and can be
/// tested without them.
pub fn settings_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("settings.json"))
}

/// Read the routing configuration out of the daemon settings document.
///
/// Any failure — missing file, unreadable, malformed, absent key — means "not
/// configured" rather than an error: routing is an enhancement, and a broken
/// settings file must never stop an agent from starting on the user's own
/// credentials.
pub fn load() -> Option<GatewayConfig> {
    load_from(&settings_path()?)
}

/// [`load`] against an explicit path.
pub fn load_from(path: &Path) -> Option<GatewayConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    let document: BTreeMap<String, Value> = serde_json::from_str(&raw).ok()?;
    GatewayConfig::from_extra(&document)
}

/// Apply gateway routing to a command about to spawn `provider_id`.
///
/// Returns whether anything was injected. When routing is off, unconfigured, or
/// unknown for this provider, the command is left exactly as upstream built it.
pub fn apply_to_command(command: &mut std::process::Command, provider_id: &str) -> bool {
    let Some(config) = load() else {
        return false;
    };
    apply_config_to_command(&config, command, provider_id)
}

/// [`apply_to_command`] against an explicit configuration.
pub fn apply_config_to_command(
    config: &GatewayConfig,
    command: &mut std::process::Command,
    provider_id: &str,
) -> bool {
    let overrides = config.env_for(provider_id);
    if overrides.is_empty() {
        return false;
    }
    for (name, value) in overrides {
        command.env(name, value);
    }
    // Codex has no base-URL environment variable, so it is pointed at a
    // generated config directory instead. A failure here leaves the key
    // exported but the routing inactive, which the settings page surfaces;
    // it must not abort the spawn.
    if provider_id == "codex"
        && let Ok(home) = config.prepare_codex_home()
    {
        command.env("CODEX_HOME", home);
    }
    true
}

/// Anthropic's base URL is the gateway root: the SDK appends `/v1` itself.
pub fn anthropic_base_url(endpoint: &str) -> String {
    normalize_endpoint(endpoint)
}

/// OpenAI-compatible clients expect the versioned path.
pub fn openai_base_url(endpoint: &str) -> String {
    format!("{}/v1", normalize_endpoint(endpoint))
}

/// Strip trailing slashes and a trailing `/v1`, which users often paste in.
fn normalize_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    trimmed
        .strip_suffix("/v1")
        .or_else(|| trimmed.strip_suffix("/V1"))
        .unwrap_or(trimmed)
        .to_owned()
}

/// Codex configuration pointing the built-in OpenAI provider at the gateway.
///
/// The defaults mirror the Electron client's generated config. Two of them are
/// first-run gates rather than preferences: `windows_wsl_setup_acknowledged`
/// suppresses the WSL setup prompt a fresh `CODEX_HOME` would otherwise raise
/// on Windows (which would hang a headless app-server session), and
/// `network_access` keeps the sandbox from blocking the gateway itself.
fn codex_config_toml(endpoint: &str, model: Option<&str>) -> String {
    let model = model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(DEFAULT_CODEX_MODEL);
    let base_url = openai_base_url(endpoint);
    format!(
        r#"# Generated by the app. Edits are overwritten on the next sign-in.
model_provider = "OpenAI"
model = "{model}"
review_model = "{model}"
disable_response_storage = true
network_access = "enabled"
windows_wsl_setup_acknowledged = true

[model_providers.OpenAI]
name = "OpenAI"
base_url = "{base_url}"
wire_api = "responses"
requires_openai_auth = true
"#
    )
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)
        .with_context(|| format!("could not write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GatewayConfig {
        GatewayConfig {
            enabled: true,
            endpoint: "https://cloud.example.org".to_owned(),
            api_key: Some("sk-general".to_owned()),
            claude_api_key: Some("sk-claude".to_owned()),
            codex_api_key: None,
            codex_model: None,
        }
    }

    #[test]
    fn normalizes_endpoints_users_paste() {
        assert_eq!(anthropic_base_url("https://a.org/"), "https://a.org");
        assert_eq!(anthropic_base_url("https://a.org/v1"), "https://a.org");
        assert_eq!(anthropic_base_url("https://a.org/v1/"), "https://a.org");
        assert_eq!(openai_base_url("https://a.org"), "https://a.org/v1");
        // Already-versioned input must not become /v1/v1.
        assert_eq!(openai_base_url("https://a.org/v1"), "https://a.org/v1");
    }

    #[test]
    fn claude_gets_base_url_and_token() {
        let env = config().env_for("claude");
        assert_eq!(
            env,
            vec![
                ("ANTHROPIC_BASE_URL".to_owned(), "https://cloud.example.org".to_owned()),
                ("ANTHROPIC_AUTH_TOKEN".to_owned(), "sk-claude".to_owned()),
            ]
        );
    }

    #[test]
    fn provider_key_falls_back_to_the_general_key() {
        // Codex has no dedicated key here, so the general one is used.
        let env = config().env_for("codex");
        assert_eq!(env, vec![("OPENAI_API_KEY".to_owned(), "sk-general".to_owned())]);
    }

    #[test]
    fn unknown_providers_are_left_alone() {
        assert!(config().env_for("opencode").is_empty());
        assert!(config().env_for("").is_empty());
    }

    #[test]
    fn disabled_config_injects_nothing() {
        let disabled = GatewayConfig {
            enabled: false,
            ..config()
        };
        assert!(disabled.env_for("claude").is_empty());
        assert!(!disabled.is_usable());
    }

    #[test]
    fn config_without_keys_is_not_usable() {
        let keyless = GatewayConfig {
            api_key: None,
            claude_api_key: None,
            codex_api_key: None,
            ..config()
        };
        assert!(!keyless.is_usable());
        assert!(keyless.env_for("claude").is_empty());

        let empty_key = GatewayConfig {
            api_key: Some(String::new()),
            claude_api_key: None,
            codex_api_key: None,
            ..config()
        };
        assert!(!empty_key.is_usable());
    }

    #[test]
    fn round_trips_through_daemon_settings_extra() {
        let mut extra = BTreeMap::new();
        config().write_into(&mut extra).expect("write");
        assert!(extra.contains_key(SETTINGS_KEY));
        assert_eq!(GatewayConfig::from_extra(&extra), Some(config()));

        GatewayConfig::remove_from(&mut extra);
        assert!(extra.is_empty());
        assert_eq!(GatewayConfig::from_extra(&extra), None);
    }

    #[test]
    fn foreign_extra_keys_are_ignored() {
        // An upstream daemon's own extras must not be mistaken for ours.
        let mut extra = BTreeMap::new();
        extra.insert("someUpstreamFlag".to_owned(), Value::Bool(true));
        assert_eq!(GatewayConfig::from_extra(&extra), None);
    }

    #[test]
    fn malformed_stored_config_is_ignored_rather_than_fatal() {
        let mut extra = BTreeMap::new();
        extra.insert(SETTINGS_KEY.to_owned(), Value::String("garbage".to_owned()));
        assert_eq!(GatewayConfig::from_extra(&extra), None);
    }

    #[test]
    fn codex_config_points_the_openai_provider_at_the_gateway() {
        let toml = codex_config_toml("https://cloud.example.org/v1/", Some("gpt-test"));
        assert!(toml.contains(r#"model_provider = "OpenAI""#));
        assert!(toml.contains(r#"model = "gpt-test""#));
        assert!(toml.contains(r#"base_url = "https://cloud.example.org/v1""#));
        assert!(toml.contains(r#"wire_api = "responses""#));
        // First-run gates: without these a fresh CODEX_HOME prompts for WSL
        // setup on Windows and sandboxes away the network.
        assert!(toml.contains("windows_wsl_setup_acknowledged = true"));
        assert!(toml.contains(r#"network_access = "enabled""#));
    }

    #[test]
    fn loads_the_config_out_of_a_settings_document() {
        let dir = std::env::temp_dir().join(format!("sub2api-gw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.json");

        // A realistic document: upstream keys alongside ours.
        let mut extra = BTreeMap::new();
        config().write_into(&mut extra).expect("write");
        let mut document = serde_json::Map::new();
        document.insert("computer_use_enabled".to_owned(), Value::Bool(false));
        for (key, value) in extra {
            document.insert(key, value);
        }
        std::fs::write(&path, serde_json::to_string(&document).expect("encode")).expect("write");

        assert_eq!(load_from(&path), Some(config()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unusable_settings_document_means_not_configured() {
        let dir = std::env::temp_dir().join(format!("sub2api-gw-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let missing = dir.join("absent.json");
        assert_eq!(load_from(&missing), None);

        let malformed = dir.join("malformed.json");
        std::fs::write(&malformed, "{not json").expect("write");
        assert_eq!(load_from(&malformed), None);

        let unrelated = dir.join("unrelated.json");
        std::fs::write(&unrelated, r#"{"computer_use_enabled":true}"#).expect("write");
        assert_eq!(load_from(&unrelated), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_injects_for_claude_and_leaves_other_providers_untouched() {
        let mut command = std::process::Command::new("echo");
        assert!(apply_config_to_command(&config(), &mut command, "claude"));

        let injected: Vec<_> = command
            .get_envs()
            .filter_map(|(name, value)| Some((name.to_str()?, value?.to_str()?)))
            .collect();
        assert!(injected.contains(&("ANTHROPIC_BASE_URL", "https://cloud.example.org")));
        assert!(injected.contains(&("ANTHROPIC_AUTH_TOKEN", "sk-claude")));

        let mut untouched = std::process::Command::new("echo");
        assert!(!apply_config_to_command(&config(), &mut untouched, "opencode"));
        assert_eq!(untouched.get_envs().count(), 0);
    }

    #[test]
    fn apply_is_a_no_op_when_routing_is_disabled() {
        let disabled = GatewayConfig {
            enabled: false,
            ..config()
        };
        let mut command = std::process::Command::new("echo");
        assert!(!apply_config_to_command(&disabled, &mut command, "claude"));
        assert_eq!(command.get_envs().count(), 0);
    }

    #[test]
    fn codex_config_falls_back_to_the_default_model() {
        for model in [None, Some(""), Some("   ")] {
            let toml = codex_config_toml("https://a.org", model);
            assert!(
                toml.contains(&format!(r#"model = "{DEFAULT_CODEX_MODEL}""#)),
                "model {model:?} should fall back"
            );
        }
    }
}
