//! AgentTeams for the built-in agent.
//!
//! The current built-in-agent session becomes the **captain**: it creates
//! resumable sub-agent members, splits a goal into tasks with explicit
//! dependencies, and coordinates the members through durable mailboxes. A
//! shared scheduler claims ready tasks for idle members; quality gates keep a
//! team from declaring work done that was not verified and reviewed.
//!
//! This crate is the pure part: the durable record and its JSON boundary,
//! state-file I/O, the task state machine, scheduling decisions, the
//! quality-gate rules, team profiles, prompt text and the view model the
//! desktop draws. The bridge (`waku-agent-bridge/src/team/`) runs members and
//! exposes the tools; the app (`src/app/team_panel.rs`) shows the team.
//!
//! Translated from `@nanmicoder/dsh-agent-teams` 0.1.22 — MIT License,
//! Copyright (c) 2026 程序员阿江(Relakkes). See NOTICE.md for the full notice.

pub mod command;
pub mod config;
pub mod key;
pub mod mailbox;
pub mod profiles;
pub mod prompts;
pub mod quality;
pub mod requests;
pub mod runtime;
pub mod scheduler;
pub mod snapshot;
pub mod store;
pub mod transitions;
pub mod types;
pub mod validate;

pub use key::{CAPTAIN_KEY, sanitize_key};
pub use types::*;
