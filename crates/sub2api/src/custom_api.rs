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

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brand;
use crate::global_config::atomic_write_private;
use crate::providers::{ApiFormat, ProviderEntry, ProviderRegistry, format_for_slot};

/// Key the legacy daemon-settings transport used; read once for migration.
pub const LEGACY_SETTINGS_KEY: &str = "sub2apiCustomApi";

/// Everything a custom endpoint can be set for, in display order.
///
/// The first three are the built-in agent, which is not a CLI: it speaks
/// three APIs and each one is reached separately, so it holds three
/// endpoints rather than one. The rest are CLIs, one endpoint each, written
/// into that CLI's own configuration file.
pub const CUSTOM_API_PROVIDERS: [&str; 8] = [
    "native_messages",
    "native_responses",
    "native_chat",
    "claude",
    "codex",
    "grok",
    "opencode",
    "pi",
];

/// The built-in agent's three endpoints, in the order the picker lists the
/// APIs they serve. Each maps to one entry in the engine's own settings:
/// `anthropic`, `codex`, `openai` respectively.
pub const NATIVE_SLOTS: [&str; 3] = ["native_messages", "native_responses", "native_chat"];

/// The single built-in-agent endpoint earlier builds kept, before it was
/// split in three. Read once, copied into the three, and then left empty —
/// see [`CustomApiConfig::normalize`].
pub const LEGACY_NATIVE_PROVIDER: &str = "native";

/// Whether an endpoint speaks Anthropic's wire format, which decides how its
/// connectivity test is shaped and what the form's protocol hint says.
pub fn uses_anthropic_shape(provider_id: &str) -> bool {
    matches!(provider_id, "claude" | "native_messages")
}

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
    /// The registry entry this profile routes through ([`crate::providers`]).
    ///
    /// When set, *that entry* is what routes; the endpoint fields above are
    /// the pre-registry copy, kept so a build that predates the registry
    /// still finds an address here rather than losing the route. A ref that
    /// no longer resolves — entry deleted, switched off, or in another wire
    /// format — means the slot routes nothing, because unbinding it was a
    /// decision, not a reason to fall back to a stale copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<String>,
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

/// The host of a normalized origin, for naming an entry the user never
/// named. `https://user@api.relay.org:8443/v1` becomes `api.relay.org`.
fn host_of(base_url: &str) -> Option<String> {
    let rest = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    // An IPv6 literal carries colons of its own, so the port is whatever
    // follows the closing bracket rather than the first colon.
    let host = match authority.strip_prefix('[') {
        Some(inside) => inside.split_once(']').map(|(host, _)| host)?,
        None => authority.split(':').next()?,
    };
    (!host.is_empty()).then(|| host.to_owned())
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
            provider_ref: None,
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
    /// The built-in agent over Anthropic Messages — the engine's
    /// `anthropic` provider entry.
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub native_messages: ProviderProfiles,
    /// The built-in agent over OpenAI Responses — the engine's `codex`
    /// provider entry.
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub native_responses: ProviderProfiles,
    /// The built-in agent over OpenAI Chat Completions — the engine's
    /// `openai` provider entry. The only one that carries a model list:
    /// nothing discovers what sits behind somebody else's endpoint.
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub native_chat: ProviderProfiles,
    /// The built-in agent's single endpoint, as earlier builds stored it.
    /// Kept so those files still load; emptied into the three above on the
    /// first read. A build that predates the split ignores the three new
    /// keys and finds this one empty, which loses the custom route but
    /// cannot corrupt anything.
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
    /// Every endpoint the user has described, independent of which slot uses
    /// it. The slots above point into this by `provider_ref`; see
    /// [`crate::providers`] for why the description lives here rather than
    /// being copied per CLI.
    #[serde(default)]
    pub registry: ProviderRegistry,
}

impl CustomApiConfig {
    pub fn is_empty(&self) -> bool {
        // The legacy slot counts: a file holding only that one is not empty,
        // it is un-migrated, and calling it empty would throw the user's
        // endpoint away. So does a registry entry no slot points at yet —
        // describing an endpoint and binding it are separate acts.
        self.registry.is_empty()
            && self.get(LEGACY_NATIVE_PROVIDER).is_none()
            && CUSTOM_API_PROVIDERS
                .into_iter()
                .all(|provider| self.get(provider).is_none())
    }

    /// Bring a stored document up to the current shape.
    ///
    /// Today that means splitting the built-in agent's old single endpoint
    /// into the three it now has. That is behaviour-preserving: the writer
    /// already pointed all three of the engine's provider entries at that
    /// one address, so three copies of it route exactly where one did. Only
    /// the model list is not copied three ways — it describes models reached
    /// over Chat Completions, and that is the one slot that lists models.
    ///
    /// Idempotent, and it cannot resurrect anything: the guard is the legacy
    /// slot being non-empty, and the last thing it does is empty it.
    pub fn normalize(&mut self) -> bool {
        self.registry.normalize();
        let split = self.split_legacy_native();
        // After the split, so the three slots it just filled are described
        // too rather than being adopted only on the next load.
        let adopted = self.adopt_into_registry();
        split || adopted
    }

    fn split_legacy_native(&mut self) -> bool {
        if self.native.is_empty() {
            return false;
        }
        let legacy = std::mem::take(&mut self.native);
        for slot in NATIVE_SLOTS {
            let Some(target) = self.profiles_mut(slot) else {
                continue;
            };
            if !target.is_empty() {
                continue;
            }
            let mut copy = legacy.clone();
            if slot != "native_chat" {
                for profile in &mut copy.profiles {
                    profile.endpoint.models.clear();
                }
            }
            *target = copy;
        }
        true
    }

    /// Describe every configured slot in the registry, once.
    ///
    /// The pre-registry file holds its own copy of an address and a key per
    /// CLI. This turns each into a registry entry and leaves the slot
    /// pointing at it; two slots holding the same address, key and wire
    /// format become one entry, which is the whole point — the relay serving
    /// Grok, OpenCode and Pi stops being three things to keep in step.
    /// Slots in different formats never merge, because the format is part of
    /// what an endpoint is.
    ///
    /// The slot's own fields are left exactly as they were. A build that
    /// predates the registry ignores `provider_ref` and finds them where it
    /// wrote them, so installing this version and going back loses nothing.
    ///
    /// Idempotent: a slot that already carries a ref is left alone, so this
    /// runs on every load without accumulating anything.
    fn adopt_into_registry(&mut self) -> bool {
        let mut adopted = false;
        for slot in CUSTOM_API_PROVIDERS {
            let Some(format) = format_for_slot(slot) else {
                continue;
            };
            // Read the slot out before touching the registry: both live in
            // this struct, and only one of them can be borrowed at a time.
            let Some(profile) = self.active_profile(slot).cloned() else {
                continue;
            };
            if profile.provider_ref.is_some() || !profile.endpoint.is_usable() {
                continue;
            }
            let base_url = profile.endpoint.base_url.trim();
            let api_key = profile.endpoint.api_key.trim();
            let id = match self.registry.find_matching(base_url, api_key, format) {
                Some(existing) => existing.id.clone(),
                None => {
                    let name = match profile.name.trim() {
                        "" => host_of(base_url).unwrap_or_else(|| slot.to_owned()),
                        named => named.to_owned(),
                    };
                    let mut entry = ProviderEntry::new(&name, format);
                    entry.base_url = base_url.to_owned();
                    entry.api_key = api_key.to_owned();
                    entry.candidate_urls = profile.candidate_urls.clone();
                    entry.auto_select = profile.auto_select;
                    entry.set_model_ids(&profile.endpoint.models);
                    self.registry.add(entry)
                }
            };
            if let Some(active) = self
                .profiles_mut(slot)
                .and_then(ProviderProfiles::active_profile_mut)
            {
                active.provider_ref = Some(id);
                adopted = true;
            }
        }
        adopted
    }

    /// The endpoint bound to `slot`, following its `provider_ref`.
    ///
    /// This is the routing answer. [`CustomApiConfig::get`] is the storage
    /// answer — what this slot's own fields say — and the two differ once a
    /// slot is bound: the registry entry is what a request actually reaches.
    pub fn resolved_endpoint(&self, slot: &str) -> Option<Cow<'_, CustomEndpoint>> {
        let profile = self.active_profile(slot)?;
        match profile.provider_ref.as_deref() {
            Some(reference) => self
                .registry
                .routable_for_slot(reference, slot)
                .map(|entry| Cow::Owned(entry.endpoint())),
            None => Some(Cow::Borrowed(&profile.endpoint)),
        }
    }

    /// The endpoint that should route `slot`, if a usable one is bound.
    pub fn routed_endpoint(&self, slot: &str) -> Option<Cow<'_, CustomEndpoint>> {
        self.resolved_endpoint(slot)
            .filter(|endpoint| endpoint.is_usable())
    }

    /// The registry entry `slot` is bound to, whatever its state.
    ///
    /// Unlike [`CustomApiConfig::resolved_endpoint`] this answers even for an
    /// entry that is switched off or in the wrong format, which is what the
    /// settings page needs in order to say so.
    pub fn bound_provider(&self, slot: &str) -> Option<&ProviderEntry> {
        let reference = self.active_profile(slot)?.provider_ref.as_deref()?;
        self.registry.get(reference)
    }

    /// Point `slot` at a registry entry, or at nothing.
    ///
    /// Creates a profile to hold the binding when the slot has none, since a
    /// slot with no profile has nowhere to record one.
    pub fn bind_provider(&mut self, slot: &str, provider_id: Option<&str>) -> bool {
        let Some(profiles) = self.profiles_mut(slot) else {
            return false;
        };
        if profiles.active_profile().is_none() {
            if provider_id.is_none() {
                return false;
            }
            profiles.add("");
        }
        let Some(active) = profiles.active_profile_mut() else {
            return false;
        };
        active.provider_ref = provider_id.map(str::to_owned);
        true
    }

    /// One CLI's profiles. `None` for ids this feature does not cover.
    pub fn profiles(&self, provider_id: &str) -> Option<&ProviderProfiles> {
        match provider_id {
            "native_messages" => Some(&self.native_messages),
            "native_responses" => Some(&self.native_responses),
            "native_chat" => Some(&self.native_chat),
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
            "native_messages" => Some(&mut self.native_messages),
            "native_responses" => Some(&mut self.native_responses),
            "native_chat" => Some(&mut self.native_chat),
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
        let mut write_through = None;
        match endpoint {
            Some(endpoint) => {
                if slot.active_profile().is_none() {
                    slot.add("");
                }
                let profile = slot
                    .active_profile_mut()
                    .expect("a profile was just added");
                profile.endpoint = endpoint.clone();
                profile.normalize();
                write_through = profile
                    .provider_ref
                    .clone()
                    .map(|reference| (reference, endpoint));
            }
            None => {
                if let Some(id) = slot.active.clone() {
                    slot.remove(&id);
                }
            }
        }
        // While a slot is bound, the registry entry is what routes — an edit
        // that only touched the slot's own copy would look like it had been
        // saved and change nothing. Model ids are merged rather than
        // replaced, because this caller knows names and the entry knows
        // context windows.
        if let Some((reference, endpoint)) = write_through
            && let Some(entry) = self.registry.get_mut(&reference)
        {
            entry.select_url(&endpoint.base_url);
            entry.api_key = endpoint.api_key.trim().to_owned();
            entry.set_model_ids(&endpoint.models);
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
    let mut config: CustomApiConfig = config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    // Every read goes through here, so no caller can see the pre-split
    // shape. The write back is incidental — the next save records it.
    config.normalize();
    config
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
    let mut config: CustomApiConfig = serde_json::from_value(value).ok()?;
    config.normalize();
    Some(config)
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
    probe_request_for_format(
        if uses_anthropic_shape(provider_id) {
            ApiFormat::Anthropic
        } else {
            ApiFormat::OpenAiChat
        },
        base_url,
        api_key,
        timeout_secs,
    )
}

/// The same probe, described by the endpoint's own wire format rather than
/// by which slot happens to use it — which is how the registry knows it.
pub fn probe_request_for_format(
    format: ApiFormat,
    base_url: &str,
    api_key: &str,
    timeout_secs: u32,
) -> (String, crate::http::Request) {
    let key = api_key.trim();
    let mut request = crate::http::Request::new().timeout_seconds(timeout_secs);
    if format.is_anthropic() {
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

/// [`probe_endpoint`] for an endpoint described by its wire format.
pub fn probe_endpoint_for_format(format: ApiFormat, base_url: &str, api_key: &str) -> ProbeResult {
    let (url, request) = probe_request_for_format(format, base_url, api_key, PROBE_TIMEOUT_SECS);
    send_probe(&url, request)
}

/// [`probe_endpoint`] with an explicit timeout.
pub fn probe_endpoint_with_timeout(
    provider_id: &str,
    base_url: &str,
    api_key: &str,
    timeout_secs: u32,
) -> ProbeResult {
    let (url, request) = probe_request_with_timeout(provider_id, base_url, api_key, timeout_secs);
    send_probe(&url, request)
}

fn send_probe(url: &str, request: crate::http::Request) -> ProbeResult {
    let started = std::time::Instant::now();
    match request.send(url) {
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

    /// The built-in agent used to hold one endpoint that the writer fanned
    /// out to all three of the engine's provider entries. Three copies of it
    /// route exactly where that one did, so the split is behaviour-
    /// preserving — and the model list goes only to the slot that has one.
    /// The Messages route talks to an Anthropic server, so its connectivity
    /// test has to be shaped like one — the other two are OpenAI-shaped.
    #[test]
    fn each_native_route_is_probed_with_its_own_wire_shape() {
        let (url, request) = probe_request("native_messages", "https://mine.example.org", "sk-a");
        assert_eq!(url, "https://mine.example.org/v1/models");
        let anthropic = format!("{request:?}");
        assert!(anthropic.contains("anthropic-version"), "{anthropic}");
        assert!(anthropic.contains("x-api-key"), "{anthropic}");

        for slot in ["native_responses", "native_chat"] {
            let (url, request) = probe_request(slot, "https://mine.example.org", "sk-a");
            assert_eq!(url, "https://mine.example.org/v1/models", "{slot}");
            let openai = format!("{request:?}");
            assert!(!openai.contains("anthropic-version"), "{slot}: {openai}");
        }
    }

    #[test]
    fn the_legacy_native_endpoint_splits_into_three_routes() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"native":{"base_url":"https://mine.example.org","api_key":"sk-mine","models":["m1"]}}"#,
        )
        .unwrap();
        assert!(config.normalize());

        for slot in NATIVE_SLOTS {
            let endpoint = config.get(slot).unwrap_or_else(|| panic!("{slot}"));
            assert_eq!(endpoint.base_url, "https://mine.example.org", "{slot}");
            assert_eq!(endpoint.api_key, "sk-mine", "{slot}");
        }
        assert_eq!(config.get("native_chat").unwrap().models, ["m1"]);
        assert!(config.get("native_messages").unwrap().models.is_empty());
        assert!(config.get("native_responses").unwrap().models.is_empty());
        assert!(config.get(LEGACY_NATIVE_PROVIDER).is_none());
    }

    /// The split must not undo a later edit, and must not come back after the
    /// user clears the three slots.
    #[test]
    fn the_split_runs_once_and_never_resurrects() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"native":{"base_url":"https://old.example.org","api_key":"sk-old"}}"#,
        )
        .unwrap();
        assert!(config.normalize());
        // A second pass has nothing left to do.
        assert!(!config.normalize());

        config.set("native_messages", None);
        config.set("native_responses", None);
        config.set("native_chat", None);
        assert!(!config.normalize());
        for slot in NATIVE_SLOTS {
            assert!(config.get(slot).is_none(), "{slot}");
            assert!(config.routed_endpoint(slot).is_none(), "{slot}");
        }
        assert!(config.get(LEGACY_NATIVE_PROVIDER).is_none());
        // The description of the endpoint survives being unbound: a user who
        // clears a slot has stopped using an endpoint, not forgotten it.
        // Three of them, because the legacy endpoint was used in all three
        // wire formats and a server implementing `/v1/messages` need not
        // implement `/v1/responses` — merging them would claim it does.
        assert_eq!(config.registry.providers.len(), 3);
        assert!(!config.is_empty());
    }

    /// The point of the registry: slots sharing an endpoint share one entry,
    /// so the address and key are typed once and edited once.
    #[test]
    fn slots_on_the_same_endpoint_and_format_adopt_one_entry() {
        let slot = |name: &str| {
            format!(
                r#""{name}":{{"profiles":[{{"id":"p-{name}","name":"",
                  "base_url":"https://relay.example.org","api_key":"sk-one",
                  "models":["m1"]}}],"active":"p-{name}"}}"#
            )
        };
        let raw = format!(
            "{{{},{},{}}}",
            slot("grok"),
            slot("opencode"),
            slot("pi")
        );
        let mut config: CustomApiConfig = serde_json::from_str(&raw).expect("decodes");
        assert!(config.normalize());

        assert_eq!(config.registry.providers.len(), 1);
        let entry = &config.registry.providers[0];
        // Named after the host, since no profile carried a name.
        assert_eq!(entry.name, "relay.example.org");
        assert_eq!(entry.format, crate::providers::ApiFormat::OpenAiChat);
        assert_eq!(entry.models.len(), 1);

        for name in ["grok", "opencode", "pi"] {
            assert_eq!(
                config.active_profile(name).unwrap().provider_ref.as_deref(),
                Some(entry.id.as_str()),
                "{name}"
            );
            assert_eq!(
                config.routed_endpoint(name).unwrap().base_url,
                "https://relay.example.org",
                "{name}"
            );
        }
    }

    /// Switching the entry off stops every slot bound to it, and does not
    /// quietly fall back to the copy the slot still carries.
    #[test]
    fn unbinding_or_disabling_the_entry_stops_the_route() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"grok":{"profiles":[{"id":"p1","name":"",
                 "base_url":"https://relay.example.org","api_key":"sk-one"}],"active":"p1"}}"#,
        )
        .expect("decodes");
        assert!(config.normalize());
        let id = config.registry.providers[0].id.clone();
        assert!(config.routed_endpoint("grok").is_some());
        // The slot's own copy is still there, which is what makes a
        // downgrade safe.
        assert_eq!(config.get("grok").unwrap().base_url, "https://relay.example.org");

        config.registry.get_mut(&id).unwrap().enabled = false;
        assert!(config.routed_endpoint("grok").is_none());
        assert!(config.bound_provider("grok").is_some());

        config.registry.get_mut(&id).unwrap().enabled = true;
        assert!(config.bind_provider("grok", None));
        // Unbound, the slot falls back to its own fields rather than to
        // nothing: that is the pre-registry path, still intact.
        assert_eq!(
            config.routed_endpoint("grok").unwrap().base_url,
            "https://relay.example.org"
        );
        assert!(config.bound_provider("grok").is_none());
    }

    /// Editing through the old per-CLI form must reach the entry that
    /// actually routes, or the save would appear to do nothing.
    #[test]
    fn writing_a_bound_slot_reaches_the_entry_and_keeps_model_metadata() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"pi":{"profiles":[{"id":"p1","name":"",
                 "base_url":"https://old.example.org","api_key":"sk-old",
                 "models":["keeper","goner"]}],"active":"p1"}}"#,
        )
        .expect("decodes");
        assert!(config.normalize());
        let id = config.registry.providers[0].id.clone();
        config
            .registry
            .get_mut(&id)
            .unwrap()
            .model_mut("keeper")
            .unwrap()
            .context_window = Some(128_000);

        config.set(
            "pi",
            Some(CustomEndpoint {
                base_url: "https://new.example.org".to_owned(),
                api_key: "sk-new".to_owned(),
                models: vec!["keeper".to_owned(), "newcomer".to_owned()],
            }),
        );

        let routed = config.routed_endpoint("pi").expect("still routed");
        assert_eq!(routed.base_url, "https://new.example.org");
        assert_eq!(routed.api_key, "sk-new");
        assert_eq!(routed.models, ["keeper", "newcomer"]);

        let entry = config.registry.get(&id).expect("the same entry");
        assert_eq!(entry.model("keeper").unwrap().context_window, Some(128_000));
        assert!(entry.model("goner").is_none());
        // The old origin stays listed, so a speed test can still reach it.
        assert!(entry.candidate_urls.contains(&"https://old.example.org".to_owned()));
    }

    /// Both halves have to survive the file: the binding, and the copy that
    /// makes going back to an older build safe.
    #[test]
    fn the_registry_and_the_binding_round_trip_through_json() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"grok":{"profiles":[{"id":"p1","name":"Relay",
                 "base_url":"https://relay.example.org","api_key":"sk-one"}],"active":"p1"}}"#,
        )
        .expect("decodes");
        assert!(config.normalize());

        let encoded = serde_json::to_string(&config).expect("encodes");
        // An older build reads this file and still finds an address.
        assert!(encoded.contains(r#""base_url":"https://relay.example.org""#));
        assert!(encoded.contains(r#""provider_ref""#));

        let mut reloaded: CustomApiConfig = serde_json::from_str(&encoded).expect("decodes again");
        // Nothing left to migrate, and nothing duplicated by trying.
        assert!(!reloaded.normalize());
        assert_eq!(reloaded, config);
        assert_eq!(reloaded.registry.providers.len(), 1);
        assert_eq!(
            reloaded.routed_endpoint("grok").unwrap().base_url,
            "https://relay.example.org"
        );
    }

    /// A ref pointing at nothing is not a reason to use the stale copy the
    /// slot still carries — the entry was deleted on purpose.
    #[test]
    fn a_dangling_reference_routes_nothing() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"grok":{"profiles":[{"id":"p1","name":"",
                 "base_url":"https://relay.example.org","api_key":"sk-one"}],"active":"p1"}}"#,
        )
        .expect("decodes");
        assert!(config.normalize());
        let id = config.registry.providers[0].id.clone();
        assert!(config.registry.remove(&id));

        assert!(config.routed_endpoint("grok").is_none());
        assert!(config.bound_provider("grok").is_none());
        // Still stored, and still the thing a downgrade would read.
        assert_eq!(config.get("grok").unwrap().base_url, "https://relay.example.org");
    }

    /// A slot the user already filled in is not overwritten by the old one.
    #[test]
    fn the_split_leaves_a_slot_the_user_already_set() {
        let mut config: CustomApiConfig = serde_json::from_str(
            r#"{"native":{"base_url":"https://old.example.org","api_key":"sk-old"},
                "native_chat":{"profiles":[{"id":"p1","name":"mine",
                  "base_url":"https://new.example.org","api_key":"sk-new"}],"active":"p1"}}"#,
        )
        .unwrap();
        assert!(config.normalize());
        assert_eq!(config.get("native_chat").unwrap().base_url, "https://new.example.org");
        assert_eq!(config.get("native_messages").unwrap().base_url, "https://old.example.org");
    }

    /// A file holding only the pre-split slot is not empty — calling it that
    /// would throw the user's endpoint away.
    #[test]
    fn a_file_holding_only_the_legacy_slot_is_not_empty() {
        let config: CustomApiConfig = serde_json::from_str(
            r#"{"native":{"base_url":"https://mine.example.org","api_key":"sk-mine"}}"#,
        )
        .unwrap();
        assert!(!config.is_empty());
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
