//! Staged multi-agent workflows.
//!
//! A run is a small graph of stages; each stage becomes one agent session
//! sharing a worktree with the others. This crate holds everything about a
//! run that is not a view: the graph and its scheduling rules ([`model`]),
//! the planner that asks a model through the gateway what to do next
//! ([`planner`]), and the objective check that decides whether a stage
//! passed ([`check`]). Like `sub2api`, it is free of GPUI and of the
//! upstream crates, so it compiles and tests in seconds; the desktop's
//! `src/app/workflow.rs` is the only consumer.
//!
//! Kept apart from `sub2api` on purpose: that crate is the managed account,
//! routing and CLI installation, and a workflow engine is none of those.

pub mod check;
pub mod model;
pub mod planner;
