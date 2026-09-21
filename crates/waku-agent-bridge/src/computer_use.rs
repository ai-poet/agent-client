//! Installing the bundled Computer Use skill where the engine looks for it.
//!
//! The engine's `Skill` tool reads flat `<name>.md` files from two places:
//! the project's `.claurst/commands/` and the engine's own config directory.
//! It does not read `SKILL.md` directories, which is the shape the app ships
//! and every CLI expects — so the bundled skill has to be written out in the
//! form this engine can actually find.
//!
//! This writes into the *engine's* configuration directory, which the fork
//! already owns and writes `settings.json` into. It is not the user's own
//! CLI configuration; installing there is a separate, explicit action on the
//! Skills settings page.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use claurst_core::config::Settings;

/// The file name, and so the name the model passes to `Skill`.
const SKILL_NAME: &str = "waku-computer-use";

fn skill_path() -> PathBuf {
    Settings::config_dir()
        .join("commands")
        .join(format!("{SKILL_NAME}.md"))
}

/// Write the bundled skill, returning where it landed.
///
/// Idempotent by content: a session that starts with the same app version
/// rewrites nothing, so the file's mtime stays meaningful and a concurrent
/// reader never sees a truncated document.
pub(crate) fn install_skill(markdown: &str) -> Result<PathBuf> {
    let path = skill_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if existing == markdown {
            return Ok(path);
        }
    }
    let parent = path
        .parent()
        .context("the engine's config directory has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", parent.display()))?;
    write_atomically(&path, markdown)?;
    Ok(path)
}

/// Take the skill away when Computer Use is off.
///
/// Leaving it would put a skill in `Skill list` whose tools are not
/// registered — the model would read instructions for a capability it
/// cannot reach. Concurrent sessions converge because the toggle is global:
/// they all start with the same answer.
pub(crate) fn remove_skill() {
    let path = skill_path();
    if path.exists() {
        if let Err(error) = std::fs::remove_file(&path) {
            tracing::warn!(%error, path = %path.display(), "agent: could not remove the computer-use skill");
        }
    }
}

/// Write through a sibling temporary file so a reader sees the old document
/// or the new one, never a half-written one.
fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let temporary = path.with_extension("md.tmp");
    std::fs::write(&temporary, contents)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    std::fs::rename(&temporary, path).with_context(|| {
        format!(
            "could not move {} into place at {}",
            temporary.display(),
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Settings::config_dir` reads `CLAURST_HOME`, so the test can point it
    /// somewhere disposable. Serialised against the other env-dependent
    /// tests by running them in one test function.
    #[test]
    fn the_skill_installs_idempotently_and_can_be_taken_away() {
        let home = std::env::temp_dir().join(format!("waku-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("temp home");
        // SAFETY: single-threaded within this test; no other thread reads the
        // environment while it is set.
        unsafe { std::env::set_var("CLAURST_HOME", &home) };

        let path = install_skill("# desktop control").expect("install");
        assert!(path.ends_with("commands/waku-computer-use.md") || path.ends_with("commands\\waku-computer-use.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# desktop control");

        // Same content: still there, and no temporary file left behind.
        install_skill("# desktop control").expect("reinstall");
        assert!(!path.with_extension("md.tmp").exists());

        // Changed content replaces it.
        install_skill("# newer").expect("update");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# newer");

        remove_skill();
        assert!(!path.exists());
        // Removing twice is not an error.
        remove_skill();

        unsafe { std::env::remove_var("CLAURST_HOME") };
        let _ = std::fs::remove_dir_all(&home);
    }
}
