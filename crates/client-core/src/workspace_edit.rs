//! Fork addition: the Files panel's edits — create, rename and move to the
//! recycle bin — run on the daemon host like every other workspace file
//! operation, so they work on a remote workspace too.
//!
//! Every path is resolved through [`super::resolve_workspace_path`], so
//! nothing can name a file outside the workspace, and nothing here replaces
//! a file or folder that is already there.

use std::fs::{self, OpenOptions};
use std::path::Path;

use anyhow::{Context as _, bail};

use super::resolve_workspace_path;

pub(super) fn create_file(root: &Path, relative: &Path) -> anyhow::Result<()> {
    let path = resolve_workspace_path(root, relative)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("could not create {}", relative.display()))?;
    Ok(())
}

pub(super) fn create_directory(root: &Path, relative: &Path) -> anyhow::Result<()> {
    let path = resolve_workspace_path(root, relative)?;
    if path.exists() {
        bail!("{} already exists", relative.display());
    }
    fs::create_dir_all(&path).with_context(|| format!("could not create {}", relative.display()))
}

pub(super) fn rename_path(root: &Path, from: &Path, to: &Path) -> anyhow::Result<()> {
    let source = resolve_workspace_path(root, from)?;
    let target = resolve_workspace_path(root, to)?;
    if !source.exists() {
        bail!("{} no longer exists", from.display());
    }
    if target.starts_with(&source) && target != source {
        bail!("cannot move {} into itself", from.display());
    }
    // A change of case only is a rename on case-insensitive filesystems,
    // where the target "exists" because it is the source.
    let same_entry = same_file(&source, &target);
    if target.exists() && !same_entry {
        bail!("{} already exists", to.display());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    fs::rename(&source, &target)
        .with_context(|| format!("could not rename {} to {}", from.display(), to.display()))
}

pub(super) fn trash_path(root: &Path, relative: &Path) -> anyhow::Result<()> {
    let path = resolve_workspace_path(root, relative)?;
    if !path.exists() {
        bail!("{} no longer exists", relative.display());
    }
    trash::delete(&path)
        .with_context(|| format!("could not move {} to the recycle bin", relative.display()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::*;

    fn workspace() -> PathBuf {
        let root = std::env::temp_dir().join(format!("waku-workspace-edit-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn creating_never_replaces_what_is_there() {
        let root = workspace();
        create_file(&root, Path::new("src/new.rs")).unwrap();
        assert!(root.join("src/new.rs").is_file());
        fs::write(root.join("src/new.rs"), "kept").unwrap();
        assert!(create_file(&root, Path::new("src/new.rs")).is_err());
        assert_eq!(fs::read_to_string(root.join("src/new.rs")).unwrap(), "kept");

        create_directory(&root, Path::new("docs/guide")).unwrap();
        assert!(root.join("docs/guide").is_dir());
        assert!(create_directory(&root, Path::new("docs/guide")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renaming_refuses_an_existing_target_and_moving_into_itself() {
        let root = workspace();
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        assert!(rename_path(&root, Path::new("a.txt"), Path::new("b.txt")).is_err());
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "b");

        rename_path(&root, Path::new("a.txt"), Path::new("nested/c.txt")).unwrap();
        assert!(!root.join("a.txt").exists());
        assert_eq!(fs::read_to_string(root.join("nested/c.txt")).unwrap(), "a");

        assert!(rename_path(&root, Path::new("nested"), Path::new("nested/inner")).is_err());
        assert!(rename_path(&root, Path::new("missing"), Path::new("other")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nothing_outside_the_workspace_can_be_named() {
        let root = workspace();
        for path in ["../escape.txt", "", "/absolute.txt"] {
            assert!(create_file(&root, Path::new(path)).is_err(), "{path}");
            assert!(create_directory(&root, Path::new(path)).is_err(), "{path}");
            assert!(trash_path(&root, Path::new(path)).is_err(), "{path}");
        }
        fs::write(root.join("in.txt"), "").unwrap();
        assert!(rename_path(&root, Path::new("in.txt"), Path::new("../out.txt")).is_err());
        assert!(root.join("in.txt").exists());
        assert!(trash_path(&root, Path::new("missing.txt")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
