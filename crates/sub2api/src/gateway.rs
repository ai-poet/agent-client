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
    /// The key for each of the built-in agent's models whose group is known
    /// (`Credentials::model_routes`). Only the built-in agent reads it: a CLI
    /// takes one key for its whole run.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub model_keys: std::collections::BTreeMap<String, String>,
    /// Each catalog model's context window (`Credentials::model_windows`).
    /// Only the built-in agent reads it; the CLIs know their own models.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub model_windows: std::collections::BTreeMap<String, u64>,
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

/// OpenAI-compatible clients expect the versioned path: `/v1`, unless the
/// endpoint already ends in a version of its own (`…/api/paas/v4`).
pub fn openai_base_url(endpoint: &str) -> String {
    let base = normalize_endpoint(endpoint);
    if ends_with_version_segment(&base) {
        base
    } else {
        format!("{base}/v1")
    }
}

/// `<base>/<path>` when `base` already ends in a version segment, else
/// `<base>/v1/<path>`.
///
/// Kept in step with `claurst_api::endpoint::versioned_url`, which is what
/// the built-in agent's adapters build their requests with; a probe from
/// here has to reach the URL the agent will.
pub fn versioned_url(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if ends_with_version_segment(base) {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

/// Whether the last path segment of `base` names an API version (`v1`,
/// `v4`, `v1beta`, `v2alpha1`). A bare origin answers false.
pub fn ends_with_version_segment(base: &str) -> bool {
    let base = base.trim().trim_end_matches('/');
    let after_scheme = base.split_once("://").map_or(base, |(_, rest)| rest);
    let Some((_, path)) = after_scheme.split_once('/') else {
        return false;
    };
    let segment = path.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    let Some(rest) = segment.strip_prefix('v') else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    let rest = &rest[digits..];
    if rest.is_empty() {
        return true;
    }
    rest.strip_prefix("alpha")
        .or_else(|| rest.strip_prefix("beta"))
        .is_some_and(|tail| tail.bytes().all(|byte| byte.is_ascii_digit()))
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
            model_keys: Default::default(),
            model_windows: Default::default(),
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
        // A provider that versions its API elsewhere keeps its own version.
        assert_eq!(
            openai_base_url("https://open.bigmodel.cn/api/paas/v4/"),
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert_eq!(
            openai_base_url("https://ark.cn-beijing.volces.com/api/v3"),
            "https://ark.cn-beijing.volces.com/api/v3"
        );
        assert_eq!(openai_base_url("https://a.org/video"), "https://a.org/video/v1");
    }

    #[test]
    fn versioned_urls_match_the_agents_adapters() {
        assert_eq!(
            versioned_url("https://a.org", "chat/completions"),
            "https://a.org/v1/chat/completions"
        );
        assert_eq!(versioned_url("https://a.org/v1/", "/models"), "https://a.org/v1/models");
        assert_eq!(
            versioned_url("https://open.bigmodel.cn/api/paas/v4", "chat/completions"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert!(ends_with_version_segment("https://a.org/v1beta"));
        for base in ["https://a.org", "https://a.org/v2proxy", "https://v1.a.org", "a.org:8080"] {
            assert!(!ends_with_version_segment(base), "{base}");
        }
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
