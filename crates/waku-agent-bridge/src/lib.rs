//! Waku's built-in agent: the vendored engine, driven in process.
//!
//! Every other provider Waku speaks to is a CLI it launches and negotiates
//! with over a protocol. This one is a library call. That difference is the
//! whole point — the engine's entry point takes the conversation by mutable
//! reference and its configuration by value, per turn:
//!
//! ```ignore
//! run_query_loop(client, messages: &mut Vec<Message>, tools, ctx, config, events, cancel, …)
//! ```
//!
//! so the things that are hard to arrange over a wire come for free:
//!
//! | Waku needs | Comes from |
//! | --- | --- |
//! | rewind, branch, resume | Waku owns `Vec<Message>` ([`history`]) |
//! | model and effort switching mid-session | a fresh `QueryConfig` each turn |
//! | real context and cost figures | `TurnComplete { usage }` |
//! | mid-turn steering | the engine's shared command queue |
//! | approvals that persist | the engine's own `PermissionManager` |
//! | background work, stoppable | the engine's task registry ([`background`]) |
//! | sub-agents with a live record | our own `Agent` tool (`subagent`) over the engine's loop |
//! | questions to the user | `AskUserQuestion`'s reply channel |
//! | MCP server tools | [`mcp_tool`], the adapter upstream keeps in its CLI |
//!
//! Routing — which endpoint and key a session uses — is *not* decided here.
//! `sub2api::global_config::native` writes it into the engine's own settings
//! file, the same way every other provider's routing is written into its
//! config, so exactly one place decides whether the signed-in account or a
//! custom endpoint wins.
//!
//! # What lives where
//!
//! This crate owns the engine's lifecycle and emits [`AgentEvent`]s that are
//! still close to the engine's own vocabulary — raw JSON tool payloads, plain
//! text deltas. Turning those into transcript rows needs `waku-core`'s
//! activity normalizer, and `waku-core` is what calls into this crate, so the
//! presentation half lives there instead: see `waku-core/src/driver/native.rs`.
//! Depending on `waku-core` from here would close that cycle.

pub mod background;
// Fork: the in-app browser tools.
mod browser;
mod computer_use;
mod config;
mod events;
mod goal;
pub mod history;
mod images;
mod mcp_tool;
// Fork: auto-memory (`auto_memory`, translated from dsh-auto-memory).
mod memory;
mod oneshot;
mod permission;
mod project_context;
mod runtime;
mod session;
mod subagent;
// Fork addition: AgentTeams for the built-in agent.
mod team;
mod tool_guidance;

pub use background::{BackgroundEntry, BackgroundKind, BackgroundStatus};
pub use config::{
    AccessMode, AgentStartOptions, COMPUTER_USE_TOOLS, ComputerUseWiring,
    ENDPOINT_PLATFORM_PREFIX, MissingApiKey, TurnOptions, UnknownEndpoint, WireFormat,
    endpoint_id, split_model,
};
pub use events::{
    AgentEvent, CompactionPhase, EventSink, PermissionChoice, SubagentEvent, SubagentStatus,
    TokenCounts,
};
pub use goal::{GoalOp, GoalSnapshot, GoalState};
// The refusals whose wording is the fork's rather than the engine's, so the
// driver can recognise them exactly and say them in the user's language.
pub use claurst_tools::{KEEP_PLANNING_DENIAL, MISSING_PLAN_ERROR, PLAN_MODE_DENIAL_SUFFIX};
pub use permission::EXIT_PLAN_MODE_DETAIL;
pub use oneshot::one_shot;
pub use session::AgentSession;

/// Fork (AgentTeams): `/agent-teams` and its profile aliases for the
/// composer, each with the profile it starts. Empty while switched off.
pub fn agent_teams_slash_commands() -> Vec<(String, Option<String>)> {
    team::slash_commands()
}

/// Names of the built-in tools a session loads and a person can switch off.
///
/// Leaves out the tools no session is offered here, and `GoalComplete`,
/// which comes and goes with the goal rather than with a setting.
///
/// The Tools settings page cannot call this — the desktop does not link the
/// engine — so it carries its own copy, `sub2api::agent_settings::BUILTIN_TOOLS`.
/// The test below is what keeps that copy honest.
pub fn tool_names() -> Vec<String> {
    let mut names: Vec<String> = claurst_tools::all_tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .filter(|name| {
            !session::UNAVAILABLE_TOOLS.contains(&name.as_str())
                && name != session::GOAL_COMPLETE_TOOL
        })
        .collect();
    names.push(subagent::AGENT_TOOL_NAME.to_string());
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    /// The Tools page lists `sub2api::agent_settings::BUILTIN_TOOLS`; the
    /// engine offers `tool_names()`. If a vendored engine update adds or
    /// renames a tool, this is what fails.
    #[test]
    fn the_settings_pages_tool_list_matches_the_engine() {
        let engine = super::tool_names();
        let mut page: Vec<String> = BUILTIN_TOOLS_FROM_SETTINGS
            .iter()
            .map(|name| name.to_string())
            .collect();
        page.sort();
        assert_eq!(engine, page);
    }

    // Duplicated here rather than imported: `sub2api` is not a dependency of
    // this crate, and must not become one. Keep in step with
    // `sub2api::agent_settings::BUILTIN_TOOLS`.
    const BUILTIN_TOOLS_FROM_SETTINGS: [&str; 37] = [
        "Agent", "ApplyPatch", "AskUserQuestion", "Bash", "BatchEdit", "Brief",
        "Config", "Edit", "EnterPlanMode", "EnterWorktree", "ExitPlanMode",
        "ExitWorktree", "Glob", "Grep", "LSP", "ListMcpResources", "NotebookEdit",
        "PowerShell", "REPL", "Read", "ReadMcpResource", "Skill", "Sleep",
        "StructuredOutput", "TaskCreate", "TaskGet", "TaskList", "TaskOutput",
        "TaskStop", "TaskUpdate", "TodoWrite", "ToolSearch", "WebFetch",
        "WebSearch", "Write", "mcp__auth", "monitor",
    ];
}
