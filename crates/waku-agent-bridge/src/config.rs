//! Turning Waku's session options into the engine's configuration.
//!
//! Two objects come out of this: a [`Config`], which is process-and-project
//! scoped (permissions, MCP roster, hooks, routing), and a [`QueryConfig`],
//! which is *per turn* — model, effort, budgets. That split is what lets a
//! model change take effect on the next turn without restarting anything.

use std::path::PathBuf;

use claurst_core::config::{Config, Settings};
use claurst_core::effort::EffortLevel;
use claurst_core::{PermissionMode, ProviderConfig};
use claurst_query::QueryConfig;

use crate::events::PermissionChoice;

/// Which API a session speaks. The gateway translates every one of them for
/// every platform, so this is the user's choice, not the model's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireFormat {
    /// Anthropic Messages — the engine's primary path.
    Messages,
    /// OpenAI Responses — what Codex speaks.
    Responses,
    /// OpenAI Chat Completions.
    Chat,
}

impl WireFormat {
    pub const ALL: [Self; 3] = [Self::Messages, Self::Responses, Self::Chat];

    /// Stable id, used as the model picker's tier id and on the wire.
    pub fn id(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Responses => "responses",
            Self::Chat => "chat",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.id() == id)
    }

    /// The engine provider that speaks this format.
    pub fn engine_provider(self) -> &'static str {
        match self {
            Self::Messages => "anthropic",
            Self::Responses => "codex",
            Self::Chat => "openai",
        }
    }

    /// What a platform speaks natively. OpenAI-keyed groups are Codex
    /// groups, whose native route is Responses; everything else defaults to
    /// the engine's primary path, which the gateway serves for every
    /// platform.
    pub fn default_for_platform(platform: Option<&str>) -> Self {
        match platform.map(|platform| platform.trim().to_ascii_lowercase()) {
            Some(platform) if platform == "openai" => Self::Responses,
            _ => Self::Messages,
        }
    }
}

/// The model id the picker sends carries the platform ahead of a `::`, so a
/// bare id — the fallback list, a session from before this existed — reads
/// as Anthropic. Nothing downstream ever sees the prefix.
pub fn split_model(id: &str) -> (Option<String>, String) {
    match id.split_once("::") {
        Some((platform, model)) if !platform.is_empty() && !model.is_empty() => {
            (Some(platform.to_owned()), model.to_owned())
        }
        _ => (None, id.to_owned()),
    }
}

/// How much the agent may do without asking. Mirrors Waku's `RuntimeMode`,
/// which this crate cannot name without depending on `waku-core`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    /// Ask before every gated tool.
    Ask,
    /// Edits go through; commands still ask.
    AutoAcceptEdits,
    /// Answer on the user's behalf, preferring the durable allow so the same
    /// tool is not asked about twice.
    Auto,
    /// Never ask.
    FullAccess,
}

impl AccessMode {
    /// The standing answer for the permission bridge, or `None` when the user
    /// should be asked.
    pub fn auto_answer(self) -> Option<PermissionChoice> {
        match self {
            Self::Ask => None,
            // Edit approval is expressed through `PermissionMode::AcceptEdits`
            // below, which lets edits past without a request at all. Anything
            // that still reaches the dialog in this mode is a command, and the
            // user asked to be consulted about those.
            Self::AutoAcceptEdits => None,
            Self::Auto => Some(PermissionChoice::AllowAlways),
            Self::FullAccess => Some(PermissionChoice::AllowOnce),
        }
    }

    fn permission_mode(self, plan: bool) -> PermissionMode {
        if plan {
            // Plan outranks access: the point of Plan is that nothing is
            // applied, whatever the access mode says.
            return PermissionMode::Plan;
        }
        match self {
            Self::Ask | Self::Auto => PermissionMode::Default,
            Self::AutoAcceptEdits => PermissionMode::AcceptEdits,
            Self::FullAccess => PermissionMode::BypassPermissions,
        }
    }
}

/// Everything a session needs at start.
#[derive(Clone, Debug)]
pub struct AgentStartOptions {
    pub cwd: PathBuf,
    pub access_mode: AccessMode,
    /// Waku's `InteractionMode::Plan`.
    pub plan_mode: bool,
    /// The bare model id, without the platform prefix.
    pub model: Option<String>,
    /// The gateway platform the model belongs to (`anthropic`, `openai`,
    /// `gemini`, …). Decides which of the account's keys is used.
    pub platform: Option<String>,
    /// `None` takes the platform's native format.
    pub wire_format: Option<WireFormat>,
    /// Waku's reasoning-effort id (`low`, `medium`, `high`, `xhigh`, `max`).
    pub reasoning_effort: Option<String>,
    /// A previous conversation to resume, as written by
    /// [`crate::history::serialize`]. Empty for a fresh session.
    pub history: Vec<u8>,
}

impl Default for AgentStartOptions {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            access_mode: AccessMode::Ask,
            plan_mode: false,
            model: None,
            platform: None,
            wire_format: None,
            reasoning_effort: None,
            history: Vec::new(),
        }
    }
}

impl AgentStartOptions {
    pub fn wire_format(&self) -> WireFormat {
        self.wire_format
            .unwrap_or_else(|| WireFormat::default_for_platform(self.platform.as_deref()))
    }
}

/// The per-turn options that can change without a restart.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TurnOptions {
    pub access_mode: Option<AccessMode>,
    pub plan_mode: Option<bool>,
    pub model: Option<String>,
    pub platform: Option<Option<String>>,
    pub wire_format: Option<Option<WireFormat>>,
    pub reasoning_effort: Option<String>,
}

/// Build the project-scoped config.
///
/// Starts from the engine's own settings file, then applies what Waku owns per
/// session.
///
/// **Routing is deliberately not applied here.** Which endpoint and key the
/// agent uses is written into that same settings file by
/// `sub2api::global_config::native`, the way every other provider's routing is
/// written into its own config — so one place decides whether the signed-in
/// account or a custom endpoint wins. Resolving it a second time here would be
/// a second answer to that question, and the two would disagree the moment a
/// user has both.
pub fn build_config(options: &AgentStartOptions) -> Config {
    let settings = Settings::load_sync().unwrap_or_default();
    let mut config = settings.effective_config();

    config.project_dir = Some(options.cwd.clone());
    if config.workspace_paths.is_empty() {
        config.workspace_paths = vec![options.cwd.clone()];
    }
    config.permission_mode = options.access_mode.permission_mode(options.plan_mode);
    if let Some(model) = &options.model {
        config.model = Some(model.clone());
    }

    select_route(&mut config, options);
    config
}

/// Point the session at the engine provider for its wire format, with the
/// key for its model's platform.
///
/// The routing writer stores every gateway key under the anthropic entry's
/// `options.gateway_keys`, filed by platform; this reads the right one and
/// makes it the key for the provider entry the format uses — and the
/// top-level key, which the engine resolves first. An entry the format needs
/// but the writer did not create (a self-hosted setup) inherits the anthropic
/// entry's base, so a custom endpoint works for all three formats too.
fn select_route(config: &mut Config, options: &AgentStartOptions) {
    let format = options.wire_format();
    let provider = format.engine_provider();

    let anthropic = config.provider_configs.get("anthropic").cloned();
    let gateway_keys = anthropic
        .as_ref()
        .and_then(|entry| entry.options.get(GATEWAY_KEYS_OPTION))
        .and_then(|value| value.as_object().cloned());
    let platform = options
        .platform
        .as_deref()
        .map(|platform| platform.trim().to_ascii_lowercase());
    let key = gateway_keys.as_ref().and_then(|keys| {
        platform
            .as_deref()
            .and_then(|platform| keys.get(platform))
            .or_else(|| keys.get("default"))
            .or_else(|| keys.get("anthropic"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    });

    config.provider = Some(provider.to_owned());
    let entry = config
        .provider_configs
        .entry(provider.to_owned())
        .or_insert_with(ProviderConfig::default);
    if entry.api_base.as_deref().is_none_or(str::is_empty) {
        entry.api_base = anthropic.as_ref().and_then(|entry| entry.api_base.clone());
    }
    entry.enabled = true;
    if let Some(key) = key {
        entry.api_key = Some(key.clone());
        config.api_key = Some(key);
    } else if provider != "anthropic" && entry.api_key.as_deref().is_none_or(str::is_empty) {
        entry.api_key = anthropic.as_ref().and_then(|entry| entry.api_key.clone());
    }
}

/// Where the routing writer files the per-platform keys. Kept in step with
/// `sub2api::global_config::native::GATEWAY_KEYS_OPTION`, which this crate
/// cannot import.
pub const GATEWAY_KEYS_OPTION: &str = "gateway_keys";

/// Build the per-turn config from the project config plus this turn's options.
pub fn build_query_config(config: &Config, options: &AgentStartOptions) -> QueryConfig {
    let mut query = QueryConfig::from_config(config);
    query.working_directory = Some(options.cwd.display().to_string());
    if let Some(model) = &options.model {
        query.model = model.clone();
    }
    query.effort_level = options
        .reasoning_effort
        .as_deref()
        .and_then(EffortLevel::from_str);
    // Let the effort level own the thinking budget. `from_config` may have
    // copied a fixed budget out of the user's settings, and leaving both set
    // would make the effort picker look connected while changing nothing.
    if query.effort_level.is_some() {
        query.thinking_budget = None;
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_outranks_every_access_mode() {
        for mode in [
            AccessMode::Ask,
            AccessMode::AutoAcceptEdits,
            AccessMode::Auto,
            AccessMode::FullAccess,
        ] {
            assert_eq!(mode.permission_mode(true), PermissionMode::Plan);
        }
    }

    #[test]
    fn a_platform_prefix_is_split_off_and_a_bare_id_is_anthropic() {
        assert_eq!(
            split_model("openai::gpt-5.6-sol"),
            (Some("openai".into()), "gpt-5.6-sol".into())
        );
        assert_eq!(split_model("claude-sonnet-5"), (None, "claude-sonnet-5".into()));
        // A model namespace with a single slash is not a platform prefix.
        assert_eq!(
            split_model("meta-llama/Llama-3.3"),
            (None, "meta-llama/Llama-3.3".into())
        );
    }

    #[test]
    fn openai_platforms_default_to_responses_and_the_rest_to_messages() {
        assert_eq!(
            WireFormat::default_for_platform(Some("openai")),
            WireFormat::Responses
        );
        assert_eq!(
            WireFormat::default_for_platform(Some("anthropic")),
            WireFormat::Messages
        );
        assert_eq!(WireFormat::default_for_platform(Some("gemini")), WireFormat::Messages);
        assert_eq!(WireFormat::default_for_platform(None), WireFormat::Messages);
    }

    #[test]
    fn the_route_takes_the_platforms_key_and_the_formats_provider() {
        let mut config = Config::default();
        let mut anthropic = ProviderConfig::default();
        anthropic.api_key = Some("sk-claude".into());
        anthropic.api_base = Some("https://gw.example".into());
        anthropic.options.insert(
            GATEWAY_KEYS_OPTION.into(),
            serde_json::json!({"anthropic": "sk-claude", "openai": "sk-codex", "default": "sk-general"}),
        );
        config.provider_configs.insert("anthropic".into(), anthropic);

        let options = AgentStartOptions {
            platform: Some("openai".into()),
            wire_format: None,
            ..AgentStartOptions::default()
        };
        select_route(&mut config, &options);
        assert_eq!(config.provider.as_deref(), Some("codex"));
        assert_eq!(config.api_key.as_deref(), Some("sk-codex"));
        let codex = config.provider_configs.get("codex").unwrap();
        assert_eq!(codex.api_key.as_deref(), Some("sk-codex"));
        assert_eq!(codex.api_base.as_deref(), Some("https://gw.example"));

        // A platform without its own key falls back to the general one.
        let options = AgentStartOptions {
            platform: Some("gemini".into()),
            wire_format: Some(WireFormat::Chat),
            ..AgentStartOptions::default()
        };
        select_route(&mut config, &options);
        assert_eq!(config.provider.as_deref(), Some("openai"));
        assert_eq!(config.api_key.as_deref(), Some("sk-general"));
    }

    #[test]
    fn only_the_asking_modes_leave_the_decision_to_the_user() {
        assert!(AccessMode::Ask.auto_answer().is_none());
        assert!(AccessMode::AutoAcceptEdits.auto_answer().is_none());
        assert_eq!(
            AccessMode::Auto.auto_answer(),
            Some(PermissionChoice::AllowAlways)
        );
        assert_eq!(
            AccessMode::FullAccess.auto_answer(),
            Some(PermissionChoice::AllowOnce)
        );
    }

}
