//! Fork addition (Waku): what the model has seen of each file, so an edit
//! cannot land on a file it never read, or on one that changed since.
//!
//! Edit and Write read the file fresh and wrote it back whether or not the
//! model had ever looked at it, and whatever had happened to it since. An
//! edit planned against what the model read an hour ago then landed on top
//! of the person's own change, or a formatter's, or a command's, and undid
//! it without a word. Claude Code refuses both cases; so does this.
//!
//! A file counts as seen once a Read returned it or the agent itself wrote it
//! (`ToolContext::record_file_change`, and after a formatter ran). It must not
//! have changed since: same size and modification time, or — when only the
//! time moved, as a checkout or a `touch` does — the same content.
//!
//! The record is per session and in memory. A session resumed after a restart
//! starts with nothing seen, and the model reads a file again before changing
//! it, which costs one call and is the safe default.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dashmap::DashMap;
use once_cell::sync::Lazy;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Seen {
    len: u64,
    modified: Option<SystemTime>,
    digest: u64,
}

static SEEN: Lazy<DashMap<(String, PathBuf), Seen>> = Lazy::new(DashMap::new);

fn key(session: &str, path: &Path) -> (String, PathBuf) {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    (session.to_owned(), path)
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The model has seen `path` as `content`: a Read returned it, or the agent
/// wrote it.
pub fn record(session: &str, path: &Path, content: &[u8]) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    SEEN.insert(
        key(session, path),
        Seen {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            digest: digest(content),
        },
    );
}

/// [`record`] what is on disk now — after a formatter rewrote a file the
/// agent had just written.
pub fn record_current(session: &str, path: &Path) {
    if let Ok(bytes) = std::fs::read(path) {
        record(session, path, &bytes);
    }
}

/// Why `path` must not be changed yet, or `None` when it may be. A file that
/// does not exist yet may always be created.
pub fn refusal(session: &str, path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let key = key(session, path);
    let Some(seen) = SEEN.get(&key).map(|entry| *entry) else {
        return Some(format!(
            "{} has not been read yet. Read it first, then change it.",
            path.display()
        ));
    };
    let modified = metadata.modified().ok();
    if seen.len == metadata.len() && seen.modified == modified {
        return None;
    }
    let unchanged = std::fs::read(path).is_ok_and(|bytes| digest(&bytes) == seen.digest);
    if unchanged {
        SEEN.insert(
            key,
            Seen {
                len: metadata.len(),
                modified,
                digest: seen.digest,
            },
        );
        return None;
    }
    Some(format!(
        "{} has changed since it was last read — by the user, a formatter or a command. \
         Read it again before changing it.",
        path.display()
    ))
}

/// Mark `path` as seen for `session`, for tests that edit a file they set up
/// themselves.
#[cfg(test)]
pub(crate) fn mark_seen(session: &str, path: &Path) {
    record_current(session, path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unread_file_is_refused_and_a_read_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "one\n").unwrap();

        let refused = refusal("unread-session", &path).expect("never read");
        assert!(refused.contains("has not been read"), "{refused}");

        record("unread-session", &path, b"one\n");
        assert_eq!(refusal("unread-session", &path), None);
        // Another session has seen nothing.
        assert!(refusal("another-session", &path).is_some());
    }

    #[test]
    fn a_file_changed_since_it_was_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.txt");
        std::fs::write(&path, "before\n").unwrap();
        record("changed-session", &path, b"before\n");

        std::fs::write(&path, "the person's own change\n").unwrap();
        let refused = refusal("changed-session", &path).expect("changed since read");
        assert!(refused.contains("has changed"), "{refused}");
    }

    #[test]
    fn a_touch_that_keeps_the_content_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.txt");
        std::fs::write(&path, "same\n").unwrap();
        record("touched-session", &path, b"same\n");

        // Rewrite the same bytes after the clock has moved on.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "same\n").unwrap();
        assert_eq!(refusal("touched-session", &path), None);
    }

    #[test]
    fn a_new_file_may_always_be_created() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(refusal("new-session", &dir.path().join("new.txt")), None);
    }
}
