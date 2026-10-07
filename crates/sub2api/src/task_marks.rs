//! What the person marked on their tasks: pinned, archived, unread.
//!
//! The task sidebar keeps pinned tasks in a section of their own across every
//! project, hides archived tasks behind an archive view instead of deleting
//! them, and dots the tasks whose reply finished while the person was looking
//! at something else. None of that belongs to a task's transcript, and the
//! daemon that owns the transcripts is shared with upstream, so the marks live
//! here, in one small file beside the other preferences (`~/.cheaprouter`).
//!
//! Debug and release builds keep separate session databases but share the
//! data directory, so each build has a file of its own; otherwise running both
//! at once would have each overwrite the other's marks with ids it does not
//! know.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::brand;
use crate::global_config::atomic_write_private;

/// The file the marks live in, one per build flavour.
pub const FILE_NAME: &str = if cfg!(debug_assertions) {
    "task-marks-debug.json"
} else {
    "task-marks.json"
};

/// Every mark, keyed by task id. Times are Unix seconds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskMarks {
    /// Pinned tasks and when they were pinned.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pinned: BTreeMap<Uuid, u64>,
    /// Archived tasks and when they were archived.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub archived: BTreeMap<Uuid, u64>,
    /// Tasks with a reply the person has not seen yet.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unread: BTreeSet<Uuid>,
}

impl TaskMarks {
    pub fn is_pinned(&self, id: Uuid) -> bool {
        self.pinned.contains_key(&id)
    }

    pub fn is_archived(&self, id: Uuid) -> bool {
        self.archived.contains_key(&id)
    }

    pub fn is_unread(&self, id: Uuid) -> bool {
        self.unread.contains(&id)
    }

    pub fn archived_at(&self, id: Uuid) -> Option<u64> {
        self.archived.get(&id).copied()
    }

    /// Pin a task. An archived task cannot be pinned: it is out of the list
    /// the pin would lift it to the top of. `true` when something changed.
    pub fn pin(&mut self, id: Uuid, now: u64) -> bool {
        if self.is_archived(id) || self.is_pinned(id) {
            return false;
        }
        self.pinned.insert(id, now);
        true
    }

    pub fn unpin(&mut self, id: Uuid) -> bool {
        self.pinned.remove(&id).is_some()
    }

    /// Archive a task. It leaves the pinned section and has nothing left to
    /// be unread about: archiving is putting it away.
    pub fn archive(&mut self, id: Uuid, now: u64) -> bool {
        if self.is_archived(id) {
            return false;
        }
        self.pinned.remove(&id);
        self.unread.remove(&id);
        self.archived.insert(id, now);
        true
    }

    pub fn unarchive(&mut self, id: Uuid) -> bool {
        self.archived.remove(&id).is_some()
    }

    /// Mark a task unread. An archived task has no dot to show, so it stays
    /// as it is.
    pub fn mark_unread(&mut self, id: Uuid) -> bool {
        !self.is_archived(id) && self.unread.insert(id)
    }

    pub fn mark_read(&mut self, id: Uuid) -> bool {
        self.unread.remove(&id)
    }

    /// Drop every mark of a task that no longer exists.
    pub fn forget(&mut self, id: Uuid) -> bool {
        let pinned = self.pinned.remove(&id).is_some();
        let archived = self.archived.remove(&id).is_some();
        let unread = self.unread.remove(&id);
        pinned || archived || unread
    }

    pub fn forget_many(&mut self, ids: &[Uuid]) -> bool {
        ids.iter()
            .fold(false, |changed, id| self.forget(*id) || changed)
    }
}

/// Where the marks live.
pub fn path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join(FILE_NAME))
}

/// The marks on disk; absent or unreadable means none.
pub fn load() -> TaskMarks {
    path().map(|path| load_from(&path)).unwrap_or_default()
}

pub fn load_from(path: &Path) -> TaskMarks {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(marks: &TaskMarks) -> Result<()> {
    let path = path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    save_to(&path, marks)
}

pub fn save_to(path: &Path, marks: &TaskMarks) -> Result<()> {
    let mut encoded = serde_json::to_string_pretty(marks).context("could not encode task marks")?;
    encoded.push('\n');
    atomic_write_private(path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn pinning_twice_or_pinning_an_archived_task_changes_nothing() {
        let mut marks = TaskMarks::default();
        assert!(marks.pin(id(1), 10));
        assert!(!marks.pin(id(1), 20));
        assert_eq!(marks.pinned.get(&id(1)), Some(&10));
        assert!(marks.unpin(id(1)));
        assert!(!marks.unpin(id(1)));

        marks.archive(id(2), 30);
        assert!(!marks.pin(id(2), 40));
        assert!(!marks.is_pinned(id(2)));
    }

    #[test]
    fn archiving_puts_a_task_away_and_unarchiving_brings_it_back_plain() {
        let mut marks = TaskMarks::default();
        marks.pin(id(1), 10);
        marks.mark_unread(id(1));
        assert!(marks.archive(id(1), 20));
        assert!(!marks.archive(id(1), 30));
        assert!(!marks.is_pinned(id(1)));
        assert!(!marks.is_unread(id(1)));
        assert_eq!(marks.archived_at(id(1)), Some(20));

        assert!(marks.unarchive(id(1)));
        assert!(!marks.unarchive(id(1)));
        assert_eq!(marks, TaskMarks::default());
    }

    #[test]
    fn an_archived_task_cannot_be_marked_unread() {
        let mut marks = TaskMarks::default();
        marks.archive(id(1), 10);
        assert!(!marks.mark_unread(id(1)));
        assert!(marks.mark_unread(id(2)));
        assert!(!marks.mark_unread(id(2)));
        assert!(marks.mark_read(id(2)));
        assert!(!marks.mark_read(id(2)));
    }

    #[test]
    fn forgetting_drops_every_mark_of_a_task() {
        let mut marks = TaskMarks::default();
        marks.pin(id(1), 10);
        marks.mark_unread(id(1));
        marks.archive(id(2), 20);
        marks.mark_unread(id(3));
        assert!(marks.forget(id(1)));
        assert!(!marks.forget(id(1)));
        assert!(marks.forget_many(&[id(2), id(3), id(4)]));
        assert_eq!(marks, TaskMarks::default());
        assert!(!marks.forget_many(&[id(2)]));
    }

    #[test]
    fn marks_round_trip_through_their_file() {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-task-marks-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = dir.join(FILE_NAME);
        let mut marks = TaskMarks::default();
        marks.pin(id(1), 10);
        marks.archive(id(2), 20);
        marks.mark_unread(id(3));
        save_to(&path, &marks).unwrap();
        assert_eq!(load_from(&path), marks);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_damaged_file_reads_as_no_marks() {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-task-marks-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = dir.join(FILE_NAME);
        assert_eq!(load_from(&path), TaskMarks::default());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_from(&path), TaskMarks::default());
        // A file from a build that knew fewer kinds of mark still loads.
        std::fs::write(
            &path,
            r#"{"pinned":{"00000000-0000-0000-0000-000000000001":5}}"#,
        )
        .unwrap();
        let loaded = load_from(&path);
        assert!(loaded.is_pinned(id(1)));
        assert!(loaded.archived.is_empty() && loaded.unread.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_build_flavour_keeps_its_own_file() {
        if cfg!(debug_assertions) {
            assert_eq!(FILE_NAME, "task-marks-debug.json");
        } else {
            assert_eq!(FILE_NAME, "task-marks.json");
        }
    }
}
