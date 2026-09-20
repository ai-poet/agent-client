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
mod config;
mod events;
pub mod history;
mod mcp_tool;
mod oneshot;
mod permission;
mod runtime;
mod session;

pub use background::{BackgroundEntry, BackgroundKind, BackgroundStatus};
pub use config::{AccessMode, AgentStartOptions, MissingApiKey, TurnOptions, WireFormat, split_model};
pub use events::{AgentEvent, EventSink, PermissionChoice};
// The two refusals whose wording is the fork's rather than the engine's, so
// the driver can recognise them exactly and say them in the user's language.
pub use claurst_tools::{KEEP_PLANNING_DENIAL, PLAN_MODE_DENIAL_SUFFIX};
pub use permission::EXIT_PLAN_MODE_DETAIL;
pub use oneshot::one_shot;
pub use session::AgentSession;

/// Names of the built-in tools a session loads.
///
/// The Tools settings page cannot call this — the desktop does not link the
/// engine — so it carries its own copy, `sub2api::agent_settings::BUILTIN_TOOLS`.
/// The test below is what keeps that copy honest.
pub fn tool_names() -> Vec<String> {
    let mut names: Vec<String> = claurst_tools::all_tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    names.push(claurst_tools::Tool::name(&claurst_query::AgentTool).to_string());
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
    const BUILTIN_TOOLS_FROM_SETTINGS: [&str; 45] = [
        "Agent", "ApplyPatch", "AskUserQuestion", "Bash", "BatchEdit", "Brief",
        "Config", "CronCreate", "CronDelete", "CronList", "Edit", "EnterPlanMode",
        "EnterWorktree", "ExitPlanMode", "ExitWorktree", "Glob", "GoalComplete",
        "Grep", "LSP", "ListMcpResources", "NotebookEdit", "PowerShell", "REPL",
        "Read", "ReadMcpResource", "RemoteTrigger", "SendMessage", "Skill", "Sleep",
        "StructuredOutput", "TaskCreate", "TaskGet", "TaskList", "TaskOutput",
        "TaskStop", "TaskUpdate", "TeamCreate", "TeamDelete", "TodoWrite",
        "ToolSearch", "WebFetch", "WebSearch", "Write", "mcp__auth", "monitor",
    ];
}
