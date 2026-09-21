//! Turning Waku's session options into the engine's configuration.
//!
//! Two objects come out of this: a [`Config`], which is process-and-project
//! scoped (permissions, MCP roster, hooks, routing), and a [`QueryConfig`],
//! which is *per turn* — model, effort, budgets. That split is what lets a
//! model change take effect on the next turn without restarting anything.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::Context as _;
use claurst_core::config::{Config, McpServerConfig, McpServerOrigin, Settings};
use claurst_core::effort::EffortLevel;
use claurst_core::{PermissionMode, ProviderConfig};
use claurst_query::QueryConfig;

use crate::events::PermissionChoice;

/// Which API a session speaks.
///
/// Not a free choice: each model family is served over one of them, and
/// [`WireFormat::for_model`] is where that is written down. Chat Completions
/// carries only what a user declared on their own endpoint.
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

    /// The one API a model from the managed catalog is reachable over.
    ///
    /// The model's family decides, not the group's platform: a composite
    /// group reports `composite` for every model in it, so the platform is
    /// only a tie-breaker for a name that gives nothing away. A model that
    /// matches neither is one the user declared on their own endpoint, and
    /// this side has no business guessing what it speaks — hence `None`.
    pub fn for_model(platform: Option<&str>, model: &str) -> Option<Self> {
        let model = model.trim().to_ascii_lowercase();
        if model.starts_with("claude") {
            return Some(Self::Messages);
        }
        if model.starts_with("gpt-")
            || model.starts_with("gpt5")
            || model.starts_with("o1")
            || model.starts_with("o3")
            || model.starts_with("o4")
            || model.starts_with("codex")
            || model.starts_with("grok")
        {
            return Some(Self::Responses);
        }
        match normalized_platform(platform).as_deref() {
            Some("anthropic") => Some(Self::Messages),
            Some("openai") | Some("grok") => Some(Self::Responses),
            _ => None,
        }
    }

    /// The format a session speaks.
    ///
    /// A catalog model has exactly one, and it wins over whatever the
    /// session stored — that is what heals a session persisted before this
    /// rule, and what stops a stale tier reaching the wire. Only a model the
    /// catalog does not place falls back to the request, which is the case a
    /// user-declared endpoint model is in.
    pub fn resolve(requested: Option<Self>, platform: Option<&str>, model: &str) -> Self {
        Self::for_model(platform, model)
            .or(requested)
            .unwrap_or(Self::Chat)
    }
}

fn normalized_platform(platform: Option<&str>) -> Option<String> {
    platform
        .map(|platform| platform.trim().to_ascii_lowercase())
        .filter(|platform| !platform.is_empty())
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

    pub(crate) fn permission_mode(self, plan: bool) -> PermissionMode {
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
    /// The language the user reads the app in, named in English
    /// (`Simplified Chinese`). `None` for English, where saying so would add
    /// a line of prompt that changes nothing.
    ///
    /// The engine's own prompt is English and models answer in kind, so an
    /// agent narrating its work would explain itself in English to someone
    /// using the app in another language.
    pub narration_language: Option<String>,
    /// A previous conversation to resume, as written by
    /// [`crate::history::serialize`]. Empty for a fresh session.
    pub history: Vec<u8>,
    /// Desktop control and image generation, when the user has turned
    /// Computer Use on. `None` leaves the session without either.
    pub computer_use: Option<ComputerUseWiring>,
}

/// What the session needs to reach the Computer Use REPL.
///
/// `waku-core` resolves these paths (they live in the app bundle) and hands
/// them over as plain values, because the bridge depends on neither
/// `waku-core` nor `waku-protocol` and cannot call those helpers itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComputerUseWiring {
    /// The `waku_js_repl` binary — the MCP server the model talks to.
    pub repl_server: PathBuf,
    /// The native helper behind it. `None` when it could not be resolved:
    /// image generation and plain JavaScript still work, and only `sky`
    /// fails, with the REPL's own message.
    pub native_helper: Option<PathBuf>,
    /// Where the helper registers its PIDs and writes preview frames.
    /// `None` alongside a missing helper: nothing would write there.
    pub process_directory: Option<PathBuf>,
    /// The bundled SKILL.md, installed so the engine's `Skill` tool can
    /// find it. `None` when the helper is missing — a skill describing
    /// tools that are absent is worse than no skill.
    pub skill_markdown: Option<String>,
}

/// The MCP server name, which prefixes every tool it advertises.
pub const COMPUTER_USE_SERVER: &str = "waku_js_repl";

/// The tools the Computer Use toggle consents to. Turning the feature on
/// *is* the approval; asking again per call would make a ten-step desktop
/// task ten dialogs. Plan mode and any rule the user wrote still refuse
/// them — see `GuiPermissionHandler::decide`.
pub const COMPUTER_USE_TOOLS: [&str; 3] = [
    "waku_js_repl_js",
    "waku_js_repl_js_reset",
    "waku_js_repl_generate_image",
];

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
            narration_language: None,
            history: Vec::new(),
            computer_use: None,
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
    if let Some(wiring) = &options.computer_use {
        install_repl_server(&mut config, wiring, &options.cwd);
    }
    config
}

/// Register the Computer Use REPL as an MCP server for this session only.
///
/// Pushed into the loaded `Config` rather than written to `settings.json`:
/// the toggle is a live preference, and a session that crashed would
/// otherwise leave a server entry behind pointing at a process directory
/// that no longer exists. `McpTool::all` wraps whatever it advertises as
/// ordinary tools named `waku_js_repl_*`.
///
/// A same-named entry the user wrote by hand is replaced, not duplicated —
/// two servers claiming one name would give the model two identical tools.
fn install_repl_server(config: &mut Config, wiring: &ComputerUseWiring, cwd: &PathBuf) {
    config
        .mcp_servers
        .retain(|server| server.name != COMPUTER_USE_SERVER);

    let mut env = HashMap::new();
    if let Some(directory) = &wiring.process_directory {
        env.insert(
            "WAKU_COMPUTER_USE_PROCESS_DIRECTORY".to_owned(),
            directory.display().to_string(),
        );
    }
    // Where generated images land when the caller names no directory.
    //
    // Only this path needs it. Every CLI driver spawns its CLI with
    // `current_dir(cwd)` and the REPL is that CLI's own child, so it
    // inherits the session directory; the engine's MCP manager spawns it
    // from wherever the daemon happens to be running instead.
    env.insert("WAKU_SESSION_CWD".to_owned(), cwd.display().to_string());
    if let Some(helper) = &wiring.native_helper {
        env.insert(
            "WAKU_COMPUTER_USE_SERVER".to_owned(),
            helper.display().to_string(),
        );
    }

    config.mcp_servers.push(McpServerConfig {
        name: COMPUTER_USE_SERVER.to_owned(),
        command: Some(wiring.repl_server.display().to_string()),
        args: Vec::new(),
        env,
        url: None,
        server_type: "stdio".to_owned(),
        origin: McpServerOrigin::User,
    });
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

/// How much of the conversation may be tool output before the engine starts
/// shedding the oldest of it.
///
/// The engine's own default is 50 000 characters, which is half of what a
/// single Bash call is allowed to return — and its shedding pass replaces a
/// whole result with a one-line notice rather than trimming it, oldest
/// first, checking whether the result covers the debt only *after* wiping
/// it. So one large command wiped its own output before the model ever read
/// it, while the transcript still showed it: the picker and the model were
/// looking at different conversations.
///
/// Raising it past several times the per-call cap keeps that from firing on
/// ordinary work. Running out of context is still handled, just by the
/// mechanism meant for it: auto-compact summarises at 90% of the window
/// instead of blanking individual results.
const TOOL_RESULT_BUDGET: usize = 600_000;

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

/// The models.dev snapshot the engine ships, parsed once.
///
/// `ModelRegistry::new` parses a 1.8 MB bundle compiled into the binary, so
/// it is built once for the process rather than per session. It carries the
/// real context window of every model it knows, which is the only reason the
/// usage meter can show anything better than a guess.
pub fn model_registry() -> &'static Arc<claurst_api::ModelRegistry> {
    static REGISTRY: OnceLock<Arc<claurst_api::ModelRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(claurst_api::ModelRegistry::new()))
}

/// Smallest registry window treated as real.
///
/// models.dev omits a limit for some models and the registry stores a 4096
/// placeholder instead; sizing a meter against that would be worse than
/// admitting we do not know. Mirrors the engine's own private constant in
/// `claurst_query::compact`.
const MIN_PLAUSIBLE_REGISTRY_WINDOW: u64 = 8192;

/// The model's real context window, or `None` when nothing here knows it.
///
/// Deliberately not `claurst_query::resolve_context_window`: that one returns
/// a plain `u64` because it always falls back to a Claude-only heuristic that
/// answers 100k for everything it does not recognise — which is every model
/// this app actually offers, and precisely the number the usage meter was
/// wrong about. A meter that says nothing beats a meter that says 100k.
///
/// The engine's auto-compact keeps using the heuristic, and should: it needs
/// *a* threshold to act on, where the meter needs the truth or silence.
pub fn registry_context_window(model: &str, fallback_provider: &str) -> Option<u64> {
    let provider = registry_provider_for(model, fallback_provider);
    model_registry()
        .get(&provider, model)
        .map(|entry| entry.info.context_window as u64)
        .filter(|window| *window >= MIN_PLAUSIBLE_REGISTRY_WINDOW)
}

/// The models.dev provider that owns `model`, which is not the provider this
/// app routes through.
///
/// `WireFormat::engine_provider` answers `anthropic` / `codex` / `openai`,
/// and the gateway's platform adds `grok` / `composite` / `default`. The
/// registry is keyed by models.dev ids, where Grok lives under `xai` and
/// `codex` does not exist at all — so passing either one straight through
/// silently misses and falls back to a guess. The registry's own family
/// table knows the mapping; only fall back to the caller's provider when it
/// does not recognise the name.
pub fn registry_provider_for(model: &str, fallback: &str) -> String {
    model_registry()
        .find_provider_for_model(model)
        .map(|provider| provider.to_string())
        .unwrap_or_else(|| fallback.to_owned())
}

/// Build the per-turn config from the project config plus this turn's options.
pub fn build_query_config(config: &Config, options: &AgentStartOptions) -> QueryConfig {
    let mut query = QueryConfig::from_config_with_registry(config, model_registry());
    // `from_config_with_registry` consults the registry to resolve the model
    // name but does not keep it, so hand it over as well: that is what sizes
    // the engine's own auto-compact against the model's real window instead
    // of the Claude-only heuristic's 100k.
    query.model_registry = Some(model_registry().clone());
    query.working_directory = Some(options.cwd.display().to_string());
    // `QueryConfig::from_config` copies neither of these, so the Agent
    // settings page wrote house rules into a file nothing read. The engine
    // has always known what to do with them once they arrive
    // (`claurst_query::runner::prompt`).
    query.system_prompt = config.custom_system_prompt.clone();
    query.append_system_prompt = session_rules(options, config.append_system_prompt.as_deref());
    query.tool_result_budget = TOOL_RESULT_BUDGET;
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

/// The appended system prompt: the language rule this product adds, then
/// whatever the user wrote on the Agent settings page.
///
/// The user's text comes last so it can overrule the rule above it — a house
/// rule that says "always answer in English" should win. The instruction
/// itself is written in English even when it names another language, because
/// the rest of the prompt around it is.
fn narration_rule(options: &AgentStartOptions) -> Option<String> {
    options.narration_language.as_deref().map(|language| {
        format!(
            "Write everything the user reads in {language}: your explanations, \
             your summaries of what you did, and the questions you ask. Code, \
             file paths, commands and identifiers stay as they are."
        )
    })
}

/// What plan mode is, and how to hand a plan back.
///
/// Fork: nothing in the engine's prompt mentions plan mode at all, so a model
/// that has not been trained to call `ExitPlanMode` (most of the non-Claude
/// ones) writes its plan as prose and ends the turn - and the "finished
/// planning" dialog, which is keyed on that tool, never appears. The model
/// has to be told the tool exists, what its `summary` is for, and that the
/// user reads the plan before anything runs.
fn plan_mode_rule(options: &AgentStartOptions) -> Option<String> {
    options.plan_mode.then(|| {
        "You are in plan mode: nothing you propose is applied yet. Read, search \
         and reason freely, then write the plan out for the user. When the plan \
         is ready, call ExitPlanMode with a short `summary` of it - that hands \
         the plan to the user, who will approve it or send you back to keep \
         planning. Do not call ExitPlanMode before the plan is written, and do \
         not try to edit files or run commands while planning."
            .to_owned()
    })
}

/// That the desktop can be driven, and where the instructions for it are.
///
/// Fork: the REPL's tools arrive over MCP with names and one-line
/// descriptions, which is not enough to use them — the how is an 19 KB
/// document, and reading it into every request would cost more than it is
/// worth on the turns that never touch the desktop. Naming the skill lets
/// the model fetch it on the turn it needs it.
fn computer_use_rule(options: &AgentStartOptions) -> Option<String> {
    options.computer_use.as_ref().map(|wiring| {
        let mut rule = String::from(
            "This computer can be driven directly. Before operating a desktop              application, call Skill with skill=\"waku-computer-use\" and follow              what it says; the tools it describes are the waku_js_repl ones.",
        );
        if wiring.native_helper.is_none() {
            rule.push_str(
                " Desktop control is unavailable in this session because its                  helper could not be started, so do not attempt it.",
            );
        }
        rule.push_str(
            " To make a picture, call waku_js_repl_generate_image rather than              looking for an external service.",
        );
        rule
    })
}

/// Everything appended to the system prompt for this session, in order:
/// plan mode, the narration language, then whatever the user wrote on the
/// Agent settings page. The user's text comes last so it can overrule the
/// rules above it.
fn session_rules(options: &AgentStartOptions, house_rules: Option<&str>) -> Option<String> {
    let parts: Vec<String> = [
        plan_mode_rule(options),
        computer_use_rule(options),
        narration_rule(options),
        house_rules.map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
mod tests {

    /// The bundled snapshot is a compile-time file that stops at the -4-5
    /// generation, so it is a fallback and not the answer. Assert against a
    /// model it actually carries — asserting on `claude-sonnet-5` here is
    /// what hid the staleness in the first place.
    #[test]
    fn the_bundled_registry_answers_for_the_models_it_carries() {
        let window = registry_context_window("claude-sonnet-4-5", "anthropic");
        assert_eq!(window, Some(200_000));
    }

    /// The whole point of returning an `Option`: the models this app actually
    /// offers are newer than the snapshot, and a wrong 100k is worse than no
    /// percentage at all. The gateway is what fills these in.
    #[test]
    fn a_model_no_source_knows_reports_nothing_rather_than_a_guess() {
        for (model, provider) in [
            ("claude-sonnet-5", "anthropic"),
            ("grok-4.6", "grok"),
            ("my-own-model", "openai"),
        ] {
            assert_eq!(
                registry_context_window(model, provider),
                None,
                "{model} should report an unknown window, not a guess"
            );
            // The heuristic this replaced would have answered 100k for every
            // one of them.
            assert_eq!(claurst_query::context_window_for_model(model), 100_000);
        }
    }

    /// The provider this app routes through is not the one models.dev keys
    /// by: Grok is `xai` there, and `codex` does not exist at all. Passing
    /// either straight through misses and silently falls back to a guess.
    #[test]
    fn the_routing_provider_is_translated_to_the_registrys_own() {
        assert_eq!(registry_provider_for("grok-4.6", "grok"), "xai");
        assert_eq!(registry_provider_for("gpt-5.6-sol", "codex"), "openai");
        assert_eq!(registry_provider_for("claude-sonnet-5", "anthropic"), "anthropic");
        // A model the registry has never heard of — one the user declared on
        // their own endpoint — keeps the caller's provider rather than
        // inventing one.
        assert_eq!(registry_provider_for("my-own-model", "openai"), "openai");
    }

    /// A placeholder window is not knowledge. models.dev omits the limit for
    /// some models and the registry stores 4096 instead; sizing a meter
    /// against that would be worse than saying nothing.
    #[test]
    fn a_placeholder_window_counts_as_unknown() {
        assert!(MIN_PLAUSIBLE_REGISTRY_WINDOW > 4096);
    }
    use super::*;

    fn wired(helper: bool) -> AgentStartOptions {
        AgentStartOptions {
            computer_use: Some(ComputerUseWiring {
                repl_server: PathBuf::from("/opt/waku_js_repl"),
                native_helper: helper.then(|| PathBuf::from("/opt/helper")),
                process_directory: helper.then(|| PathBuf::from("/tmp/cu")),
                skill_markdown: Some("# skill".into()),
            }),
            cwd: PathBuf::from("/work"),
            ..AgentStartOptions::default()
        }
    }

    /// The server is registered for this session only, and a hand-written
    /// entry of the same name is replaced rather than duplicated — two
    /// servers claiming one name would give the model the tool twice.
    #[test]
    fn the_repl_is_registered_once_and_replaces_a_same_named_entry() {
        let mut config = Config::default();
        config.mcp_servers.push(McpServerConfig {
            name: COMPUTER_USE_SERVER.to_owned(),
            command: Some("/somewhere/else".into()),
            args: Vec::new(),
            env: HashMap::new(),
            url: None,
            server_type: "stdio".to_owned(),
            origin: McpServerOrigin::User,
        });
        let options = wired(true);
        install_repl_server(&mut config, options.computer_use.as_ref().unwrap(), &options.cwd);

        let servers: Vec<&McpServerConfig> = config
            .mcp_servers
            .iter()
            .filter(|server| server.name == COMPUTER_USE_SERVER)
            .collect();
        assert_eq!(servers.len(), 1);
        let server = servers[0];
        assert_eq!(server.command.as_deref(), Some("/opt/waku_js_repl"));
        assert_eq!(server.server_type, "stdio");
        assert_eq!(
            server.env.get("WAKU_COMPUTER_USE_SERVER").map(String::as_str),
            Some("/opt/helper")
        );
        assert_eq!(server.env.get("WAKU_SESSION_CWD").map(String::as_str), Some("/work"));
        assert!(server.env.contains_key("WAKU_COMPUTER_USE_PROCESS_DIRECTORY"));
    }

    /// Without the helper the REPL still runs — image generation and plain
    /// JavaScript work — so the server is registered without the variable
    /// that would promise desktop control.
    #[test]
    fn a_missing_helper_registers_the_server_without_promising_desktop_control() {
        let mut config = Config::default();
        let options = wired(false);
        install_repl_server(&mut config, options.computer_use.as_ref().unwrap(), &options.cwd);
        let server = &config.mcp_servers[0];
        assert!(!server.env.contains_key("WAKU_COMPUTER_USE_SERVER"));
        // Nothing would write there without a helper.
        assert!(!server.env.contains_key("WAKU_COMPUTER_USE_PROCESS_DIRECTORY"));
        // The REPL itself still runs, so image generation survives.
        assert_eq!(server.command.as_deref(), Some("/opt/waku_js_repl"));

        let rule = computer_use_rule(&options).expect("a rule");
        assert!(rule.contains("unavailable"), "{rule}");
    }

    /// Nothing registers the server when the toggle is off.
    #[test]
    fn no_server_and_no_rule_without_the_toggle() {
        let options = AgentStartOptions::default();
        let config = build_config_from(Settings::default(), &options);
        assert!(config.mcp_servers.is_empty());
        assert_eq!(computer_use_rule(&options), None);
    }

    /// The rule names the skill, because the tool descriptions alone do not
    /// say how to use them.
    #[test]
    fn the_rule_points_at_the_skill_and_the_image_tool() {
        let rule = computer_use_rule(&wired(true)).expect("a rule");
        assert!(rule.contains("waku-computer-use"), "{rule}");
        assert!(rule.contains("waku_js_repl_generate_image"), "{rule}");
        assert!(!rule.contains("unavailable"), "{rule}");
    }

    /// Nothing in the engine's prompt mentions plan mode, so the bridge has
    /// to. The rule is present exactly when the session is planning, and it
    /// names the tool the model must call to hand the plan back.
    #[test]
    fn plan_mode_is_explained_only_while_planning() {
        let planning = AgentStartOptions { plan_mode: true, ..AgentStartOptions::default() };
        let rule = session_rules(&planning, None).expect("a rule while planning");
        assert!(rule.contains("ExitPlanMode"), "{rule}");
        assert!(rule.contains("summary"), "{rule}");

        let building = AgentStartOptions { plan_mode: false, ..AgentStartOptions::default() };
        assert_eq!(session_rules(&building, None), None);
    }

    /// Order matters: plan mode first, then the language, then the user's
    /// own text last so it can overrule both.
    #[test]
    fn session_rules_keep_the_users_text_last() {
        let options = AgentStartOptions {
            plan_mode: true,
            narration_language: Some("Simplified Chinese".into()),
            ..AgentStartOptions::default()
        };
        let combined = session_rules(&options, Some("Prefer tabs.")).expect("all three");
        let plan = combined.find("plan mode").expect("plan rule");
        let language = combined.find("Simplified Chinese").expect("language rule");
        let house = combined.find("Prefer tabs.").expect("house rule");
        assert!(plan < language && language < house, "{combined}");
        assert!(combined.ends_with("Prefer tabs."));
    }

    #[test]
    fn the_language_rule_comes_first_so_house_rules_can_overrule_it() {
        let options = AgentStartOptions {
            narration_language: Some("Simplified Chinese".into()),
            ..AgentStartOptions::default()
        };
        let combined = session_rules(&options, Some("  Prefer tabs.  ")).expect("both");
        assert!(combined.starts_with("Write everything the user reads in Simplified Chinese"));
        assert!(combined.trim_end().ends_with("Prefer tabs."));

        // Either one alone is the whole thing.
        assert_eq!(session_rules(&options, None), Some(combined[..combined.find("\n\n").unwrap()].to_owned()));
        let english = AgentStartOptions::default();
        assert_eq!(session_rules(&english, Some("Prefer tabs.")), Some("Prefer tabs.".to_owned()));
        assert_eq!(session_rules(&english, None), None);
        // Blank house rules are not house rules.
        assert_eq!(session_rules(&english, Some("   ")), None);
    }

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
    fn a_models_family_decides_its_api() {
        for model in ["claude-sonnet-5", "claude-opus-4-6"] {
            assert_eq!(
                WireFormat::for_model(Some("anthropic"), model),
                Some(WireFormat::Messages),
                "{model}"
            );
        }
        for model in ["gpt-5.6-sol", "o3-mini", "o4-mini", "codex-mini", "grok-4.6"] {
            assert_eq!(
                WireFormat::for_model(Some("openai"), model),
                Some(WireFormat::Responses),
                "{model}"
            );
        }
        // The family wins over the group's platform, which is what makes a
        // composite group — every model in it reports `composite` — work.
        assert_eq!(
            WireFormat::for_model(Some("composite"), "grok-4.6"),
            Some(WireFormat::Responses)
        );
        assert_eq!(
            WireFormat::for_model(Some("composite"), "claude-sonnet-5"),
            Some(WireFormat::Messages)
        );
        // A name that says nothing falls back to the platform.
        assert_eq!(
            WireFormat::for_model(Some("anthropic"), "some-internal-alias"),
            Some(WireFormat::Messages)
        );
        // And a model the catalog does not place is the user's own.
        assert_eq!(WireFormat::for_model(Some("gemini"), "gemini-3-pro"), None);
        assert_eq!(WireFormat::for_model(None, "my-local-model"), None);
    }

    #[test]
    fn a_catalog_models_family_outranks_whatever_the_session_stored() {
        // A session persisted before the rule existed heals on the next turn
        // rather than failing on the wire.
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Messages), Some("grok"), "grok-4.6"),
            WireFormat::Responses
        );
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Chat), Some("openai"), "gpt-5.6-sol"),
            WireFormat::Responses
        );
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Responses), Some("anthropic"), "claude-sonnet-5"),
            WireFormat::Messages
        );
    }

    #[test]
    fn a_user_declared_model_keeps_the_format_it_was_given() {
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Chat), None, "my-local-model"),
            WireFormat::Chat
        );
        assert_eq!(
            WireFormat::resolve(Some(WireFormat::Messages), None, "my-local-model"),
            WireFormat::Messages
        );
        // Nothing asked for: Chat Completions is where a declared model goes.
        assert_eq!(
            WireFormat::resolve(None, None, "my-local-model"),
            WireFormat::Chat
        );
    }

    #[test]
    fn a_stale_session_option_is_clamped_before_it_reaches_the_wire() {
        let options = AgentStartOptions {
            platform: Some("grok".into()),
            model: Some("grok-4.6".into()),
            wire_format: Some(WireFormat::Messages),
            ..AgentStartOptions::default()
        };
        assert_eq!(options.wire_format(), WireFormat::Responses);
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
