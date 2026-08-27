//! One-shot startup migration of legacy upstream-named storage.
//!
//! The fork renamed every storage location that carried upstream's name:
//! `~/.waku` → `~/.cheaprouter`, `~/.config/waku` → `~/.config/cheaprouter`,
//! and the platform data/cache folders `Waku` / `Waku Debug` →
//! `CheapRouter` / `CheapRouter Debug`. Each pair is a same-parent rename,
//! attempted once per process start before anything opens files.
//!
//! A rename only happens when the old directory exists and the new one does
//! not: if both exist the new one wins and the old is left untouched for the
//! user to inspect, and a failed rename (locked files, permissions) is
//! reported but never blocks startup — it is simply retried next launch.

use std::path::{Path, PathBuf};

use crate::brand;

/// Upstream's home dot-directory.
const LEGACY_DATA_DIR_NAME: &str = ".waku";

/// Upstream's platform data/cache folder renames (release, debug). Keep the
/// new names in sync with `waku_protocol::identity::DATA_DIRECTORY_NAME`.
const PLATFORM_DIR_RENAMES: &[(&str, &str)] = &[
    ("Waku", "CheapRouter"),
    ("Waku Debug", "CheapRouter Debug"),
];

/// Rename every legacy storage directory to its branded name. Returns
/// human-readable warnings for renames that were needed but failed; callers
/// log them and continue.
pub fn migrate_legacy_storage() -> Vec<String> {
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        pairs.push((
            home.join(LEGACY_DATA_DIR_NAME),
            home.join(brand::DATA_DIR_NAME),
        ));
        // User-global slash commands (`composer_complete` also keeps scanning
        // the legacy location, so a failed rename loses nothing).
        pairs.push((
            home.join(".config").join("waku"),
            home.join(".config").join("cheaprouter"),
        ));
    }
    // data_local_dir: app.db, updater.json, WebView2 profile (Windows/Linux).
    // data_dir: the Computer Use helper install (macOS Application Support,
    // same as data_local_dir there). cache_dir: the model catalog cache.
    for root in [dirs::data_local_dir(), dirs::data_dir(), dirs::cache_dir()]
        .into_iter()
        .flatten()
    {
        for (old, new) in PLATFORM_DIR_RENAMES {
            pairs.push((root.join(old), root.join(new)));
        }
    }

    let mut warnings = Vec::new();
    for (old, new) in pairs {
        rename_pair(&old, &new, &mut warnings);
    }
    warnings
}

fn rename_pair(old: &Path, new: &Path, warnings: &mut Vec<String>) {
    if !old.is_dir() || new.exists() {
        return;
    }
    if let Err(error) = std::fs::rename(old, new) {
        warnings.push(format!(
            "could not migrate {} to {}: {error}",
            old.display(),
            new.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sub2api-migrate-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn renames_legacy_directory_with_contents() {
        let root = temp_root("rename");
        let old = root.join(".waku");
        std::fs::create_dir_all(old.join("projects")).unwrap();
        std::fs::write(old.join("settings.json"), b"{}").unwrap();

        let mut warnings = Vec::new();
        rename_pair(&old, &root.join(".cheaprouter"), &mut warnings);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!old.exists());
        assert!(root.join(".cheaprouter/projects").is_dir());
        assert_eq!(
            std::fs::read(root.join(".cheaprouter/settings.json")).unwrap(),
            b"{}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_new_directory_wins_and_legacy_is_left_alone() {
        let root = temp_root("both");
        let old = root.join(".waku");
        let new = root.join(".cheaprouter");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("marker"), b"old").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("marker"), b"new").unwrap();

        let mut warnings = Vec::new();
        rename_pair(&old, &new, &mut warnings);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(std::fs::read(old.join("marker")).unwrap(), b"old");
        assert_eq!(std::fs::read(new.join("marker")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn absent_legacy_directory_is_a_no_op() {
        let root = temp_root("absent");
        let mut warnings = Vec::new();
        rename_pair(&root.join(".waku"), &root.join(".cheaprouter"), &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!root.join(".cheaprouter").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
