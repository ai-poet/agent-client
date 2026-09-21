//! What the installed Claude Code can be told to do.
//!
//! Sibling of [`crate::codex_compat`], and the same shape: probe `--help`
//! once per binary, cache the answer, and choose the safe direction when the
//! probe itself fails.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

/// Whether this build accepts `--plugin-dir`, which loads a plugin for one
/// session without touching the user's own configuration.
///
/// Waku uses it to hand Claude Code the bundled Computer Use skill the same
/// way Codex gets it through `skills/extraRoots/set` — nothing is written to
/// `~/.claude`, and the plugin disappears with the session.
///
/// A failed probe answers `false`. An unknown flag is fatal at spawn, so
/// omitting it is the safe direction: the session still starts, and the skill
/// can be installed from the Skills page instead.
pub fn supports_plugin_dir(binary: &Path) -> bool {
    static CACHE: Mutex<Option<HashMap<PathBuf, bool>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(&known) = cache.get(binary) {
        return known;
    }
    let supported = probe_help_mentions(binary, "--plugin-dir");
    cache.insert(binary.to_owned(), supported);
    supported
}

fn probe_help_mentions(binary: &Path, flag: &str) -> bool {
    let mut command = std::process::Command::new(binary);
    command
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.output() {
        Ok(output) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            text.contains(flag)
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unknown flag kills the launch, so a binary that cannot be asked is
    /// assumed not to take one.
    #[test]
    fn a_binary_that_cannot_run_takes_no_plugin_dir() {
        assert!(!supports_plugin_dir(Path::new(
            "/definitely/not/a/real/claude/binary"
        )));
    }
}
