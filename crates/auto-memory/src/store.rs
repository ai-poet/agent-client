// Translated from dsh-auto-memory (MIT, Copyright (c) 2026 AskTheWay); see NOTICE.md.

//! The memory store: typed memory files and each scope's `MEMORY.md`.
//!
//! The files are the truth; `MEMORY.md` is derived and rebuilt in full
//! inside the scope lock after every write and delete, and removed when the
//! scope is empty (so an empty store adds nothing to the prompt). No single
//! malformed or hostile file can break an operation: it is skipped, as are
//! symbolic links and files whose stem is not their own name.
//!
//! Layout under the root (`<engine config>/auto-memory`):
//! `user/*.md` and `projects/<project key>/*.md`, each with its `MEMORY.md`
//! and, for a project, a `project.json` naming the workspace.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::claude_mirror::ClaudeMirror;
use crate::frontmatter::{self, Value, quoted, scalar};
use crate::fsutil::{atomic_write_text, is_symlink};
use crate::lock::with_scope_lock;
use crate::name::{normalize_name, project_key};
use crate::types::{MemoryDraft, MemoryRecord, MemoryScope, MemoryType, ScopeDir};

/// The derived index file.
pub const INDEX_FILE: &str = "MEMORY.md";
/// A project directory's note of the workspace it belongs to.
pub const PROJECT_FILE: &str = "project.json";
/// The user scope's directory.
pub const USER_DIR: &str = "user";
/// The parent of every project directory.
pub const PROJECTS_DIR: &str = "projects";

const DAY_MS: u64 = 86_400_000;

/// Read one memory file. `None` for anything malformed — including a name
/// that does not normalize, which no writer here produces.
pub fn parse_record(raw: &str, scope: MemoryScope) -> Option<MemoryRecord> {
    let doc = frontmatter::parse(raw)?;
    let fields = &doc.fields;
    let name = normalize_name(fields.get("name")?.as_text()?).ok()?;
    let description = fields.get("description")?.as_text()?.trim().to_owned();
    if description.is_empty() {
        return None;
    }
    let kind = match fields.get("type") {
        None => fields
            .get("metadata")
            .and_then(Value::as_map)
            .and_then(|metadata| metadata.get("type"))
            .and_then(Value::as_text)
            .map_or(Some(MemoryType::Reference), MemoryType::parse)?,
        Some(value) => MemoryType::parse(value.as_text()?)?,
    };
    let millis = |key: &str| {
        fields
            .get(key)
            .and_then(Value::as_int)
            .filter(|value| *value > 0)
            .map(|value| value as u64)
    };
    Some(MemoryRecord {
        name,
        title: fields
            .get("title")
            .and_then(Value::as_text)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned),
        description,
        kind,
        body: doc.body.trim().to_owned(),
        scope,
        pinned: fields.get("pinned").and_then(Value::as_bool) == Some(true),
        created_ms: millis("created"),
        updated_ms: millis("updated"),
        last_read_ms: millis("lastRead"),
        reads: fields
            .get("reads")
            .and_then(Value::as_int)
            .filter(|reads| *reads >= 0)
            .map(|reads| reads as u64),
    })
}

/// Write one memory file (the reference's key order).
pub fn serialize_record(record: &MemoryRecord) -> String {
    let mut lines = vec!["---".to_owned(), format!("name: {}", record.name)];
    if let Some(title) = &record.title {
        lines.push(format!("title: {}", quoted(title)));
    }
    lines.push(format!("description: {}", quoted(&record.description)));
    lines.push(format!("type: {}", scalar(record.kind.as_str())));
    if record.pinned {
        lines.push("pinned: true".to_owned());
    }
    for (key, value) in [
        ("created", record.created_ms),
        ("updated", record.updated_ms),
        ("lastRead", record.last_read_ms),
        ("reads", record.reads),
    ] {
        if let Some(value) = value {
            lines.push(format!("{key}: {value}"));
        }
    }
    lines.push("---".to_owned());
    format!("{}\n\n{}\n", lines.join("\n"), record.body.trim())
}

/// One line of whitespace: titles and descriptions become index lines.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a memory is out of the injected index: older than the threshold,
/// never read, not pinned. Records without lifecycle metadata never are.
pub fn is_stale(record: &MemoryRecord, stale_after_days: u32, now_ms: u64) -> bool {
    if stale_after_days == 0 || record.pinned || record.reads.unwrap_or(0) > 0 {
        return false;
    }
    let Some(updated) = record.updated_ms.or(record.created_ms) else {
        return false;
    };
    now_ms.saturating_sub(updated) > u64::from(stale_after_days) * DAY_MS
}

/// The index text: pinned first, then by name — stable, so the prompt
/// prefix stays cacheable. Empty for no records.
pub fn render_index_body(records: &[MemoryRecord]) -> String {
    let mut ordered = records.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut text = String::new();
    for record in ordered {
        text.push_str(&format!(
            "- [{}]({}.md){} — {}\n",
            record.heading(),
            record.name,
            if record.pinned { " 📌" } else { "" },
            record.description
        ));
    }
    text
}

/// A prune candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PruneCandidate {
    pub name: String,
    pub description: String,
    pub scope: MemoryScope,
    pub days_since_update: u64,
}

/// The store.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    root: PathBuf,
    stale_after_days: u32,
    mirror: Option<ClaudeMirror>,
}

impl MemoryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            stale_after_days: 0,
            mirror: None,
        }
    }

    pub fn with_stale_after_days(mut self, days: u32) -> Self {
        self.stale_after_days = days;
        self
    }

    pub fn with_claude_mirror(mut self, mirror: Option<ClaudeMirror>) -> Self {
        self.mirror = mirror;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn stale_after_days(&self) -> u32 {
        self.stale_after_days
    }

    pub fn user_dir(&self) -> ScopeDir {
        ScopeDir {
            scope: MemoryScope::User,
            dir: self.root.join(USER_DIR),
            cwd: None,
        }
    }

    pub fn project_dir(&self, cwd: &Path) -> ScopeDir {
        ScopeDir {
            scope: MemoryScope::Project,
            dir: self.root.join(PROJECTS_DIR).join(project_key(cwd)),
            cwd: Some(cwd.to_path_buf()),
        }
    }

    /// A project directory found on disk, with the workspace its
    /// `project.json` names.
    pub fn project_dir_by_key(&self, key: &str) -> ScopeDir {
        let dir = self.root.join(PROJECTS_DIR).join(key);
        let cwd = read_project_cwd(&dir);
        ScopeDir {
            scope: MemoryScope::Project,
            dir,
            cwd,
        }
    }

    /// Every memory in a scope, by name. Malformed files, symlinks, the
    /// index and files whose stem is not their name are skipped.
    pub fn list(&self, dir: &ScopeDir) -> Vec<MemoryRecord> {
        let Ok(entries) = std::fs::read_dir(&dir.dir) else {
            return Vec::new();
        };
        let mut records = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let stem = file_name.strip_suffix(".md")?;
                if file_name.eq_ignore_ascii_case(INDEX_FILE) || is_symlink(&path) {
                    return None;
                }
                if !entry.file_type().ok()?.is_file() {
                    return None;
                }
                let raw = std::fs::read_to_string(&path).ok()?;
                parse_record(&raw, dir.scope).filter(|record| record.name == stem)
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.name.cmp(&right.name));
        records
    }

    /// One memory, by a name that is normalized first.
    pub fn read(&self, dir: &ScopeDir, name: &str) -> anyhow::Result<Option<MemoryRecord>> {
        let name = normalize_name(name).map_err(anyhow::Error::msg)?;
        Ok(read_file(&dir.dir, &name, dir.scope))
    }

    /// The first scope in `dirs` holding `name`, with its record.
    pub fn find_in(
        &self,
        name: &str,
        dirs: &[ScopeDir],
    ) -> anyhow::Result<Option<(ScopeDir, MemoryRecord)>> {
        let name = normalize_name(name).map_err(anyhow::Error::msg)?;
        Ok(dirs.iter().find_map(|dir| {
            read_file(&dir.dir, &name, dir.scope).map(|record| (dir.clone(), record))
        }))
    }

    /// Create or update a memory, rebuild the index, mirror a project
    /// memory into Claude Code. Returns the record and whether it was new.
    pub fn write(&self, dir: &ScopeDir, draft: MemoryDraft) -> anyhow::Result<(MemoryRecord, bool)> {
        let name = normalize_name(&draft.name).map_err(anyhow::Error::msg)?;
        let description = one_line(&draft.description);
        if description.is_empty() {
            anyhow::bail!("memory description must not be empty");
        }
        let title = draft
            .title
            .as_deref()
            .map(one_line)
            .filter(|title| !title.is_empty());
        let (record, created) = with_scope_lock(&dir.dir, || {
            let existing = read_file(&dir.dir, &name, dir.scope);
            let now = crate::now_ms();
            let record = MemoryRecord {
                name: name.clone(),
                title: title.clone(),
                description: description.clone(),
                kind: draft.kind,
                body: draft.body.trim().to_owned(),
                scope: dir.scope,
                pinned: draft
                    .pinned
                    .unwrap_or_else(|| existing.as_ref().is_some_and(|record| record.pinned)),
                created_ms: Some(
                    existing
                        .as_ref()
                        .and_then(|record| record.created_ms)
                        .unwrap_or(now),
                ),
                updated_ms: Some(now),
                last_read_ms: existing.as_ref().and_then(|record| record.last_read_ms),
                reads: existing.as_ref().and_then(|record| record.reads),
            };
            let file = dir.dir.join(format!("{name}.md"));
            atomic_write_text(&file, &serialize_record(&record))
                .with_context(|| format!("failed to write {}", file.display()))?;
            if let Some(cwd) = &dir.cwd {
                note_project(&dir.dir, cwd);
            }
            self.rebuild_index(dir)?;
            Ok((record, existing.is_none()))
        })?;
        self.mirror_write(dir, &record);
        Ok((record, created))
    }

    /// Count one read (best effort: a failure only loses the count). The
    /// index is rebuilt only when the read revives a stale memory.
    pub fn touch(&self, dir: &ScopeDir, name: &str) {
        let Ok(name) = normalize_name(name) else {
            return;
        };
        let file = dir.dir.join(format!("{name}.md"));
        if !file.is_file() {
            return;
        }
        let result = with_scope_lock(&dir.dir, || {
            let Some(record) = read_file(&dir.dir, &name, dir.scope) else {
                return Ok(());
            };
            let revives = is_stale(&record, self.stale_after_days, crate::now_ms());
            let touched = MemoryRecord {
                reads: Some(record.reads.unwrap_or(0) + 1),
                last_read_ms: Some(crate::now_ms()),
                ..record
            };
            atomic_write_text(&file, &serialize_record(&touched))?;
            if revives {
                self.rebuild_index(dir)?;
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::debug!("memory read count not recorded: {error:#}");
        }
    }

    /// Delete one memory; `false` when there was none.
    pub fn delete(&self, dir: &ScopeDir, name: &str) -> anyhow::Result<bool> {
        let name = normalize_name(name).map_err(anyhow::Error::msg)?;
        let file = dir.dir.join(format!("{name}.md"));
        if !dir.dir.is_dir() || !file.is_file() {
            return Ok(false);
        }
        let removed = with_scope_lock(&dir.dir, || {
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(error).with_context(|| format!("failed to delete {}", file.display()));
                }
            }
            self.rebuild_index(dir)?;
            Ok(true)
        })?;
        if removed {
            self.mirror_remove(dir, &[name]);
        }
        Ok(removed)
    }

    /// Delete every memory in a scope, in one lock window (a concurrent
    /// write cannot slip past "delete all"); returns how many went.
    pub fn clear(&self, dir: &ScopeDir) -> anyhow::Result<usize> {
        if !dir.dir.is_dir() {
            return Ok(0);
        }
        let removed = with_scope_lock(&dir.dir, || {
            let mut removed = Vec::new();
            for entry in std::fs::read_dir(&dir.dir)?.filter_map(Result::ok) {
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let Some(stem) = file_name.strip_suffix(".md") else {
                    continue;
                };
                if file_name.eq_ignore_ascii_case(INDEX_FILE) {
                    continue;
                }
                // One file held open elsewhere is skipped and not counted.
                if std::fs::remove_file(entry.path()).is_ok() {
                    removed.push(stem.to_owned());
                }
            }
            let _ = std::fs::remove_file(dir.dir.join(INDEX_FILE));
            Ok(removed)
        })?;
        self.mirror_remove(dir, &removed);
        Ok(removed.len())
    }

    /// Rebuild a scope's index under its lock (the session-start refresh:
    /// eviction is evaluated at rebuild, and a store nobody writes to would
    /// otherwise never evict). Silent on failure.
    pub fn refresh_index(&self, dir: &ScopeDir) {
        if !dir.dir.is_dir() {
            return;
        }
        if let Err(error) = with_scope_lock(&dir.dir, || self.rebuild_index(dir)) {
            tracing::debug!("memory index refresh failed: {error:#}");
        }
    }

    /// The index text, `None` when there is none.
    pub fn read_index(&self, dir: &ScopeDir) -> Option<String> {
        let file = dir.dir.join(INDEX_FILE);
        if is_symlink(&file) {
            return None;
        }
        std::fs::read_to_string(file)
            .ok()
            .filter(|text| !text.trim().is_empty())
    }

    /// Memories last updated at least `older_than_days` ago. Pinned
    /// memories and records without lifecycle metadata never match.
    pub fn prune_candidates(
        &self,
        dirs: &[ScopeDir],
        older_than_days: u32,
        now_ms: u64,
    ) -> Vec<PruneCandidate> {
        let mut candidates = Vec::new();
        for dir in dirs {
            for record in self.list(dir) {
                if record.pinned {
                    continue;
                }
                let Some(updated) = record.updated_ms.or(record.created_ms) else {
                    continue;
                };
                let days = now_ms.saturating_sub(updated) / DAY_MS;
                if days >= u64::from(older_than_days) {
                    candidates.push(PruneCandidate {
                        name: record.name,
                        description: record.description,
                        scope: dir.scope,
                        days_since_update: days,
                    });
                }
            }
        }
        candidates
    }

    /// Rebuild a scope's index; the caller holds the lock.
    fn rebuild_index(&self, dir: &ScopeDir) -> anyhow::Result<()> {
        let now = crate::now_ms();
        let visible = self
            .list(dir)
            .into_iter()
            .filter(|record| !is_stale(record, self.stale_after_days, now))
            .collect::<Vec<_>>();
        let index = dir.dir.join(INDEX_FILE);
        if visible.is_empty() {
            match std::fs::remove_file(&index) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            return Ok(());
        }
        atomic_write_text(&index, &render_index_body(&visible))
            .with_context(|| format!("failed to write {}", index.display()))
    }

    /// Mirror every memory of a project scope into Claude Code — for the
    /// mirror being switched back on, when the memories written while it
    /// was off have no copy there yet. A no-op without a mirror.
    pub fn sync_mirror(&self, dir: &ScopeDir) {
        if self.mirror.is_none() {
            return;
        }
        for record in self.list(dir) {
            self.mirror_write(dir, &record);
        }
    }

    fn mirror_write(&self, dir: &ScopeDir, record: &MemoryRecord) {
        if let (Some(mirror), MemoryScope::Project, Some(cwd)) = (&self.mirror, dir.scope, &dir.cwd)
            && let Err(error) = mirror.write(cwd, record)
        {
            tracing::warn!("memory not mirrored to Claude Code: {error:#}");
        }
    }

    fn mirror_remove(&self, dir: &ScopeDir, names: &[String]) {
        if names.is_empty() {
            return;
        }
        if let (Some(mirror), MemoryScope::Project, Some(cwd)) = (&self.mirror, dir.scope, &dir.cwd)
            && let Err(error) = mirror.remove(cwd, names)
        {
            tracing::warn!("memory mirror in Claude Code not removed: {error:#}");
        }
    }
}

fn read_file(dir: &Path, name: &str, scope: MemoryScope) -> Option<MemoryRecord> {
    let file = dir.join(format!("{name}.md"));
    if is_symlink(&file) {
        return None;
    }
    let raw = std::fs::read_to_string(file).ok()?;
    parse_record(&raw, scope).filter(|record| record.name == name)
}

/// Record which workspace a project directory belongs to (once).
fn note_project(dir: &Path, cwd: &Path) {
    let file = dir.join(PROJECT_FILE);
    if read_project_cwd(dir).as_deref() == Some(cwd) {
        return;
    }
    let text = serde_json::json!({ "cwd": cwd.to_string_lossy() }).to_string();
    if let Err(error) = atomic_write_text(&file, &format!("{text}\n")) {
        tracing::debug!("{} not written: {error}", file.display());
    }
}

fn read_project_cwd(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(PROJECT_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("cwd")?.as_str().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str, description: &str) -> MemoryDraft {
        MemoryDraft {
            name: name.to_owned(),
            title: None,
            description: description.to_owned(),
            kind: MemoryType::Project,
            body: format!("Body of {name}."),
            pinned: None,
        }
    }

    fn store() -> (tempfile::TempDir, MemoryStore, ScopeDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().join("auto-memory"));
        let project = store.project_dir(Path::new("/work/app"));
        (dir, store, project)
    }

    #[test]
    fn records_round_trip_through_the_file_format() {
        let record = MemoryRecord {
            name: "user-prefers-python".into(),
            title: Some("用户是 Python 后端工程师".into()),
            description: "Prefers: \"python\" — and pnpm".into(),
            kind: MemoryType::User,
            body: "Line one\n\n**Why:** because".into(),
            scope: MemoryScope::User,
            pinned: true,
            created_ms: Some(10),
            updated_ms: Some(20),
            last_read_ms: Some(30),
            reads: Some(2),
        };
        let parsed = parse_record(&serialize_record(&record), MemoryScope::User).unwrap();
        assert_eq!(parsed, record);
    }

    #[test]
    fn malformed_records_are_none() {
        for raw in [
            "no frontmatter",
            "---\ndescription: x\n---\n",
            "---\nname: a\n---\n",
            "---\nname: a\ndescription: x\ntype: other\n---\n",
            "---\nname: 中文\ndescription: x\n---\n",
        ] {
            assert!(parse_record(raw, MemoryScope::Project).is_none(), "{raw}");
        }
        // Missing type reads as reference; Claude Code's nested type counts.
        let plain = parse_record("---\nname: a\ndescription: x\n---\n", MemoryScope::Project).unwrap();
        assert_eq!(plain.kind, MemoryType::Reference);
        let nested = parse_record(
            "---\nname: a\ndescription: x\nmetadata:\n  type: feedback\n---\n",
            MemoryScope::Project,
        )
        .unwrap();
        assert_eq!(nested.kind, MemoryType::Feedback);
    }

    #[test]
    fn writes_update_by_name_and_keep_the_index_in_order() {
        let (_tmp, store, project) = store();
        let (_, created) = store.write(&project, draft("b-fact", "second")).unwrap();
        assert!(created);
        store.write(&project, draft("a-fact", "first")).unwrap();
        let (record, created) = store.write(&project, draft("b-fact", "second, revised")).unwrap();
        assert!(!created);
        assert_eq!(record.description, "second, revised");
        assert_eq!(store.list(&project).len(), 2);
        assert_eq!(
            store.read_index(&project).unwrap(),
            "- [a-fact](a-fact.md) — first\n- [b-fact](b-fact.md) — second, revised\n"
        );
        let cwd = read_project_cwd(&project.dir).unwrap();
        assert_eq!(cwd, Path::new("/work/app"));
    }

    #[test]
    fn pinned_memories_lead_the_index_and_keep_their_pin() {
        let (_tmp, store, project) = store();
        store.write(&project, draft("a-fact", "a")).unwrap();
        store
            .write(&project, MemoryDraft { pinned: Some(true), ..draft("z-fact", "z") })
            .unwrap();
        // An update that does not mention the pin keeps it.
        store.write(&project, draft("z-fact", "z2")).unwrap();
        let index = store.read_index(&project).unwrap();
        assert!(index.starts_with("- [z-fact](z-fact.md) 📌 — z2\n"), "{index}");
    }

    #[test]
    fn lifecycle_metadata_survives_updates() {
        let (_tmp, store, project) = store();
        let (first, _) = store.write(&project, draft("a", "x")).unwrap();
        store.touch(&project, "a");
        store.touch(&project, "a");
        let (second, _) = store.write(&project, draft("a", "y")).unwrap();
        assert_eq!(second.created_ms, first.created_ms);
        assert_eq!(second.reads, Some(2));
        assert!(second.last_read_ms.is_some());
    }

    #[test]
    fn deleting_the_last_memory_removes_the_index() {
        let (_tmp, store, project) = store();
        store.write(&project, draft("a", "x")).unwrap();
        assert!(store.delete(&project, "a").unwrap());
        assert!(!store.delete(&project, "a").unwrap());
        assert!(store.read_index(&project).is_none());
        let missing = store.project_dir(Path::new("/nowhere"));
        assert!(!store.delete(&missing, "a").unwrap());
    }

    #[test]
    fn listing_skips_the_index_malformed_files_and_foreign_stems() {
        let (_tmp, store, project) = store();
        store.write(&project, draft("good", "x")).unwrap();
        std::fs::write(project.dir.join("broken.md"), "not a memory").unwrap();
        std::fs::write(
            project.dir.join("other.md"),
            "---\nname: good\ndescription: impostor\n---\n",
        )
        .unwrap();
        let names = store
            .list(&project)
            .into_iter()
            .map(|record| record.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["good"]);
    }

    #[test]
    fn scopes_are_isolated() {
        let (_tmp, store, project) = store();
        let user = store.user_dir();
        let other = store.project_dir(Path::new("/work/other"));
        store.write(&project, draft("a", "project")).unwrap();
        store.write(&user, draft("a", "user")).unwrap();
        assert!(store.list(&other).is_empty());
        let (found, record) = store
            .find_in("a", &[user.clone(), project.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(found.scope, MemoryScope::User);
        assert_eq!(record.description, "user");
    }

    #[test]
    fn clear_removes_every_memory_and_the_index() {
        let (_tmp, store, project) = store();
        for name in ["a", "b", "c"] {
            store.write(&project, draft(name, "x")).unwrap();
        }
        assert_eq!(store.clear(&project).unwrap(), 3);
        assert!(store.list(&project).is_empty());
        assert!(store.read_index(&project).is_none());
    }

    #[test]
    fn concurrent_writers_leave_a_complete_index() {
        let (_tmp, store, project) = store();
        let handles = (0..12)
            .map(|index| {
                let store = store.clone();
                let project = project.clone();
                std::thread::spawn(move || {
                    store
                        .write(&project, draft(&format!("fact-{index:02}"), "x"))
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(store.read_index(&project).unwrap().lines().count(), 12);
    }

    fn aged(store: &MemoryStore, dir: &ScopeDir, name: &str, days: u64, reads: u64, pinned: bool) {
        let now = crate::now_ms();
        let record = MemoryRecord {
            name: name.into(),
            title: None,
            description: name.into(),
            kind: MemoryType::Project,
            body: "b".into(),
            scope: dir.scope,
            pinned,
            created_ms: Some(now - days * DAY_MS),
            updated_ms: Some(now - days * DAY_MS),
            last_read_ms: None,
            reads: (reads > 0).then_some(reads),
        };
        std::fs::create_dir_all(&dir.dir).unwrap();
        std::fs::write(dir.dir.join(format!("{name}.md")), serialize_record(&record)).unwrap();
        let _ = store;
    }

    #[test]
    fn eviction_hides_only_stale_unread_unpinned_memories() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path()).with_stale_after_days(30);
        let project = store.project_dir(Path::new("/w"));
        aged(&store, &project, "stale-unread", 40, 0, false);
        aged(&store, &project, "stale-read", 40, 3, false);
        aged(&store, &project, "stale-pinned", 40, 0, true);
        aged(&store, &project, "fresh", 1, 0, false);
        store.refresh_index(&project);
        let index = store.read_index(&project).unwrap();
        assert!(!index.contains("stale-unread"));
        for kept in ["stale-read", "stale-pinned", "fresh"] {
            assert!(index.contains(kept), "{kept}");
        }
        // The file stays, and one read brings it back.
        assert_eq!(store.list(&project).len(), 4);
        store.touch(&project, "stale-unread");
        assert!(store.read_index(&project).unwrap().contains("stale-unread"));
    }

    #[test]
    fn prune_skips_pinned_and_unstamped_memories() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path());
        let project = store.project_dir(Path::new("/w"));
        aged(&store, &project, "old", 40, 0, false);
        aged(&store, &project, "old-pinned", 40, 0, true);
        aged(&store, &project, "new", 2, 0, false);
        std::fs::write(project.dir.join("legacy.md"), "---\nname: legacy\ndescription: x\n---\n").unwrap();
        let names = store
            .prune_candidates(std::slice::from_ref(&project), 30, crate::now_ms())
            .into_iter()
            .map(|candidate| candidate.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["old"]);
    }
}
