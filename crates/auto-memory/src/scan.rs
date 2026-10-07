//! Every memory on disk, grouped by scope, for the settings page.

use std::path::{Path, PathBuf};

use crate::store::{INDEX_FILE, MemoryStore, PROJECTS_DIR, is_stale};
use crate::types::{MemoryRecord, MemoryScope, ScopeDir};

/// One scope's memories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryGroup {
    pub dir: ScopeDir,
    /// The project directory's key (its folder name); `None` for the user
    /// scope.
    pub key: Option<String>,
    /// Pinned first, then by name — the index order.
    pub memories: Vec<MemoryRecord>,
    /// Size of the scope's `MEMORY.md`.
    pub index_bytes: usize,
    /// Memories left out of the index as stale.
    pub hidden: usize,
}

impl MemoryGroup {
    pub fn scope(&self) -> MemoryScope {
        self.dir.scope
    }

    /// The workspace a project group belongs to, when known.
    pub fn project_path(&self) -> Option<&Path> {
        self.dir.cwd.as_deref()
    }
}

/// The user group first (when it has anything), then every project with
/// memories, ordered by workspace path.
pub fn scan_groups(store: &MemoryStore) -> Vec<MemoryGroup> {
    let now = crate::now_ms();
    let read_group = |dir: ScopeDir, key: Option<String>| {
        let mut memories = store.list(&dir);
        memories.sort_by(|left, right| {
            right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| left.name.cmp(&right.name))
        });
        let hidden = memories
            .iter()
            .filter(|record| is_stale(record, store.stale_after_days(), now))
            .count();
        let index_bytes = std::fs::metadata(dir.dir.join(INDEX_FILE))
            .map(|metadata| metadata.len() as usize)
            .unwrap_or(0);
        MemoryGroup {
            dir,
            key,
            memories,
            index_bytes,
            hidden,
        }
    };

    let mut groups = Vec::new();
    let user = read_group(store.user_dir(), None);
    if !user.memories.is_empty() {
        groups.push(user);
    }
    let mut projects = std::fs::read_dir(store.root().join(PROJECTS_DIR))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|key| read_group(store.project_dir_by_key(&key), Some(key)))
        .filter(|group| !group.memories.is_empty())
        .collect::<Vec<_>>();
    projects.sort_by_key(|group| {
        group
            .dir
            .cwd
            .clone()
            .unwrap_or_else(|| PathBuf::from(group.key.clone().unwrap_or_default()))
    });
    groups.extend(projects);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryDraft, MemoryType};

    #[test]
    fn groups_list_user_first_then_projects_by_path() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path());
        let draft = |name: &str| MemoryDraft {
            name: name.into(),
            title: None,
            description: "d".into(),
            kind: MemoryType::User,
            body: "b".into(),
            pinned: None,
        };
        store.write(&store.project_dir(Path::new("/z/app")), draft("z")).unwrap();
        store.write(&store.project_dir(Path::new("/a/app")), draft("a")).unwrap();
        store.write(&store.user_dir(), draft("me")).unwrap();
        let groups = scan_groups(&store);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].scope(), MemoryScope::User);
        assert_eq!(groups[1].project_path(), Some(Path::new("/a/app")));
        assert_eq!(groups[2].project_path(), Some(Path::new("/z/app")));
        assert!(groups[1].index_bytes > 0);
    }
}
