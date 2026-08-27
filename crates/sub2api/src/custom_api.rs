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

#[cfg(test)]
mod tests {
    use super::*;

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
