//! The managed gateway's routing values.
//!
//! This used to carry an entire transport (daemon settings, spawn-time
//! environment injection, generated config homes). Routing now writes each
//! CLI's own global configuration instead — see [`crate::global_config`] —
//! so what remains here is the desktop-side value object built from the
//! signed-in credentials, plus the endpoint normalization every writer
//! shares.

use serde::{Deserialize, Serialize};

/// What the signed-in account can route with.
///
/// Deliberately holds gateway API keys only — never the OAuth access or
/// refresh token, which stay in the credential file.
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
    /// Codex model to pin, when the account specifies one.
    #[serde(default)]
    pub codex_model: Option<String>,
}

impl GatewayConfig {
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
    pub fn key_for(&self, provider_id: &str) -> Option<&str> {
        let specific = match provider_id {
            "claude" => self.claude_api_key.as_deref(),
            "codex" => self.codex_api_key.as_deref(),
            _ => None,
        };
        specific
            .or(self.api_key.as_deref())
            .filter(|key| !key.is_empty())
    }
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
    fn provider_keys_fall_back_to_the_general_key() {
        let config = config();
        assert_eq!(config.key_for("claude"), Some("sk-claude"));
        // Codex has no dedicated key here, so the general one is used.
        assert_eq!(config.key_for("codex"), Some("sk-general"));
        // Unknown providers get the general key too (the caller decides
        // whether that provider is gateway-routable at all).
        assert_eq!(config.key_for("grok"), Some("sk-general"));
    }

    #[test]
    fn disabled_or_keyless_configs_are_not_usable() {
        assert!(config().is_usable());
        assert!(
            !GatewayConfig {
                enabled: false,
                ..config()
            }
            .is_usable()
        );
        assert!(
            !GatewayConfig {
                api_key: None,
                claude_api_key: None,
                codex_api_key: None,
                ..config()
            }
            .is_usable()
        );
        let empty_key = GatewayConfig {
            api_key: Some(String::new()),
            claude_api_key: None,
            codex_api_key: None,
            ..config()
        };
        assert!(!empty_key.is_usable());
        assert_eq!(empty_key.key_for("claude"), None);
    }
}
