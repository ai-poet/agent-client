//! Turning Waku's session options into the engine's configuration.
//!
//! Two objects come out of this: a [`Config`], which is process-and-project
//! scoped (permissions, MCP roster, hooks, routing), and a [`QueryConfig`],
//! which is *per turn* — model, effort, budgets. That split is what lets a
//! model change take effect on the next turn without restarting anything.

use std::path::PathBuf;

use claurst_core::config::{Config, Settings};
use claurst_core::effort::EffortLevel;
use claurst_core::PermissionMode;
use claurst_query::QueryConfig;

use crate::events::PermissionChoice;

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
    pub model: Option<String>,
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
            reasoning_effort: None,
            history: Vec::new(),
        }
    }
}

/// The per-turn options that can change without a restart.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TurnOptions {
    pub access_mode: Option<AccessMode>,
    pub plan_mode: Option<bool>,
    pub model: Option<String>,
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

    config
}

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
