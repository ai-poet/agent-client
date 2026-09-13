//! Turning Waku's session options into the engine's configuration.
//!
//! Two objects come out of this: a [`Config`], which is process-and-project
//! scoped (permissions, MCP roster, hooks, routing), and a [`QueryConfig`],
//! which is *per turn* — model, effort, budgets. That split is what lets a
//! model change take effect on the next turn without restarting anything.

use std::path::PathBuf;

use anyhow::Context as _;
use claurst_core::config::{Config, Settings};
use claurst_core::effort::EffortLevel;
use claurst_core::{PermissionMode, ProviderConfig};
use claurst_query::QueryConfig;

use crate::events::PermissionChoice;

/// Which API a session speaks.
///
/// Not a free choice: the gateway serves all three endpoints but routes each
/// by the key's group platform, and what waits on the other side differs.
/// [`WireFormat::supported_for`] is where that is written down.
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

    /// The route a platform is served over natively.
    ///
    /// The gateway splits platforms in two: the OpenAI-compatible ones reach
    /// their upstream through the Responses API, everything else through
    /// Anthropic Messages. Picking the native one means no translation in
    /// the middle, which is both faster and the path least likely to differ
    /// from what the upstream actually supports.
    pub fn default_for_platform(platform: Option<&str>) -> Self {
        if openai_compatible_platform(platform) {
            Self::Responses
        } else {
            Self::Messages
        }
    }

    /// Whether this format can carry `model` on `platform` at all.
    ///
    /// Two things rule a combination out, and both are facts about code that
    /// exists rather than guesses:
    ///
    /// - The gateway has no Responses translator for Gemini groups. A
    ///   Responses request from one is forwarded as Anthropic Messages to a
    ///   Gemini upstream, which is not a thing that can work.
    /// - The engine's own Chat Completions client refuses `gpt-5*`, `o3*`
    ///   and `o4*`, which is most of what an OpenAI group offers.
    ///
    /// Messages on an OpenAI group is deliberately *not* excluded: it
    /// depends on a per-group setting this side cannot see, so it stays
    /// offered and answers with the gateway's own 403 when it is off.
    pub fn supported_for(self, platform: Option<&str>, model: &str) -> bool {
        let platform = normalized_platform(platform);
        match self {
            Self::Responses => platform.as_deref() != Some("gemini"),
            Self::Chat => !model_needs_responses_api(model),
            Self::Messages => true,
        }
    }

    /// The formats that can carry `model` on `platform`, in listing order.
    pub fn available_for(platform: Option<&str>, model: &str) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|format| format.supported_for(platform, model))
            .collect()
    }

    /// The format a session should speak: the one asked for when it can
    /// carry this model, otherwise the platform's native one, otherwise
    /// whatever is left. Never returns a combination that cannot work.
    pub fn resolve(
        requested: Option<Self>,
        platform: Option<&str>,
        model: &str,
    ) -> Self {
        if let Some(format) = requested
            && format.supported_for(platform, model)
        {
            return format;
        }
        let native = Self::default_for_platform(platform);
        if native.supported_for(platform, model) {
            return native;
        }
        Self::ALL
            .into_iter()
            .find(|format| format.supported_for(platform, model))
            // Messages is unconditional above, so this is unreachable; it
            // costs one line to not have to prove that at every call site.
            .unwrap_or(Self::Messages)
    }
}

/// Platforms the gateway forwards through its OpenAI stack rather than its
/// Anthropic one. Mirrors `isOpenAIResponsesCompatibleGatewayPlatform` in the
/// service's `routes/gateway.go`; a platform missing here is served as
/// Anthropic, which is the safe way to be wrong about a new one.
fn openai_compatible_platform(platform: Option<&str>) -> bool {
    matches!(
        normalized_platform(platform).as_deref(),
        Some(
            "openai"
                | "grok"
                | "kimi"
                | "zhipu"
                | "deepseek"
                | "minimax"
                | "opencode_go"
                | "opencodego"
        )
    )
}

fn normalized_platform(platform: Option<&str>) -> Option<String> {
    platform
        .map(|platform| platform.trim().to_ascii_lowercase())
        .filter(|platform| !platform.is_empty())
}

/// Models the engine's Chat Completions client refuses outright, because
/// OpenAI serves them over Responses. Mirrors `OpenAiProvider::use_responses_api`.
fn model_needs_responses_api(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4")
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
    /// The format this session actually speaks.
    ///
    /// Clamped, not merely defaulted: a session persisted before a rule
    /// existed, or restored from a client that lets the user pick freely,
    /// must not be able to hold a combination the gateway cannot serve. It
    /// heals here rather than failing on the wire.
    pub fn wire_format(&self) -> WireFormat {
        WireFormat::resolve(
            self.wire_format,
            self.platform.as_deref(),
            self.model.as_deref().unwrap_or_default(),
        )
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
pub fn build_config(options: &AgentStartOptions) -> anyhow::Result<Config> {
    Ok(build_config_from(load_settings()?, options))
}

/// The engine's global settings.
///
/// A file that does not exist is the engine's own defaults, exactly as the
/// engine treats it. A file that exists but cannot be parsed is an error
/// carrying its path: continuing with defaults would run the session with no
/// key, no MCP roster and no permission rules, and fail later with a message
/// that names none of that.
pub(crate) fn load_settings() -> anyhow::Result<Settings> {
    let path = Settings::global_settings_path();
    if !path.exists() {
        return Ok(Settings::default());
    }
    Settings::load_sync().with_context(|| {
        format!(
            "the built-in agent's settings file {} could not be read",
            path.display()
        )
    })
}

/// [`build_config`] on settings the caller already loaded — the seam the
/// tests drive with a settings document rather than a file.
pub(crate) fn build_config_from(settings: Settings, options: &AgentStartOptions) -> Config {
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

/// A route with nothing to authenticate it.
///
/// Raised before any request is made, so the user reads which route lacks a
/// key and where a key would go — rather than the engine's own "run `claurst
/// auth login`" advice, which names a CLI this product does not ship.
#[derive(Clone, Debug)]
pub struct MissingApiKey {
    /// The engine provider the wire format selected (`anthropic`, `codex`,
    /// `openai`).
    pub provider: String,
    /// The model's platform, when the session named one.
    pub platform: Option<String>,
    pub settings_path: PathBuf,
}

impl std::fmt::Display for MissingApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no API key for the built-in agent's `{}` route",
            self.provider
        )?;
        if let Some(platform) = &self.platform {
            write!(f, " (platform `{platform}`)")?;
        }
        write!(
            f,
            ": sign in, or set provider_configs.{}.api_key in {}",
            self.provider,
            self.settings_path.display()
        )
    }
}

impl std::error::Error for MissingApiKey {}

/// The gate a route has to pass before a client is built for it.
///
/// `resolved` is the key the caller resolved for the selected provider
/// through the engine's own precedence (config, provider entry, environment,
/// stored OAuth tokens). Pure, so it is tested without touching any of those.
pub(crate) fn missing_route_key(
    config: &Config,
    platform: Option<&str>,
    resolved: Option<&str>,
) -> Option<MissingApiKey> {
    if resolved.is_some_and(|key| !key.trim().is_empty()) {
        return None;
    }
    Some(MissingApiKey {
        provider: config.selected_provider_id().to_owned(),
        platform: platform.map(str::to_owned),
        settings_path: Settings::global_settings_path(),
    })
}

/// Build the per-turn config from the project config plus this turn's options.
pub fn build_query_config(config: &Config, options: &AgentStartOptions) -> QueryConfig {
    let mut query = QueryConfig::from_config(config);
    query.working_directory = Some(options.cwd.display().to_string());
    // `QueryConfig::from_config` copies neither of these, so the Agent
    // settings page wrote house rules into a file nothing read. The engine
    // has always known what to do with them once they arrive
    // (`claurst_query::runner::prompt`).
    query.system_prompt = config.custom_system_prompt.clone();
    query.append_system_prompt = config.append_system_prompt.clone();
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

    /// Byte-for-byte what `sub2api::global_config::native::take_over` writes
    /// into an empty config directory for a signed-in account. Duplicated
    /// here because `sub2api` is not — and must not become — a dependency of
    /// this crate; `sub2api`'s own test asserts its writer still produces
    /// this document. This is the JSON → `Settings` hop that used to fail.
    const FRESH_TAKEOVER_SETTINGS: &str = r#"{
  "config": {
    "api_key": "sk-claude",
    "provider_configs": {
      "anthropic": {
        "api_key": "sk-claude",
        "api_base": "https://gateway.example.org",
        "enabled": true,
        "options": {
          "gateway_keys": {
            "anthropic": "sk-claude",
            "default": "sk-general",
            "openai": "sk-codex"
          }
        }
      },
      "codex": {
        "api_key": "sk-claude",
        "api_base": "https://gateway.example.org",
        "enabled": true
      },
      "openai": {
        "api_key": "sk-claude",
        "api_base": "https://gateway.example.org",
        "enabled": true
      }
    }
  }
}"#;

    fn fresh_settings() -> Settings {
        serde_json::from_str(FRESH_TAKEOVER_SETTINGS)
            .expect("the routing writer's document must load as engine settings")
    }

    #[test]
    fn a_fresh_takeover_file_parses_and_routes_every_format() {
        for (platform, format, provider, key) in [
            ("anthropic", WireFormat::Messages, "anthropic", "sk-claude"),
            ("openai", WireFormat::Responses, "codex", "sk-codex"),
            ("gemini", WireFormat::Chat, "openai", "sk-general"),
        ] {
            let options = AgentStartOptions {
                platform: Some(platform.into()),
                wire_format: Some(format),
                ..AgentStartOptions::default()
            };
            let config = build_config_from(fresh_settings(), &options);
            assert_eq!(config.provider.as_deref(), Some(provider), "{platform}");
            assert_eq!(config.api_key.as_deref(), Some(key), "{platform}");
            let entry = config.provider_configs.get(provider).unwrap();
            assert_eq!(
                entry.api_base.as_deref(),
                Some("https://gateway.example.org"),
                "{platform}"
            );
            assert_eq!(
                config.resolve_provider_api_key(config.selected_provider_id()).as_deref(),
                Some(key),
                "{platform}"
            );
            assert!(missing_route_key(&config, Some(platform), Some(key)).is_none());
        }
    }

    #[test]
    fn a_partial_config_block_keeps_the_engines_defaults() {
        let config = build_config_from(fresh_settings(), &AgentStartOptions::default());
        // Never written by the routing writer; must come from the engine's
        // defaults rather than fail the document.
        let defaults = Config::default();
        assert_eq!(config.auto_compact, defaults.auto_compact);
        assert_eq!(config.compact_threshold, defaults.compact_threshold);
        assert_eq!(config.verbose, defaults.verbose);
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn missing_route_key_names_the_provider_and_the_platform() {
        let mut config = Config::default();
        config.provider = Some("codex".into());
        let missing = missing_route_key(&config, Some("openai"), None).expect("no key");
        assert_eq!(missing.provider, "codex");
        assert_eq!(missing.platform.as_deref(), Some("openai"));
        let text = missing.to_string();
        assert!(text.contains("codex"), "{text}");
        assert!(text.contains("openai"), "{text}");
        assert!(missing_route_key(&config, None, Some("   ")).is_some());
        assert!(missing_route_key(&config, None, Some("sk")).is_none());
    }

    /// Reads the settings file actually installed on this machine and prints
    /// what each format would route with. A manual aid for the verification
    /// steps, not part of the suite.
    #[test]
    #[ignore]
    fn parses_the_installed_settings_file() {
        let settings = load_settings().expect("installed settings");
        for (platform, format) in [
            ("anthropic", WireFormat::Messages),
            ("openai", WireFormat::Responses),
            ("openai", WireFormat::Chat),
        ] {
            let options = AgentStartOptions {
                platform: Some(platform.into()),
                wire_format: Some(format),
                ..AgentStartOptions::default()
            };
            let config = build_config_from(settings.clone(), &options);
            let key = config.resolve_provider_api_key(config.selected_provider_id());
            println!(
                "{platform} / {:?} -> provider {:?}, base {:?}, key resolved: {}",
                format,
                config.provider,
                config.resolve_anthropic_api_base(),
                key.is_some_and(|key| !key.is_empty())
            );
        }
    }

    #[test]
    fn a_platform_is_served_over_the_route_its_upstream_speaks() {
        for platform in [
            "openai",
            "grok",
            "kimi",
            "zhipu",
            "deepseek",
            "minimax",
            "opencode_go",
        ] {
            assert_eq!(
                WireFormat::default_for_platform(Some(platform)),
                WireFormat::Responses,
                "{platform}"
            );
        }
        for platform in ["anthropic", "gemini", "antigravity", "composite"] {
            assert_eq!(
                WireFormat::default_for_platform(Some(platform)),
                WireFormat::Messages,
                "{platform}"
            );
        }
        // Unknown platforms are served as Anthropic, the conservative half.
        assert_eq!(
            WireFormat::default_for_platform(Some("something-new")),
            WireFormat::Messages
        );
        assert_eq!(WireFormat::default_for_platform(None), WireFormat::Messages);
    }

    #[test]
    fn gemini_has_no_responses_translator_and_gpt_5_has_no_chat_client() {
        assert!(!WireFormat::Responses.supported_for(Some("gemini"), "gemini-3-pro"));
        assert!(WireFormat::Messages.supported_for(Some("gemini"), "gemini-3-pro"));
        assert!(WireFormat::Chat.supported_for(Some("gemini"), "gemini-3-pro"));

        for model in ["gpt-5.6-sol", "o3-mini", "o4-mini"] {
            assert!(!WireFormat::Chat.supported_for(Some("openai"), model), "{model}");
            assert!(WireFormat::Responses.supported_for(Some("openai"), model), "{model}");
        }
        // Older OpenAI models still have a Chat client.
        assert!(WireFormat::Chat.supported_for(Some("openai"), "gpt-4o"));

        // Messages is never ruled out here: whether an OpenAI group accepts it
        // is a per-group setting this side cannot read.
        assert!(WireFormat::Messages.supported_for(Some("openai"), "gpt-5.6-sol"));
    }

    #[test]
    fn a_route_that_cannot_work_is_replaced_rather_than_sent() {
        // A session that had picked Responses before gemini models existed.
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Responses), Some("gemini"), "gemini-3-pro"),
            WireFormat::Messages
        );
        // Chat on a model whose only route is Responses.
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Chat), Some("openai"), "gpt-5.6-sol"),
            WireFormat::Responses
        );
        // A workable choice is always kept, native or not.
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Messages), Some("grok"), "grok-4.6"),
            WireFormat::Messages
        );
        // Nothing chosen falls to the platform's own route.
        assert_eq!(
            WireFormat::resolve(None, Some("grok"), "grok-4.6"),
            WireFormat::Responses
        );
    }

    #[test]
    fn the_offered_formats_are_the_ones_that_can_carry_the_model() {
        assert_eq!(
            WireFormat::available_for(Some("gemini"), "gemini-3-pro"),
            vec![WireFormat::Messages, WireFormat::Chat]
        );
        assert_eq!(
            WireFormat::available_for(Some("openai"), "gpt-5.6-sol"),
            vec![WireFormat::Messages, WireFormat::Responses]
        );
        assert_eq!(
            WireFormat::available_for(Some("anthropic"), "claude-sonnet-5"),
            vec![WireFormat::Messages, WireFormat::Responses, WireFormat::Chat]
        );
    }

    #[test]
    fn a_stale_session_option_is_clamped_before_it_reaches_the_wire() {
        let options = AgentStartOptions {
            platform: Some("gemini".into()),
            model: Some("gemini-3-pro".into()),
            wire_format: Some(WireFormat::Responses),
            ..AgentStartOptions::default()
        };
        assert_eq!(options.wire_format(), WireFormat::Messages);
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
