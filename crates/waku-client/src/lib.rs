//! Rust transport and lifecycle support for clients of `waku-daemon`.
//!
//! This crate intentionally depends only on [`waku_protocol`], so GUI and CLI
//! clients cannot accidentally reach daemon-owned filesystem, Git, database,
//! or provider implementations.

pub mod activity_phase;
mod client;
pub mod command_env;
pub mod composer_complete;
pub mod computer_use;
pub mod driver;
pub mod persistence;
// Fork addition: the digest a side question (`/btw`, ⌘;) is asked with.
pub mod side_chat;
mod process;
pub mod status_capsule;
pub mod turn_segments;
mod workspace_client;

pub use client::{
    DAEMON_CONNECTION_CLOSED, DAEMON_DISCONNECTED, DAEMON_DROPPED, DaemonClient,
    is_daemon_transport_error,
};
pub use process::{
    DEFAULT_EXPOSED_DAEMON_PORT, DaemonExposureSettings, DaemonProcess, DaemonSupervisor,
    parse_allowed_origins,
};
pub use waku_protocol::*;
pub use workspace_client::WorkspaceClient;

pub mod git_branch {
    pub use waku_protocol::git::{BranchEntry, BranchSnapshot};
}

pub mod git_commit {
    pub use waku_protocol::git::AgentInvocation;
    pub use waku_protocol::git::CommitSnapshot as Snapshot;
}

pub mod worktree {
    pub use waku_protocol::git::CreatedWorktree;
}
