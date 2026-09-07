//! Choosing which of the managed service's domains the CLIs talk to.
//!
//! The service answers on more than one origin, and which one is fastest —
//! or reachable at all — is a property of the user's network, not of the
//! account. This applies the same candidate-and-measure idea as a custom
//! endpoint's alternate domains, one level up: the origin written into every
//! CLI's configuration.
//!
//! **Signing in stays on one origin.** Tokens are minted, refreshed and
//! checked against the domain the browser flow used
//! ([`crate::auth::credentials_from_fragment`] rejects a callback from
//! anywhere else, and the refresh path reconciles a stored session by
//! endpoint), so `Credentials.endpoint` is never rewritten here. Only
//! [`crate::GatewayConfig::endpoint`] — the URL the agent CLIs are pointed
//! at — moves.
//!
//! A domain counts as usable only when it answers the models listing with
//! the account's own gateway key. That is the same request a routed CLI
//! makes first, so a green result means the CLI will work, not merely that
//! the host is up.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::auth::Credentials;
use crate::brand;
use crate::custom_api::normalize_base_url;
use crate::global_config::atomic_write_private;
use crate::speedtest::{self, CandidateResult};

/// Which service origin the CLIs are routed through.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GatewayOriginConfig {
    /// Origins the user added, on top of the ones the build ships with.
    #[serde(default)]
    pub candidates: Vec<String>,
    /// The origin in use. `None` means the build's primary.
    #[serde(default)]
    pub chosen: Option<String>,
    /// Switch to the fastest answering origin after a measurement.
    #[serde(default)]
    pub auto_select: bool,
}

impl GatewayOriginConfig {
    /// Every origin that may be measured: the build's, then the user's.
    /// Normalized and de-duplicated, so the same host typed two ways is one
    /// entry.
    pub fn effective_candidates(&self) -> Vec<String> {
        let mut origins: Vec<String> = Vec::new();
        let mut push = |raw: &str| {
            if let Ok(url) = normalize_base_url(raw)
                && !origins.iter().any(|known| *known == url)
            {
                origins.push(url);
            }
        };
        for endpoint in brand::managed_service_endpoints() {
            push(&endpoint);
        }
        for candidate in &self.candidates {
            push(candidate);
        }
        origins
    }

    /// Origins the user added and may therefore remove — the build's own are
    /// permanent.
    pub fn is_removable(&self, origin: &str) -> bool {
        !brand::managed_service_endpoints()
            .iter()
            .any(|known| normalize_base_url(known).is_ok_and(|known| known == origin))
    }

    /// The origin to route through: the chosen one while it is still a
    /// candidate, otherwise the build's primary.
    ///
    /// Falling back rather than trusting the stored value matters after an
    /// update drops a retired domain — the alternative is every CLI pointed
    /// at a host that no longer exists.
    pub fn origin(&self) -> Option<String> {
        let candidates = self.effective_candidates();
        self.chosen
            .as_ref()
            .filter(|chosen| candidates.iter().any(|known| known == *chosen))
            .cloned()
            .or_else(|| candidates.first().cloned())
    }

    /// Route through `origin`, remembering it as a candidate.
    pub fn select(&mut self, origin: &str) -> Result<()> {
        let origin = normalize_base_url(origin)
            .map_err(|error| anyhow!("{origin} is not a usable URL: {error:?}"))?;
        self.add(&origin)?;
        self.chosen = Some(origin);
        Ok(())
    }

    /// Add an origin. Returns false when it was already known.
    pub fn add(&mut self, origin: &str) -> Result<bool> {
        let origin = normalize_base_url(origin)
            .map_err(|error| anyhow!("{origin} is not a usable URL: {error:?}"))?;
        if self.effective_candidates().iter().any(|known| *known == origin) {
            return Ok(false);
        }
        self.candidates.push(origin);
        Ok(true)
    }

    /// Drop a user-added origin. Removing the one in use falls back to the
    /// build's primary.
    pub fn remove(&mut self, origin: &str) {
        self.candidates.retain(|known| known != origin);
        if self.chosen.as_deref() == Some(origin) {
            self.chosen = None;
        }
    }
}

/// Where the choice lives.
pub fn config_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("gateway-origin.json"))
}

/// Load the choice; absent or unreadable means "the build's primary".
pub fn load() -> GatewayOriginConfig {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(config: &GatewayOriginConfig) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut encoded =
        serde_json::to_string_pretty(config).context("could not encode the gateway origin")?;
    encoded.push('\n');
    atomic_write_private(&path, encoded.as_bytes())
}

/// The protocol and key to probe an origin with: whichever gateway key the
/// session actually holds. Probing the data plane rather than the account
/// API is the point — it proves the domain serves this account's traffic.
pub fn probe_target(credentials: &Credentials) -> Option<(&'static str, String)> {
    let usable = |key: &Option<String>| {
        key.as_ref()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
    };
    if let Some(key) = usable(&credentials.claude_api_key) {
        return Some(("claude", key));
    }
    // Codex and the general key both speak the OpenAI shape.
    usable(&credentials.codex_api_key)
        .or_else(|| usable(&credentials.api_key))
        .map(|key| ("codex", key))
}

/// Measure every candidate origin. Blocking; callers run it off the UI
/// thread. Without a gateway key there is nothing meaningful to ask, so the
/// result is empty rather than a set of unauthenticated successes.
pub fn test_origins(
    config: &GatewayOriginConfig,
    credentials: &Credentials,
    timeout_secs: u32,
) -> Vec<CandidateResult> {
    let Some((provider_id, key)) = probe_target(credentials) else {
        return Vec::new();
    };
    speedtest::test_candidates(
        provider_id,
        &config.effective_candidates(),
        &key,
        timeout_secs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials(claude: Option<&str>, codex: Option<&str>, general: Option<&str>) -> Credentials {
        Credentials {
            claude_api_key: claude.map(str::to_owned),
            codex_api_key: codex.map(str::to_owned),
            api_key: general.map(str::to_owned),
            ..Credentials::default()
        }
    }

    #[test]
    fn built_in_origins_come_first_and_are_deduped() {
        let mut config = GatewayOriginConfig::default();
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        assert_eq!(config.effective_candidates(), vec![primary.clone()]);

        // The same host typed differently is not a second entry.
        assert!(!config.add(&format!("{primary}/")).expect("add"));
        assert!(config.add("mirror.example.org").expect("add"));
        assert!(!config.add("https://mirror.example.org/v1").expect("add"));
        assert_eq!(
            config.effective_candidates(),
            vec![primary.clone(), "https://mirror.example.org".to_owned()]
        );
        assert!(config.add("not a url").is_err());
    }

    #[test]
    fn origin_falls_back_when_the_choice_is_gone() {
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        let mut config = GatewayOriginConfig::default();
        assert_eq!(config.origin().as_deref(), Some(primary.as_str()));

        config.select("mirror.example.org").expect("select");
        assert_eq!(config.origin().as_deref(), Some("https://mirror.example.org"));
        assert!(config.is_removable("https://mirror.example.org"));
        assert!(!config.is_removable(&primary));

        // A stored choice that is no longer a candidate is not routed to.
        let orphaned = GatewayOriginConfig {
            candidates: Vec::new(),
            chosen: Some("https://retired.example.org".to_owned()),
            auto_select: false,
        };
        assert_eq!(orphaned.origin().as_deref(), Some(primary.as_str()));

        config.remove("https://mirror.example.org");
        assert_eq!(config.origin().as_deref(), Some(primary.as_str()));
        assert_eq!(config.chosen, None);
    }

    #[test]
    fn probe_target_prefers_the_keys_the_session_holds() {
        assert_eq!(
            probe_target(&credentials(Some("sk-ant"), Some("sk-oai"), None)),
            Some(("claude", "sk-ant".to_owned()))
        );
        assert_eq!(
            probe_target(&credentials(None, Some("sk-oai"), Some("sk-gen"))),
            Some(("codex", "sk-oai".to_owned()))
        );
        assert_eq!(
            probe_target(&credentials(None, None, Some("sk-gen"))),
            Some(("codex", "sk-gen".to_owned()))
        );
        // Blank keys are not keys.
        assert_eq!(probe_target(&credentials(Some("  "), None, None)), None);
        assert_eq!(probe_target(&Credentials::default()), None);
        // And without one there is nothing to measure.
        assert!(
            test_origins(&GatewayOriginConfig::default(), &Credentials::default(), 5).is_empty()
        );
    }

    #[test]
    fn config_round_trips_and_missing_file_is_default() {
        let config = GatewayOriginConfig {
            candidates: vec!["https://mirror.example.org".to_owned()],
            chosen: Some("https://mirror.example.org".to_owned()),
            auto_select: true,
        };
        let encoded = serde_json::to_string(&config).expect("encode");
        let decoded: GatewayOriginConfig = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, config);
        let empty: GatewayOriginConfig = serde_json::from_str("{}").expect("decode empty");
        assert_eq!(empty, GatewayOriginConfig::default());
    }
}
