// Translated from dsh-auto-memory (MIT, Copyright (c) 2026 AskTheWay); see NOTICE.md.

//! The memory record: one Markdown file, Claude Code's four memory types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What a memory is about (Claude Code's four kinds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Who the user is: role, expertise, durable preferences.
    User,
    /// How to work: a correction or a confirmed approach, with why.
    Feedback,
    /// Ongoing work, goals and constraints (absolute dates).
    Project,
    /// Pointers to outside resources.
    Reference,
}

impl MemoryType {
    pub const ALL: [MemoryType; 4] = [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == raw.trim())
    }
}

/// Which directory a memory lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    /// One workspace: injected only into sessions working there.
    Project,
    /// Every session of this user.
    User,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryScope::Project => "project",
            MemoryScope::User => "user",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "project" => Some(MemoryScope::Project),
            "user" => Some(MemoryScope::User),
            _ => None,
        }
    }
}

/// One stored memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRecord {
    /// Kebab-case identifier, also the file name without `.md`.
    pub name: String,
    /// Human heading for the index (any language); the name when absent.
    pub title: Option<String>,
    /// One-line summary: shown in the index, used to spot duplicates.
    pub description: String,
    pub kind: MemoryType,
    /// The fact itself, Markdown.
    pub body: String,
    /// Which directory it was read from (not stored in the file).
    pub scope: MemoryScope,
    /// Leads the index, survives truncation, never evicted as stale.
    pub pinned: bool,
    pub created_ms: Option<u64>,
    pub updated_ms: Option<u64>,
    pub last_read_ms: Option<u64>,
    /// How often `memory_read` returned it.
    pub reads: Option<u64>,
}

impl MemoryRecord {
    /// The index heading: the title, or the name.
    pub fn heading(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// What a writer supplies; lifecycle metadata is the store's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryDraft {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub kind: MemoryType,
    pub body: String,
    /// `None` keeps the existing pin state.
    pub pinned: Option<bool>,
}

/// A scope's directory.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeDir {
    pub scope: MemoryScope,
    pub dir: PathBuf,
    /// The workspace a project scope belongs to (for its `project.json` and
    /// the Claude Code mirror). `None` for the user scope, and for a project
    /// directory found on disk whose workspace is unknown.
    pub cwd: Option<PathBuf>,
}
