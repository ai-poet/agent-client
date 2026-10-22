//! Trajectory for the built-in agent.
//!
//! A timed record of what a session's agent did, request by request: when
//! each model request went out, when its first token came back and when it
//! ended, what it cost in tokens, what it said and thought, every retry,
//! every tool call with its payload and result, every compaction, and the
//! system prompt and tool catalog each request ran with. The desktop draws
//! it as a ledger with a three-lane timeline above it and an inspector
//! beside it.
//!
//! This crate is the pure part: the record and its JSON lines
//! ([`record`]), the files they are written to ([`writer`]) and read back
//! from ([`reader`]), and the view model built from them ([`snapshot`],
//! [`timeline`], [`metrics`], [`prompt_diff`]). The bridge
//! (`waku-agent-bridge/src/trajectory.rs`) writes it; the app
//! (`src/app/trajectory_panel.rs`) draws it.
//!
//! The ledger model, the timeline projection and the timing rules are
//! translated from DeepSeek Harness's `ui-trajectory` 0.2.1-alpha.2 — MIT
//! License, Copyright (c) 2026 DeepSeek. See NOTICE.md for the full notice.

pub mod metrics;
pub mod prompt_diff;
pub mod reader;
pub mod record;
pub mod snapshot;
pub mod timeline;
pub mod writer;

pub use record::{Record, SCHEMA};
pub use snapshot::{Cell, CellKind, Snapshot, build};
