//! Auto-memory for the built-in agent.
//!
//! Facts worth keeping across sessions — who the user is, how they want the
//! work done, what the project is in the middle of, where things live — are
//! stored as one Markdown file each, with a small YAML frontmatter. Every
//! scope keeps a derived `MEMORY.md` index, and the index (not the files) is
//! what the model sees on every request, within a byte budget. The model
//! reads, writes and forgets memories through six tools; an optional pass at
//! the end of a session extracts new memories by itself.
//!
//! Two scopes: **project** (one directory per workspace) and **user** (one
//! directory shared by every session). Project memories are also mirrored,
//! one way, into Claude Code's own project memory, so the Claude Code CLI in
//! the same workspace sees them ([`claude_mirror`]).
//!
//! This crate is the pure part: the record, its file format, the store with
//! its cross-process lock, the index and prompt section, the consolidation
//! prompt and the settings. The bridge (`waku-agent-bridge/src/memory/`)
//! exposes the tools and runs consolidation; the app
//! (`src/app/agent_memory_settings.rs`) manages the files.
//!
//! Translated from `dsh-auto-memory` 0.3.0 — MIT License, Copyright (c) 2026
//! AskTheWay. See NOTICE.md for the full notice.

pub mod claude_mirror;
pub mod config;
pub mod consolidate;
pub mod frontmatter;
pub mod fsutil;
pub mod links;
pub mod lock;
pub mod name;
pub mod prompt;
pub mod scan;
pub mod store;
pub mod types;

use std::path::{Path, PathBuf};

pub use config::MemoryConfig;
pub use store::MemoryStore;
pub use types::*;

/// Directory under the engine's config directory that holds every memory.
/// Not `memory/`: the vendored engine's AutoDream claims that one.
pub const MEMORY_DIR: &str = "auto-memory";

/// `<config_dir>/auto-memory`.
pub fn memory_root(config_dir: &Path) -> PathBuf {
    config_dir.join(MEMORY_DIR)
}

/// The store the settings describe: the stale threshold, and the Claude
/// Code mirror when it is switched on and Claude Code's directory resolves.
pub fn store_for(config_dir: &Path, config: &MemoryConfig) -> MemoryStore {
    let mirror = if config.mirror_to_claude_code {
        claude_mirror::ClaudeMirror::from_env()
    } else {
        None
    };
    MemoryStore::new(memory_root(config_dir))
        .with_stale_after_days(config.stale_after_days())
        .with_claude_mirror(mirror)
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
