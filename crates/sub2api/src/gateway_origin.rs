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

/// How long the server's node list is trusted before it is asked again.
pub const SERVER_NODES_TTL_SECS: i64 = 6 * 60 * 60;

/// At most this many server nodes are kept; the service caps its list at ten.
const MAX_SERVER_NODES: usize = 10;

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
    /// Nodes the service lists publicly (the admin's "custom endpoints"),
    /// cached so they are still offered when the primary cannot be reached —
    /// which is exactly when another node matters.
    #[serde(default)]
    pub server_nodes: Vec<ServerNode>,
    /// Unix seconds of the last successful read of [`Self::server_nodes`].
    #[serde(default)]
    pub server_nodes_fetched_at: Option<i64>,
    /// The origin an automatic fallback moved away from. Once it answers
    /// again the route goes back to it, so a single bad launch does not
    /// leave the user on a node they never chose.
    #[serde(default)]
    pub fallback_from: Option<String>,
    /// Timeouts for model requests through the managed gateway.
    #[serde(default)]
    pub network: NetworkTimeouts,
}

/// One node from the service's public list.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ServerNode {
    /// Display name as the admin wrote it (translated for display only).
    pub name: String,
    /// Normalized origin, no path.
    pub origin: String,
    #[serde(default)]
    pub description: String,
}

/// How long a model request may take to connect, and how long a response
/// may stay silent, before it is abandoned and retried. Read by the built-in
/// agent; the other CLIs keep their own.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct NetworkTimeouts {
    #[serde(default = "default_connect_secs")]
    pub connect_secs: u32,
    #[serde(default = "default_idle_secs")]
    pub idle_secs: u32,
}

fn default_connect_secs() -> u32 {
    15
}

fn default_idle_secs() -> u32 {
    // Generous: a model reasoning behind an endpoint that sends no
    // keep-alives can stay silent for minutes; a dead peer is caught sooner
    // by TCP keep-alive.
    300
}

impl Default for NetworkTimeouts {
    fn default() -> Self {
        Self {
            connect_secs: default_connect_secs(),
            idle_secs: default_idle_secs(),
        }
    }
}

/// What a node is called in the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeLabel {
    /// The build's primary origin, the default route.
    Primary,
    /// A node the service lists, with the admin's wording.
    Server { name: String, description: String },
    /// The CDN node the build ships with.
    BuiltInCdn,
    /// Anything else: the user's own or an unlabelled build alternate.
    Other,
}

impl GatewayOriginConfig {
    /// Every origin that may be measured: the build's, the service's, then
    /// the user's. Normalized and de-duplicated, so the same host typed two
    /// ways is one entry.
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
        for node in &self.server_nodes {
            push(&node.origin);
        }
        for candidate in &self.candidates {
            push(candidate);
        }
        origins
    }

    /// Origins the user added and may therefore remove — the build's own and
    /// the service's are permanent.
    pub fn is_removable(&self, origin: &str) -> bool {
        let built_in = brand::managed_service_endpoints()
            .iter()
            .any(|known| normalize_base_url(known).is_ok_and(|known| known == origin));
        let listed = self.server_nodes.iter().any(|node| node.origin == origin);
        !built_in && !listed
    }

    /// How the picker names `origin`. The primary keeps its own label (it is
    /// the default); otherwise the admin's wording wins over the build's,
    /// since it can be corrected without a release.
    pub fn node_label(&self, origin: &str) -> NodeLabel {
        let is = |raw: &str| normalize_base_url(raw).is_ok_and(|known| known == origin);
        if is(brand::MANAGED_SERVICE_URL) {
            return NodeLabel::Primary;
        }
        if let Some(node) = self.server_nodes.iter().find(|node| node.origin == origin) {
            return NodeLabel::Server {
                name: node.name.clone(),
                description: node.description.clone(),
            };
        }
        if !brand::MANAGED_SERVICE_CDN_URL.trim().is_empty() && is(brand::MANAGED_SERVICE_CDN_URL) {
            return NodeLabel::BuiltInCdn;
        }
        NodeLabel::Other
    }

    /// Whether the cached server list is old enough to ask again.
    pub fn server_nodes_due(&self, now: i64) -> bool {
        self.server_nodes_fetched_at
            .is_none_or(|at| now.saturating_sub(at) >= SERVER_NODES_TTL_SECS)
    }

    /// Replace the cached server list with what the service lists now.
    ///
    /// Only bare HTTPS origins become nodes: the same list feeds the web
    /// panel's API Keys page, where an admin may also list addresses with a
    /// path (another protocol's prefix), which are not the gateway's root.
    /// Returns whether the list changed.
    pub fn adopt_server_nodes(
        &mut self,
        listed: &[crate::client::PublicEndpoint],
        now: i64,
    ) -> bool {
        let mut nodes: Vec<ServerNode> = Vec::new();
        for entry in listed {
            let Some(origin) = server_node_origin(&entry.endpoint) else {
                continue;
            };
            if nodes.iter().any(|known| known.origin == origin) {
                continue;
            }
            nodes.push(ServerNode {
                name: entry.name.trim().to_owned(),
                origin,
                description: entry.description.trim().to_owned(),
            });
            if nodes.len() == MAX_SERVER_NODES {
                break;
            }
        }
        self.server_nodes_fetched_at = Some(now);
        if nodes == self.server_nodes {
            return false;
        }
        self.server_nodes = nodes;
        true
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

    /// Route through `origin`, remembering it as a candidate. A deliberate
    /// choice, so any pending return from a fallback is forgotten.
    pub fn select(&mut self, origin: &str) -> Result<()> {
        let origin = normalize_base_url(origin)
            .map_err(|error| anyhow!("{origin} is not a usable URL: {error:?}"))?;
        self.add(&origin)?;
        self.chosen = Some(origin);
        self.fallback_from = None;
        Ok(())
    }

    /// Route through `origin` because the current one stopped answering,
    /// remembering where the route was so it can go back
    /// ([`Self::restore_target`]). Repeated fallbacks keep the first origin.
    pub fn fall_back(&mut self, origin: &str) -> Result<()> {
        let from = self.fallback_from.clone().or_else(|| self.origin());
        self.select(origin)?;
        if from.as_deref() != self.chosen.as_deref() {
            self.fallback_from = from;
        }
        Ok(())
    }

    /// The origin a fallback moved away from, while it is still a candidate
    /// and not already the route.
    pub fn restore_target(&self) -> Option<String> {
        let from = self.fallback_from.as_ref()?;
        let current = self.origin();
        (current.as_deref() != Some(from.as_str())
            && self
                .effective_candidates()
                .iter()
                .any(|known| known == from))
        .then(|| from.clone())
    }

    /// Add an origin. Returns false when it was already known.
    pub fn add(&mut self, origin: &str) -> Result<bool> {
        let origin = normalize_base_url(origin)
            .map_err(|error| anyhow!("{origin} is not a usable URL: {error:?}"))?;
        if self
            .effective_candidates()
            .iter()
            .any(|known| *known == origin)
        {
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
        if self.fallback_from.as_deref() == Some(origin) {
            self.fallback_from = None;
        }
    }
}

/// The node origin an entry of the service's list stands for: a bare HTTPS
/// origin (a trailing `/v1` is the same root), or nothing.
fn server_node_origin(raw: &str) -> Option<String> {
    let origin = normalize_base_url(raw).ok()?;
    let host = origin.strip_prefix("https://")?;
    (!host.is_empty() && !host.contains('/')).then_some(origin)
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

    fn credentials(
        claude: Option<&str>,
        codex: Option<&str>,
        general: Option<&str>,
    ) -> Credentials {
        Credentials {
            claude_api_key: claude.map(str::to_owned),
            codex_api_key: codex.map(str::to_owned),
            api_key: general.map(str::to_owned),
            ..Credentials::default()
        }
    }

    fn built_in() -> Vec<String> {
        let mut origins: Vec<String> = Vec::new();
        for endpoint in brand::managed_service_endpoints() {
            let url = normalize_base_url(&endpoint).expect("built-in origin");
            if !origins.contains(&url) {
                origins.push(url);
            }
        }
        origins
    }

    fn listed(entries: &[(&str, &str, &str)]) -> Vec<crate::client::PublicEndpoint> {
        entries
            .iter()
            .map(
                |(name, endpoint, description)| crate::client::PublicEndpoint {
                    name: (*name).to_owned(),
                    endpoint: (*endpoint).to_owned(),
                    description: (*description).to_owned(),
                },
            )
            .collect()
    }

    #[test]
    fn built_in_origins_come_first_and_are_deduped() {
        let mut config = GatewayOriginConfig::default();
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        let built_in = built_in();
        assert_eq!(config.effective_candidates(), built_in);
        assert_eq!(built_in.first(), Some(&primary));

        // The same host typed differently is not a second entry.
        assert!(!config.add(&format!("{primary}/")).expect("add"));
        assert!(config.add("mirror.example.org").expect("add"));
        assert!(!config.add("https://mirror.example.org/v1").expect("add"));
        let mut expected = built_in.clone();
        expected.push("https://mirror.example.org".to_owned());
        assert_eq!(config.effective_candidates(), expected);
        assert!(config.add("not a url").is_err());
    }

    #[test]
    fn origin_falls_back_when_the_choice_is_gone() {
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        let mut config = GatewayOriginConfig::default();
        assert_eq!(config.origin().as_deref(), Some(primary.as_str()));

        config.select("mirror.example.org").expect("select");
        assert_eq!(
            config.origin().as_deref(),
            Some("https://mirror.example.org")
        );
        assert!(config.is_removable("https://mirror.example.org"));
        assert!(!config.is_removable(&primary));

        // A stored choice that is no longer a candidate is not routed to.
        let orphaned = GatewayOriginConfig {
            chosen: Some("https://retired.example.org".to_owned()),
            ..GatewayOriginConfig::default()
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
            server_nodes: vec![ServerNode {
                name: "HK".to_owned(),
                origin: "https://hk.example.org".to_owned(),
                description: "Hong Kong".to_owned(),
            }],
            server_nodes_fetched_at: Some(1_700_000_000),
            fallback_from: Some("https://hk.example.org".to_owned()),
            network: NetworkTimeouts {
                connect_secs: 5,
                idle_secs: 90,
            },
        };
        let encoded = serde_json::to_string(&config).expect("encode");
        let decoded: GatewayOriginConfig = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, config);
        let empty: GatewayOriginConfig = serde_json::from_str("{}").expect("decode empty");
        assert_eq!(empty, GatewayOriginConfig::default());
        assert_eq!(
            empty.network,
            NetworkTimeouts {
                connect_secs: 15,
                idle_secs: 300
            }
        );
        // A file written before the node fields existed still loads.
        let legacy: GatewayOriginConfig = serde_json::from_str(
            r#"{"candidates":[],"chosen":"https://mirror.example.org","auto_select":false}"#,
        )
        .expect("decode legacy");
        assert_eq!(legacy.chosen.as_deref(), Some("https://mirror.example.org"));
        assert!(legacy.server_nodes.is_empty());
        let partial: NetworkTimeouts =
            serde_json::from_str(r#"{"idle_secs":60}"#).expect("decode partial");
        assert_eq!(
            partial,
            NetworkTimeouts {
                connect_secs: 15,
                idle_secs: 60
            }
        );
    }

    #[test]
    fn server_nodes_keep_only_bare_https_origins() {
        let mut config = GatewayOriginConfig::default();
        let changed = config.adopt_server_nodes(
            &listed(&[
                ("HK", "https://hk.example.org/", "Hong Kong"),
                ("HK again", "https://hk.example.org/v1", ""),
                ("Antigravity", "https://hk.example.org/antigravity", ""),
                ("Plain", "http://plain.example.org", ""),
                ("Broken", "not a url", ""),
                ("Tokyo", "tokyo.example.org", " Japan "),
            ]),
            100,
        );
        assert!(changed);
        assert_eq!(
            config.server_nodes,
            vec![
                ServerNode {
                    name: "HK".to_owned(),
                    origin: "https://hk.example.org".to_owned(),
                    description: "Hong Kong".to_owned(),
                },
                ServerNode {
                    name: "Tokyo".to_owned(),
                    origin: "https://tokyo.example.org".to_owned(),
                    description: "Japan".to_owned(),
                },
            ]
        );
        assert_eq!(config.server_nodes_fetched_at, Some(100));
        // Same list again: nothing to save beyond the timestamp.
        assert!(!config.adopt_server_nodes(
            &listed(&[
                ("HK", "https://hk.example.org", "Hong Kong"),
                ("Tokyo", "https://tokyo.example.org", "Japan"),
            ]),
            200,
        ));
        assert_eq!(config.server_nodes_fetched_at, Some(200));

        let mut expected = built_in();
        expected.push("https://hk.example.org".to_owned());
        expected.push("https://tokyo.example.org".to_owned());
        assert_eq!(config.effective_candidates(), expected);
        // The service's nodes are not the user's to remove.
        assert!(!config.is_removable("https://hk.example.org"));
    }

    #[test]
    fn server_nodes_are_due_after_their_ttl() {
        let mut config = GatewayOriginConfig::default();
        assert!(config.server_nodes_due(0));
        config.server_nodes_fetched_at = Some(1_000);
        assert!(!config.server_nodes_due(1_000 + SERVER_NODES_TTL_SECS - 1));
        assert!(config.server_nodes_due(1_000 + SERVER_NODES_TTL_SECS));
    }

    #[test]
    fn a_node_the_service_drops_stops_being_routed() {
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        let mut config = GatewayOriginConfig::default();
        config.adopt_server_nodes(&listed(&[("HK", "https://hk.example.org", "")]), 1);
        config.chosen = Some("https://hk.example.org".to_owned());
        assert_eq!(config.origin().as_deref(), Some("https://hk.example.org"));
        config.adopt_server_nodes(&[], 2);
        assert_eq!(config.origin().as_deref(), Some(primary.as_str()));
    }

    #[test]
    fn labels_prefer_the_primary_then_the_service_wording() {
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        let mut config = GatewayOriginConfig::default();
        assert_eq!(config.node_label(&primary), NodeLabel::Primary);
        assert_eq!(
            config.node_label("https://mine.example.org"),
            NodeLabel::Other
        );
        let cdn = normalize_base_url(brand::MANAGED_SERVICE_CDN_URL).ok();
        if let Some(cdn) = cdn.as_ref().filter(|cdn| **cdn != primary) {
            assert_eq!(config.node_label(cdn), NodeLabel::BuiltInCdn);
            config.adopt_server_nodes(&listed(&[("CDN node", cdn, "Asia")]), 1);
            assert_eq!(
                config.node_label(cdn),
                NodeLabel::Server {
                    name: "CDN node".to_owned(),
                    description: "Asia".to_owned(),
                }
            );
        }
        config.adopt_server_nodes(&listed(&[("Main", &primary, "")]), 2);
        assert_eq!(config.node_label(&primary), NodeLabel::Primary);
    }

    #[test]
    fn a_fallback_returns_to_the_origin_it_left() {
        let primary = normalize_base_url(brand::MANAGED_SERVICE_URL).expect("primary");
        let mut config = GatewayOriginConfig::default();
        assert_eq!(config.restore_target(), None);

        config
            .fall_back("https://mirror.example.org")
            .expect("fall back");
        assert_eq!(
            config.origin().as_deref(),
            Some("https://mirror.example.org")
        );
        assert_eq!(config.restore_target().as_deref(), Some(primary.as_str()));
        // A second fallback still remembers the first origin.
        config
            .fall_back("https://other.example.org")
            .expect("fall back again");
        assert_eq!(config.restore_target().as_deref(), Some(primary.as_str()));

        // Going back is a plain selection and clears the mark.
        config.select(&primary).expect("restore");
        assert_eq!(config.fallback_from, None);
        assert_eq!(config.restore_target(), None);

        // A deliberate choice made while fallen back cancels the return.
        config
            .fall_back("https://mirror.example.org")
            .expect("fall back");
        config.select("https://other.example.org").expect("choose");
        assert_eq!(config.restore_target(), None);

        // Removing the origin a fallback left also cancels it.
        config.select("https://mirror.example.org").expect("choose");
        config
            .fall_back("https://other.example.org")
            .expect("fall back");
        config.remove("https://mirror.example.org");
        assert_eq!(config.restore_target(), None);
    }
}
