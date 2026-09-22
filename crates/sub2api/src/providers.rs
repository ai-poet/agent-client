//! The provider registry — one endpoint described once, routed anywhere.
//!
//! [`crate::custom_api`] stores routing the way the app grew into it: one
//! slot per CLI, each holding its own copy of a base URL and a key. That
//! shape made a relay serving three CLIs three separate entries to keep in
//! step, and it had nowhere to record what the models behind that relay can
//! do, because a slot's model list is bare strings.
//!
//! Here a *provider* is the thing the user actually has — an address, a key,
//! the wire format it speaks, and the models it serves with their context
//! windows and reasoning tiers — and a slot merely points at one. Nothing
//! about a CLI is stored twice, and a model's metadata has an owner.
//!
//! The registry lives inside the same `custom-api.json` document, so it
//! inherits that file's atomic private write and its compatibility rules.

use serde::{Deserialize, Serialize};

use crate::custom_api::CustomEndpoint;

/// The context window assumed for a model that declares none.
///
/// Nothing can discover what sits behind somebody else's base URL, so this is
/// a display default and an estimate the user can correct — not a fact about
/// their endpoint.
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;

/// The window implied by the `[1m]` suffix relays use to mark the long-context
/// variant of a model (`claude-sonnet-4-5[1m]`).
pub const LONG_CONTEXT_WINDOW: u32 = 1_000_000;

/// The wire format an endpoint speaks.
///
/// This is a property of the *endpoint*, not of the model: the same model
/// reached through two relays can require two different shapes, and it is the
/// path the adapter appends — `/v1/messages`, `/v1/responses`,
/// `/v1/chat/completions` — that a server either implements or does not.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    /// Anthropic Messages — `/v1/messages`.
    Anthropic,
    /// OpenAI Responses — `/v1/responses`.
    OpenAiResponses,
    /// OpenAI Chat Completions — `/v1/chat/completions`. The broadest, and
    /// so the default for an endpoint whose format was never recorded.
    #[default]
    OpenAiChat,
}

impl ApiFormat {
    pub const ALL: [Self; 3] = [Self::Anthropic, Self::OpenAiResponses, Self::OpenAiChat];

    /// The path the adapter appends to the stored address.
    ///
    /// Worth showing in the UI: a server that answers
    /// `/v1/chat/completions` very often does not answer `/v1/responses`,
    /// and the address alone gives no way to tell which one an endpoint is
    /// for.
    pub fn request_path(self) -> &'static str {
        match self {
            Self::Anthropic => "/v1/messages",
            Self::OpenAiResponses => "/v1/responses",
            Self::OpenAiChat => "/v1/chat/completions",
        }
    }

    /// Whether a connectivity probe should be shaped as Anthropic's.
    pub fn is_anthropic(self) -> bool {
        matches!(self, Self::Anthropic)
    }
}

/// The one format a slot can be routed with.
///
/// Each CLI's configuration file names a single adapter, so a slot does not
/// choose: pointing `codex` at a Chat Completions endpoint would write an
/// address the Codex adapter then asks `/v1/responses` for. `None` for ids
/// this feature does not cover.
pub fn format_for_slot(slot: &str) -> Option<ApiFormat> {
    match slot {
        "claude" | "native_messages" => Some(ApiFormat::Anthropic),
        "codex" | "native_responses" => Some(ApiFormat::OpenAiResponses),
        "grok" | "opencode" | "pi" | "native_chat" => Some(ApiFormat::OpenAiChat),
        _ => None,
    }
}

/// Whether `slot` can route through an endpoint speaking `format`.
pub fn slot_accepts(slot: &str, format: ApiFormat) -> bool {
    format_for_slot(slot) == Some(format)
}

/// One model the user says their endpoint serves.
///
/// Every field but the id is optional and user-declared. This is the only
/// place that knowledge can come from — no list request tells us the context
/// window of a model behind an arbitrary relay — so the type is built to
/// carry "unknown" rather than to guess and be believed.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelEntry {
    /// What goes on the wire.
    pub id: String,
    /// What the picker shows. Empty means "show the id".
    #[serde(default)]
    pub name: String,
    /// Tokens the model accepts. `None` = not declared; see
    /// [`ModelEntry::context_window_or_default`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    /// Tokens the model may produce in one reply. `None` = not declared, and
    /// the request omits the cap rather than inventing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u32>,
    /// The reasoning efforts this model accepts, in the order to offer them
    /// (`["off", "low", "high"]`). Empty = the model is not a reasoning
    /// model, and the traits menu shows no tier for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_efforts: Vec<String>,
    /// Which of `reasoning_efforts` to start on. Ignored when it names none
    /// of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning: Option<String>,
}

impl ModelEntry {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.trim().to_owned(),
            ..Self::default()
        }
    }

    /// What to show, which is the id unless a name was given.
    pub fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.id
        } else {
            self.name.trim()
        }
    }

    /// The context window to assume.
    ///
    /// An undeclared window reads the `[1m]` suffix relays use to mark a
    /// long-context variant, and otherwise falls back to
    /// [`DEFAULT_CONTEXT_WINDOW`]. Both are defaults the user can overwrite;
    /// neither is reported as if it had been discovered.
    pub fn context_window_or_default(&self) -> u32 {
        if let Some(declared) = self.context_window.filter(|window| *window > 0) {
            return declared;
        }
        if self.id.trim_end().to_ascii_lowercase().ends_with("[1m]") {
            LONG_CONTEXT_WINDOW
        } else {
            DEFAULT_CONTEXT_WINDOW
        }
    }

    /// The effort to start on: the declared default when it is one of the
    /// offered tiers, else the first offered.
    pub fn default_reasoning_effort(&self) -> Option<&str> {
        let declared = self
            .default_reasoning
            .as_deref()
            .map(str::trim)
            .filter(|effort| self.reasoning_efforts.iter().any(|known| known == effort));
        declared.or_else(|| self.reasoning_efforts.first().map(String::as_str))
    }

    fn normalize(&mut self) {
        self.id = self.id.trim().to_owned();
        self.name = self.name.trim().to_owned();
        self.reasoning_efforts.retain(|effort| !effort.is_empty());
        if self.context_window == Some(0) {
            self.context_window = None;
        }
        if self.max_output == Some(0) {
            self.max_output = None;
        }
    }
}

fn enabled_by_default() -> bool {
    true
}

/// One endpoint the user has: where it is, how to authenticate, what it
/// speaks, and what it serves.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderEntry {
    /// Stable identity, `pr-<unix ms>-<n>`; never shown. Slots point at this.
    #[serde(default)]
    pub id: String,
    /// The user's name for it. Empty means the UI shows a placeholder.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub format: ApiFormat,
    /// The origin in use, normalized by `custom_api::normalize_base_url`.
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// Off keeps the entry and its models but stops it routing anything —
    /// the slots pointing at it fall back as if nothing were configured.
    /// Deleting would lose the models the user typed in.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Alternate origins for the same service (mirror domains, a direct IP).
    /// Always contains `base_url` once it is set.
    #[serde(default)]
    pub candidate_urls: Vec<String>,
    /// After a speed test, move `base_url` to the fastest candidate that
    /// answered.
    #[serde(default)]
    pub auto_select: bool,
}

impl Default for ProviderEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            format: ApiFormat::default(),
            base_url: String::new(),
            api_key: String::new(),
            enabled: true,
            models: Vec::new(),
            candidate_urls: Vec::new(),
            auto_select: false,
        }
    }
}

impl ProviderEntry {
    /// A fresh entry with a new id.
    pub fn new(name: &str, format: ApiFormat) -> Self {
        Self {
            id: next_provider_id(),
            name: name.trim().to_owned(),
            format,
            ..Self::default()
        }
    }

    /// Enough to route: an address and a key.
    pub fn is_usable(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }

    /// What routes, which is nothing while the entry is switched off.
    pub fn is_routable(&self) -> bool {
        self.enabled && self.is_usable()
    }

    /// The entry as the writers still want it: address, key, model ids.
    pub fn endpoint(&self) -> CustomEndpoint {
        CustomEndpoint {
            base_url: self.base_url.trim().to_owned(),
            api_key: self.api_key.trim().to_owned(),
            models: self
                .models
                .iter()
                .map(|model| model.id.trim().to_owned())
                .filter(|id| !id.is_empty())
                .collect(),
        }
    }

    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|model| model.id == id)
    }

    pub fn model_mut(&mut self, id: &str) -> Option<&mut ModelEntry> {
        self.models.iter_mut().find(|model| model.id == id)
    }

    /// Add a model. Returns false when the id is blank or already listed —
    /// two entries for one id would make "which metadata wins" a question
    /// with no answer.
    pub fn add_model(&mut self, model: ModelEntry) -> bool {
        let id = model.id.trim();
        if id.is_empty() || self.models.iter().any(|known| known.id == id) {
            return false;
        }
        let mut model = model;
        model.normalize();
        self.models.push(model);
        true
    }

    /// Replace the model list with `ids`, keeping what is already known
    /// about the ones that survive.
    ///
    /// The ids come from a form that knows only names. Rebuilding the list
    /// from them would throw away the context windows and reasoning tiers
    /// the registry exists to hold, every time the user re-ordered a line.
    /// Returns whether anything changed.
    pub fn set_model_ids<I, S>(&mut self, ids: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut kept: Vec<ModelEntry> = Vec::new();
        for id in ids {
            let id = id.as_ref().trim();
            if id.is_empty() || kept.iter().any(|model| model.id == id) {
                continue;
            }
            kept.push(
                self.model(id)
                    .cloned()
                    .unwrap_or_else(|| ModelEntry::new(id)),
            );
        }
        let changed = kept != self.models;
        self.models = kept;
        changed
    }

    pub fn remove_model(&mut self, id: &str) -> bool {
        let before = self.models.len();
        self.models.retain(|model| model.id != id);
        before != self.models.len()
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

    /// Drop a candidate. Removing the origin in use moves `base_url` to the
    /// first remaining one (or clears it when none is left).
    pub fn remove_candidate(&mut self, url: &str) {
        self.candidate_urls.retain(|known| known != url);
        if self.base_url == url {
            self.base_url = self.candidate_urls.first().cloned().unwrap_or_default();
        }
    }

    /// Route through `url`, listing it as a candidate if it was not yet.
    pub fn select_url(&mut self, url: &str) {
        let url = url.trim();
        self.add_candidate(url);
        self.base_url = url.to_owned();
    }

    fn normalize(&mut self) {
        if self.id.is_empty() {
            self.id = next_provider_id();
        }
        self.name = self.name.trim().to_owned();
        self.base_url = self.base_url.trim().to_owned();
        self.api_key = self.api_key.trim().to_owned();
        let base_url = self.base_url.clone();
        if !base_url.is_empty() {
            self.add_candidate(&base_url);
        }
        for model in &mut self.models {
            model.normalize();
        }
        self.models.retain(|model| !model.id.is_empty());
    }
}

/// Monotonic within a process, so two entries created in the same
/// millisecond still get distinct ids.
fn next_provider_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("pr-{millis}-{n}")
}

/// Every endpoint the user has described, in the order they are listed.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderRegistry {
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
}

impl ProviderRegistry {
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&ProviderEntry> {
        self.providers.iter().find(|entry| entry.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ProviderEntry> {
        self.providers.iter_mut().find(|entry| entry.id == id)
    }

    /// The entry a slot may actually route through: present, switched on,
    /// filled in, and speaking the one format that slot's adapter uses.
    pub fn routable_for_slot(&self, id: &str, slot: &str) -> Option<&ProviderEntry> {
        self.get(id)
            .filter(|entry| entry.is_routable() && slot_accepts(slot, entry.format))
    }

    /// Entries a slot could be pointed at, for the UI's picker.
    pub fn candidates_for_slot(&self, slot: &str) -> impl Iterator<Item = &ProviderEntry> {
        self.providers
            .iter()
            .filter(move |entry| slot_accepts(slot, entry.format))
    }

    /// Add an entry, giving it an id if it has none. Returns that id.
    pub fn add(&mut self, mut entry: ProviderEntry) -> String {
        entry.normalize();
        let id = entry.id.clone();
        self.providers.push(entry);
        id
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|entry| entry.id != id);
        before != self.providers.len()
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        match self.get_mut(id) {
            Some(entry) => {
                entry.name = name.trim().to_owned();
                true
            }
            None => false,
        }
    }

    /// The entry already describing this endpoint, if any.
    ///
    /// Used by the migration so one relay serving three CLIs becomes one
    /// entry rather than three — which is the whole point of the registry.
    /// Address and key must both match: the same host with two keys is two
    /// accounts, and merging them would route one CLI with the other's.
    pub fn find_matching(
        &self,
        base_url: &str,
        api_key: &str,
        format: ApiFormat,
    ) -> Option<&ProviderEntry> {
        let base_url = base_url.trim();
        let api_key = api_key.trim();
        self.providers.iter().find(|entry| {
            entry.format == format && entry.base_url == base_url && entry.api_key == api_key
        })
    }

    /// Repair what a hand-edited or partially written file may have left
    /// inconsistent: missing ids, untrimmed fields, a base URL absent from
    /// its own candidates, a model with no id.
    pub fn normalize(&mut self) {
        for entry in &mut self.providers {
            entry.normalize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping is 1:1 and total over the slots the feature covers: a
    /// slot whose format were unknown would silently stop routing.
    #[test]
    fn every_routable_slot_names_exactly_one_format() {
        for slot in crate::custom_api::CUSTOM_API_PROVIDERS {
            let format = format_for_slot(slot).expect("every slot has a format");
            assert!(slot_accepts(slot, format));
            for other in ApiFormat::ALL {
                assert_eq!(other == format, slot_accepts(slot, other), "{slot}/{other:?}");
            }
        }
        assert_eq!(format_for_slot("not-a-slot"), None);
    }

    /// The path is what makes a format checkable against somebody else's
    /// server, so each one keeps its own.
    #[test]
    fn each_format_asks_for_its_own_path() {
        let paths: Vec<_> = ApiFormat::ALL
            .into_iter()
            .map(ApiFormat::request_path)
            .collect();
        assert_eq!(
            paths,
            ["/v1/messages", "/v1/responses", "/v1/chat/completions"]
        );
    }

    /// An undeclared window is a default, and the `[1m]` marker is read
    /// rather than ignored — a long-context variant would otherwise show as
    /// the ordinary one.
    #[test]
    fn an_undeclared_context_window_falls_back_and_reads_the_marker() {
        assert_eq!(
            ModelEntry::new("gpt-5.6").context_window_or_default(),
            DEFAULT_CONTEXT_WINDOW
        );
        assert_eq!(
            ModelEntry::new("claude-sonnet-4-5[1m]").context_window_or_default(),
            LONG_CONTEXT_WINDOW
        );
        assert_eq!(
            ModelEntry::new("CLAUDE-SONNET-4-5[1M]").context_window_or_default(),
            LONG_CONTEXT_WINDOW
        );
        let declared = ModelEntry {
            context_window: Some(64_000),
            ..ModelEntry::new("claude-sonnet-4-5[1m]")
        };
        assert_eq!(declared.context_window_or_default(), 64_000);
    }

    /// A default naming a tier that is not offered would leave the picker
    /// showing something the endpoint never accepts.
    #[test]
    fn the_starting_effort_is_one_of_the_offered_tiers() {
        let mut model = ModelEntry::new("grok-4");
        assert_eq!(model.default_reasoning_effort(), None);

        model.reasoning_efforts = vec!["low".to_owned(), "high".to_owned()];
        assert_eq!(model.default_reasoning_effort(), Some("low"));

        model.default_reasoning = Some("high".to_owned());
        assert_eq!(model.default_reasoning_effort(), Some("high"));

        model.default_reasoning = Some("max".to_owned());
        assert_eq!(model.default_reasoning_effort(), Some("low"));
    }

    #[test]
    fn a_model_is_listed_once_and_keeps_its_metadata() {
        let mut entry = ProviderEntry::new("Relay", ApiFormat::OpenAiChat);
        assert!(entry.add_model(ModelEntry {
            context_window: Some(128_000),
            ..ModelEntry::new("  my-model  ")
        }));
        assert!(!entry.add_model(ModelEntry::new("my-model")));
        assert_eq!(entry.model("my-model").unwrap().context_window, Some(128_000));
        assert!(entry.remove_model("my-model"));
        assert!(!entry.remove_model("my-model"));
    }

    /// A form that only knows names must not be able to erase what the
    /// registry knows about a model that is still listed.
    #[test]
    fn replacing_the_model_list_keeps_what_survives() {
        let mut entry = ProviderEntry::new("Relay", ApiFormat::OpenAiChat);
        entry.add_model(ModelEntry {
            context_window: Some(128_000),
            ..ModelEntry::new("keeper")
        });
        entry.add_model(ModelEntry::new("goner"));

        assert!(entry.set_model_ids(["keeper", " newcomer ", "", "keeper"]));
        let ids: Vec<_> = entry.models.iter().map(|m| m.id.clone()).collect();
        assert_eq!(ids, ["keeper", "newcomer"]);
        assert_eq!(entry.model("keeper").unwrap().context_window, Some(128_000));
        assert_eq!(entry.model("newcomer").unwrap().context_window, None);

        // Writing the same list back reports no change, so a save that
        // touched nothing does not look like an edit.
        assert!(!entry.set_model_ids(["keeper", "newcomer"]));
    }

    /// The writers take ids; everything else the registry knows is for the
    /// app's own use.
    #[test]
    fn the_endpoint_projection_carries_the_model_ids() {
        let mut entry = ProviderEntry::new("Relay", ApiFormat::OpenAiChat);
        entry.base_url = "  https://mine.example.org  ".to_owned();
        entry.api_key = " key ".to_owned();
        entry.add_model(ModelEntry::new("a"));
        entry.add_model(ModelEntry::new("b"));
        let endpoint = entry.endpoint();
        assert_eq!(endpoint.base_url, "https://mine.example.org");
        assert_eq!(endpoint.api_key, "key");
        assert_eq!(endpoint.models, vec!["a".to_owned(), "b".to_owned()]);
    }

    /// Switched off is not the same as deleted: the models the user typed
    /// survive, but nothing routes through it.
    #[test]
    fn a_disabled_entry_keeps_its_models_and_routes_nothing() {
        let mut registry = ProviderRegistry::default();
        let mut entry = ProviderEntry::new("Relay", ApiFormat::OpenAiChat);
        entry.base_url = "https://mine.example.org".to_owned();
        entry.api_key = "key".to_owned();
        entry.add_model(ModelEntry::new("a"));
        entry.enabled = false;
        let id = registry.add(entry);

        assert!(registry.get(&id).unwrap().is_usable());
        assert!(!registry.get(&id).unwrap().is_routable());
        assert_eq!(registry.routable_for_slot(&id, "grok"), None);
        assert_eq!(registry.get(&id).unwrap().models.len(), 1);

        registry.get_mut(&id).unwrap().enabled = true;
        assert!(registry.routable_for_slot(&id, "grok").is_some());
    }

    /// Pointing a slot at an endpoint speaking another format would write an
    /// address the CLI's adapter then asks the wrong path for.
    #[test]
    fn a_slot_refuses_an_entry_in_another_format() {
        let mut registry = ProviderRegistry::default();
        let mut entry = ProviderEntry::new("Relay", ApiFormat::OpenAiChat);
        entry.base_url = "https://mine.example.org".to_owned();
        entry.api_key = "key".to_owned();
        let id = registry.add(entry);

        assert!(registry.routable_for_slot(&id, "native_chat").is_some());
        assert_eq!(registry.routable_for_slot(&id, "native_messages"), None);
        assert_eq!(registry.routable_for_slot(&id, "codex"), None);

        let names: Vec<_> = registry
            .candidates_for_slot("opencode")
            .map(|entry| entry.id.clone())
            .collect();
        assert_eq!(names, vec![id]);
        assert_eq!(registry.candidates_for_slot("claude").count(), 0);
    }

    /// The same host with two keys is two accounts, not one entry.
    #[test]
    fn matching_takes_the_key_into_account() {
        let mut registry = ProviderRegistry::default();
        let mut first = ProviderEntry::new("A", ApiFormat::OpenAiChat);
        first.base_url = "https://mine.example.org".to_owned();
        first.api_key = "one".to_owned();
        let id = registry.add(first);

        assert_eq!(
            registry
                .find_matching("https://mine.example.org", "one", ApiFormat::OpenAiChat)
                .map(|entry| entry.id.clone()),
            Some(id)
        );
        assert!(
            registry
                .find_matching("https://mine.example.org", "two", ApiFormat::OpenAiChat)
                .is_none()
        );
        assert!(
            registry
                .find_matching("https://mine.example.org", "one", ApiFormat::Anthropic)
                .is_none()
        );
    }

    /// Ids are assigned on the way in, so a caller never has to mint one.
    #[test]
    fn normalizing_fills_in_ids_and_candidates() {
        let mut registry = ProviderRegistry::default();
        registry.providers.push(ProviderEntry {
            base_url: "  https://mine.example.org  ".to_owned(),
            models: vec![ModelEntry::new("  "), ModelEntry::new("kept")],
            ..ProviderEntry::default()
        });
        registry.normalize();
        let entry = &registry.providers[0];
        assert!(entry.id.starts_with("pr-"));
        assert_eq!(entry.base_url, "https://mine.example.org");
        assert_eq!(entry.candidate_urls, vec!["https://mine.example.org"]);
        assert_eq!(entry.models.len(), 1);
        assert_eq!(entry.models[0].id, "kept");
    }

    /// A file written by a build that predates `enabled` must not read as a
    /// registry of switched-off entries.
    #[test]
    fn an_entry_without_the_enabled_key_is_on() {
        let entry: ProviderEntry =
            serde_json::from_str(r#"{"id":"pr-1","base_url":"https://x.test","api_key":"k"}"#)
                .expect("decodes");
        assert!(entry.enabled);
        assert_eq!(entry.format, ApiFormat::OpenAiChat);
    }
}
