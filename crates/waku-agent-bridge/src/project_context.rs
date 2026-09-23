//! Project instructions: the AGENTS.md / CLAUDE.md files that tell an agent
//! how a repository wants to be worked on.
//!
//! The engine knows how to find these (`claurst_core::context::ContextBuilder`),
//! but upstream calls it from its CLI crate, which is not vendored — so the
//! built-in agent never saw them, while Claude Code, Codex and Pi working in
//! the same repository all did.
//!
//! The rule is Pi's: the engine's own global file first, then for every
//! directory from the filesystem root down to the working directory, the first
//! of `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md` that exists. One per
//! directory, because a repository keeping both usually has one point at the
//! other. Read afresh for every turn, so an edit reaches the next one.

use std::path::{Path, PathBuf};

/// Checked in order; the first that exists in a directory is that
/// directory's file. Upper-case spellings for case-sensitive filesystems.
const CANDIDATES: [&str; 5] = [
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

/// Past this a file is cut. These are instructions, not documentation; one
/// this long is almost always a mistake, and it would ride along with every
/// request of every turn.
const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextFile {
    pub path: PathBuf,
    pub content: String,
}

/// Every instruction file that applies to `cwd`, outermost first.
pub(crate) fn load(cwd: &Path, global_dir: &Path) -> Vec<ContextFile> {
    let mut files: Vec<ContextFile> = Vec::new();
    let mut push = |file: ContextFile| {
        // A git worktree nested inside its own repository carries a copy of
        // the same file, and loading both would apply it twice.
        if !files.iter().any(|known| known.path == file.path || known.content == file.content) {
            files.push(file);
        }
    };
    if let Some(global) = file_in(global_dir) {
        push(global);
    }
    let mut ancestors: Vec<&Path> = cwd.ancestors().collect();
    ancestors.reverse();
    for dir in ancestors {
        if let Some(file) = file_in(dir) {
            push(file);
        }
    }
    files
}

fn file_in(dir: &Path) -> Option<ContextFile> {
    CANDIDATES.iter().find_map(|name| {
        let path = dir.join(name);
        if !path.is_file() {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let content = decode(&bytes);
        (!content.trim().is_empty()).then_some(ContextFile { path, content })
    })
}

fn decode(bytes: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if let Some(stripped) = text.strip_prefix('\u{feff}') {
        text = stripped.to_owned();
    }
    if text.len() > MAX_FILE_BYTES {
        let mut end = MAX_FILE_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let total = text.len();
        text.truncate(end);
        text.push_str(&format!(
            "\n[Truncated: this file is {} KB; only the first {} KB is included.]",
            total / 1024,
            MAX_FILE_BYTES / 1024
        ));
    }
    text
}

/// The files as a prompt section, or nothing when there are none.
pub(crate) fn render(files: &[ContextFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut section = String::from(
        "Project instructions follow: the repository's own guidance for agents working in \
         it, from its instruction files, outermost directory first. Follow them. Where they \
         conflict with the general guidance above, they take precedence; a file closer to \
         the working directory takes precedence over one further out.",
    );
    for file in files {
        section.push_str(&format!(
            "\n\n<project_instructions path=\"{}\">\n{}\n</project_instructions>",
            file.path.display(),
            file.content.trim_end()
        ));
    }
    Some(section)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn files_are_found_from_the_outside_in_one_per_directory() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let repo = root.path().join("repo");
        let crate_dir = repo.join("crates").join("app");
        write(&home.join("AGENTS.md"), "global rule");
        write(&repo.join("AGENTS.md"), "repo rule");
        // Both in one directory: AGENTS.md wins, the pointer is not loaded.
        write(&repo.join("CLAUDE.md"), "AGENTS.md");
        write(&crate_dir.join("CLAUDE.md"), "crate rule");

        let files = load(&crate_dir, &home);
        let contents: Vec<&str> = files.iter().map(|file| file.content.as_str()).collect();
        assert_eq!(contents, ["global rule", "repo rule", "crate rule"]);
    }

    #[test]
    fn an_override_beats_the_ordinary_file() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("AGENTS.md"), "shared");
        write(&root.path().join("AGENTS.override.md"), "mine");
        let files = load(root.path(), &root.path().join("nowhere"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "mine");
    }

    /// A worktree inside its repository has its own checkout of the same file.
    #[test]
    fn the_same_instructions_are_not_applied_twice() {
        let root = tempfile::tempdir().unwrap();
        let worktree = root.path().join(".worktrees").join("feature");
        write(&root.path().join("AGENTS.md"), "one rule");
        write(&worktree.join("AGENTS.md"), "one rule");
        assert_eq!(load(&worktree, &root.path().join("nowhere")).len(), 1);
    }

    #[test]
    fn a_byte_order_mark_and_empty_files_are_dropped() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("AGENTS.md"), "\u{feff}rule");
        let files = load(root.path(), &root.path().join("nowhere"));
        assert_eq!(files[0].content, "rule");

        let empty = tempfile::tempdir().unwrap();
        write(&empty.path().join("AGENTS.md"), "  \n");
        assert!(load(empty.path(), &empty.path().join("nowhere")).is_empty());
    }

    #[test]
    fn an_oversized_file_is_cut_and_says_so() {
        let text = "é".repeat(MAX_FILE_BYTES);
        let decoded = decode(text.as_bytes());
        assert!(decoded.len() < text.len());
        assert!(decoded.contains("[Truncated"));
    }

    #[test]
    fn rendering_names_each_file_and_nothing_renders_as_nothing() {
        assert_eq!(render(&[]), None);
        let rendered = render(&[ContextFile {
            path: PathBuf::from("/repo/AGENTS.md"),
            content: "Use tabs.\n".into(),
        }])
        .unwrap();
        assert!(rendered.contains("<project_instructions path=\"/repo/AGENTS.md\">\nUse tabs.\n</project_instructions>"));
    }
}
