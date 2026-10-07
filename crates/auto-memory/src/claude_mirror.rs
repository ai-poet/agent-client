//! The one-way mirror of project memories into Claude Code's own project
//! memory, so the Claude Code CLI working in the same folder knows what the
//! built-in agent learned.
//!
//! Claude Code keeps `~/.claude/projects/<slug>/memory/` (`$CLAUDE_CONFIG_DIR`
//! when set): one Markdown file per memory with `name`, `description` and a
//! `metadata:` block holding the type, and a `MEMORY.md` of
//! `- [Title](file.md) — hook` lines. The slug is the workspace path with
//! every character outside `[A-Za-z0-9]` turned into `-`.
//!
//! Rules: the store here stays the truth and the mirror is best effort.
//! Mirrored files carry `metadata.origin: cheaprouter`, and only such files
//! are ever replaced or removed — a Claude Code memory of the same name is
//! left alone. In `MEMORY.md` only the line pointing at a mirrored file is
//! added, replaced or dropped; Claude Code's own lines stay as they are.
//! User-scope memories are not mirrored (Claude Code has no global memory
//! directory of this kind), and nothing flows back.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::frontmatter::{self, Value, quoted, scalar};
use crate::fsutil::{atomic_write_text, is_symlink};
use crate::types::MemoryRecord;

/// The marker on every file this mirror writes.
pub const ORIGIN: &str = "cheaprouter";
const INDEX_FILE: &str = "MEMORY.md";
/// Claude Code shortens longer slugs with a hash of its own; such
/// workspaces are not mirrored rather than mirrored to the wrong place.
const SLUG_MAX: usize = 200;

/// What a mirror write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorOutcome {
    Written,
    /// A Claude Code memory of the same name exists and was left alone.
    SkippedForeign,
    /// The workspace path is too long to name Claude Code's directory.
    SkippedPath,
}

/// Where Claude Code keeps project memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeMirror {
    projects: PathBuf,
}

impl ClaudeMirror {
    /// `$CLAUDE_CONFIG_DIR/projects`, else `~/.claude/projects`.
    pub fn from_env() -> Option<Self> {
        let base = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .filter(|dir| !dir.as_os_str().is_empty())
            .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))?;
        Some(Self::new(base))
    }

    /// A mirror into `claude_dir` (the directory holding `projects/`).
    pub fn new(claude_dir: impl Into<PathBuf>) -> Self {
        Self {
            projects: claude_dir.into().join("projects"),
        }
    }

    /// `<claude>/projects/<slug>/memory`, `None` for a path too long.
    pub fn memory_dir(&self, cwd: &Path) -> Option<PathBuf> {
        let slug = claude_project_slug(cwd);
        (slug.len() <= SLUG_MAX).then(|| self.projects.join(slug).join("memory"))
    }

    /// Mirror one project memory.
    pub fn write(&self, cwd: &Path, record: &MemoryRecord) -> anyhow::Result<MirrorOutcome> {
        let Some(dir) = self.memory_dir(cwd) else {
            return Ok(MirrorOutcome::SkippedPath);
        };
        let file = dir.join(format!("{}.md", record.name));
        if file.exists() && !is_ours(&file) {
            tracing::warn!(
                "Claude Code already has a memory named {}; not mirrored",
                record.name
            );
            return Ok(MirrorOutcome::SkippedForeign);
        }
        atomic_write_text(&file, &render_file(record))
            .with_context(|| format!("failed to write {}", file.display()))?;
        let line = index_line(record);
        update_index(&dir, &record.name, Some(&line))?;
        Ok(MirrorOutcome::Written)
    }

    /// Drop mirrored memories; returns how many files went.
    pub fn remove(&self, cwd: &Path, names: &[String]) -> anyhow::Result<usize> {
        let Some(dir) = self.memory_dir(cwd) else {
            return Ok(0);
        };
        let mut removed = 0;
        for name in names {
            let file = dir.join(format!("{name}.md"));
            if file.exists() {
                if !is_ours(&file) {
                    continue;
                }
                std::fs::remove_file(&file)
                    .with_context(|| format!("failed to delete {}", file.display()))?;
                removed += 1;
            }
            // Only once the file is gone: a dangling line points nowhere.
            update_index(&dir, name, None)?;
        }
        Ok(removed)
    }
}

/// Claude Code's project directory name for a workspace.
pub fn claude_project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn is_ours(file: &Path) -> bool {
    if is_symlink(file) {
        return false;
    }
    std::fs::read_to_string(file)
        .ok()
        .and_then(|raw| frontmatter::parse(&raw))
        .and_then(|doc| {
            doc.fields
                .get("metadata")
                .and_then(Value::as_map)
                .and_then(|metadata| metadata.get("origin"))
                .and_then(Value::as_text)
                .map(|origin| origin == ORIGIN)
        })
        .unwrap_or(false)
}

/// The file in Claude Code's format.
pub fn render_file(record: &MemoryRecord) -> String {
    let modified = record
        .updated_ms
        .and_then(|ms| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64))
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    format!(
        "---\nname: {}\ndescription: {}\nmetadata:\n  node_type: memory\n  type: {}\n  origin: {}\n  modified: {}\n---\n\n{}\n",
        scalar(&record.name),
        quoted(&record.description),
        record.kind.as_str(),
        ORIGIN,
        modified,
        record.body.trim()
    )
}

fn index_line(record: &MemoryRecord) -> String {
    format!(
        "- [{}]({}.md) — {}",
        record.heading(),
        record.name,
        record.description
    )
}

/// Replace, add or (with `None`) drop the index line pointing at `name`.
fn update_index(dir: &Path, name: &str, line: Option<&str>) -> anyhow::Result<()> {
    let index = dir.join(INDEX_FILE);
    if is_symlink(&index) {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&index).unwrap_or_default();
    let target = format!("]({name}.md)");
    let mut lines = Vec::new();
    let mut replaced = false;
    for current in existing.lines() {
        if current.contains(&target) {
            if let Some(line) = line
                && !replaced
            {
                lines.push(line.to_owned());
                replaced = true;
            }
            continue;
        }
        lines.push(current.to_owned());
    }
    if let Some(line) = line
        && !replaced
    {
        lines.push(line.to_owned());
    }
    let text = if lines.iter().all(|line| line.trim().is_empty()) {
        String::new()
    } else {
        format!("{}\n", lines.join("\n").trim_end())
    };
    if text == existing {
        return Ok(());
    }
    if text.is_empty() {
        if index.exists() {
            std::fs::remove_file(&index)
                .with_context(|| format!("failed to delete {}", index.display()))?;
        }
        return Ok(());
    }
    atomic_write_text(&index, &text).with_context(|| format!("failed to write {}", index.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryScope, MemoryType};

    fn record(name: &str, description: &str) -> MemoryRecord {
        MemoryRecord {
            name: name.into(),
            title: Some(format!("Title {name}")),
            description: description.into(),
            kind: MemoryType::Feedback,
            body: "Use pnpm.\n\n**Why:** lockfile".into(),
            scope: MemoryScope::Project,
            pinned: false,
            created_ms: Some(1),
            updated_ms: Some(1_791_070_200_000),
            last_read_ms: None,
            reads: None,
        }
    }

    #[test]
    fn slugs_match_claude_codes_directory_names() {
        assert_eq!(
            claude_project_slug(Path::new("C:\\Projects\\sub2api\\client")),
            "C--Projects-sub2api-client"
        );
        assert_eq!(claude_project_slug(Path::new("/home/ada/my_app")), "-home-ada-my-app");
    }

    #[test]
    fn writes_a_file_claude_code_and_this_crate_can_both_read() {
        let tmp = tempfile::tempdir().unwrap();
        let mirror = ClaudeMirror::new(tmp.path());
        let cwd = Path::new("/work/app");
        assert_eq!(mirror.write(cwd, &record("use-pnpm", "prefers pnpm")).unwrap(), MirrorOutcome::Written);
        let dir = mirror.memory_dir(cwd).unwrap();
        let raw = std::fs::read_to_string(dir.join("use-pnpm.md")).unwrap();
        assert!(raw.contains("  type: feedback\n  origin: cheaprouter\n  modified: 2026-10-03T23:30:00.000Z"));
        let parsed = crate::store::parse_record(&raw, MemoryScope::Project).unwrap();
        assert_eq!(parsed.kind, MemoryType::Feedback);
        assert_eq!(
            std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap(),
            "- [Title use-pnpm](use-pnpm.md) — prefers pnpm\n"
        );
    }

    #[test]
    fn claude_codes_own_memories_and_lines_are_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let mirror = ClaudeMirror::new(tmp.path());
        let cwd = Path::new("/work/app");
        let dir = mirror.memory_dir(cwd).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let theirs = "---\nname: theirs\ndescription: x\nmetadata:\n  type: user\n---\n\nmine\n";
        std::fs::write(dir.join("theirs.md"), theirs).unwrap();
        std::fs::write(dir.join(INDEX_FILE), "- [Theirs](theirs.md) — keep me\n").unwrap();

        assert_eq!(
            mirror.write(cwd, &record("theirs", "overwrite?")).unwrap(),
            MirrorOutcome::SkippedForeign
        );
        assert_eq!(std::fs::read_to_string(dir.join("theirs.md")).unwrap(), theirs);

        mirror.write(cwd, &record("ours", "added")).unwrap();
        mirror.write(cwd, &record("ours", "replaced")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap(),
            "- [Theirs](theirs.md) — keep me\n- [Title ours](ours.md) — replaced\n"
        );

        assert_eq!(mirror.remove(cwd, &["ours".into(), "theirs".into()]).unwrap(), 1);
        assert!(dir.join("theirs.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap(),
            "- [Theirs](theirs.md) — keep me\n"
        );
    }

    #[test]
    fn the_store_mirrors_project_memories_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mirror = ClaudeMirror::new(tmp.path().join("claude"));
        let store = crate::MemoryStore::new(tmp.path().join("auto-memory"))
            .with_claude_mirror(Some(mirror.clone()));
        let cwd = Path::new("/work/app");
        let draft = |name: &str| crate::MemoryDraft {
            name: name.into(),
            title: None,
            description: "d".into(),
            kind: MemoryType::Project,
            body: "b".into(),
            pinned: None,
        };
        store.write(&store.project_dir(cwd), draft("project-fact")).unwrap();
        store.write(&store.user_dir(), draft("user-fact")).unwrap();
        let dir = mirror.memory_dir(cwd).unwrap();
        assert!(dir.join("project-fact.md").exists());
        assert!(!dir.join("user-fact.md").exists());

        store.delete(&store.project_dir(cwd), "project-fact").unwrap();
        assert!(!dir.join("project-fact.md").exists());
        assert!(!dir.join(INDEX_FILE).exists());
    }

    #[test]
    fn without_a_mirror_nothing_is_written_to_claude_code() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::MemoryStore::new(tmp.path().join("auto-memory"));
        let cwd = Path::new("/work/app");
        store
            .write(
                &store.project_dir(cwd),
                crate::MemoryDraft {
                    name: "a".into(),
                    title: None,
                    description: "d".into(),
                    kind: MemoryType::Project,
                    body: "b".into(),
                    pinned: None,
                },
            )
            .unwrap();
        assert!(!tmp.path().join("claude").exists());
    }

    #[test]
    fn switching_the_mirror_back_on_copies_what_was_written_meanwhile() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("auto-memory");
        let cwd = Path::new("/work/app");
        let unmirrored = crate::MemoryStore::new(&root);
        unmirrored
            .write(
                &unmirrored.project_dir(cwd),
                crate::MemoryDraft {
                    name: "written-offline".into(),
                    title: None,
                    description: "d".into(),
                    kind: MemoryType::Project,
                    body: "b".into(),
                    pinned: None,
                },
            )
            .unwrap();

        let mirror = ClaudeMirror::new(tmp.path().join("claude"));
        let store = crate::MemoryStore::new(&root).with_claude_mirror(Some(mirror.clone()));
        // As the settings page finds it: by key, the workspace from project.json.
        let key = crate::name::project_key(cwd);
        store.sync_mirror(&store.project_dir_by_key(&key));
        let dir = mirror.memory_dir(cwd).unwrap();
        assert!(dir.join("written-offline.md").exists());
        let index = std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
        assert!(index.contains("](written-offline.md)"), "{index}");
    }
}
