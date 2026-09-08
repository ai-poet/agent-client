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

mod config;
mod events;
pub mod history;
mod permission;
mod runtime;
mod session;

pub use config::{AccessMode, AgentStartOptions, TurnOptions};
pub use events::{AgentEvent, EventSink, PermissionChoice};
pub use session::AgentSession;

/// Names of the tools a Native session loads, for the Tools settings page.
///
/// Built from the same list the session builds, so the page can never drift
/// from what the model is actually offered.
pub fn tool_names() -> Vec<String> {
    let mut names: Vec<String> = claurst_tools::all_tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    names.push(claurst_tools::Tool::name(&claurst_query::AgentTool).to_string());
    names
}
