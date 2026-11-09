#![recursion_limit = "256"]

//! Waku's daemon-side core.
//!
//! Provider, database, filesystem, and Git implementations live here, behind
//! the transport-neutral contract in `client-protocol`. Client applications
//! intentionally depend on `client` instead of this crate.

rust_i18n::i18n!("../../locales", fallback = "en");

macro_rules! tr {
    ($key:expr) => {
        crate::i18n::translate($key)
    };
    ($key:expr, $($args:tt)*) => {
        rust_i18n::t!($key, $($args)*).into_owned()
    };
}

pub mod activity_overview;
pub mod attachments;
pub mod blob_store;
pub mod checkpoint;
pub mod command_env;
pub mod composer_complete;
pub mod computer_use;
pub mod daemon;
pub mod driver;
mod frontmatter;
pub mod git_branch;
pub mod git_commit;
pub mod i18n;
pub mod identity;
pub mod model;
pub mod model_catalog;
pub mod persistence;
pub mod projectless;
pub mod settings;
pub mod skills;
pub mod terminal;
pub mod theme;
pub mod usage;
pub mod usage_history;
pub mod workspace;
pub mod worktree;

mod fs_ext;
mod protocol;
mod server;

pub use protocol::{
    APP_EXECUTABLE_ENV, ClientMessage, Command, DAEMON_ADDRESS_ENV, DAEMON_TOKEN_ENV, DaemonReady,
    PROTOCOL_VERSION, ReplayCursor, Request, ResponseOutcome, ResponsePayload, RpcError,
    SequencedEvent, ServerMessage, WireComputerToolRequest, WireDriverEvent,
    WireDriverStartOptions, WireSessionOptions,
};
pub use server::{Backend, EventSink, ServerOptions, serve};
pub use settings::{DaemonSettings, DaemonSettingsStore};
pub use workspace::{WorkspaceOperation, WorkspaceResult};
