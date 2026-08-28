//! Version compatibility for the Codex CLI.
//!
//! Codex releases fast and its `app-server` surface is labeled experimental.
//! Older releases speak stdio implicitly and reject a `--stdio` flag with
//! `unexpected argument '--stdio' found`; newer ones (0.148+ verified) grew a
//! `--listen` transport selector with `--stdio` as its shorthand. Upstream
//! hardcodes the flag for new Codex, which breaks every older install. The
//! only version-proof source of truth is the binary itself: ask
//! `app-server --help` once and remember the answer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

/// Arguments that start the app server on the given `codex` binary.
pub fn app_server_args(binary: &Path) -> &'static [&'static str] {
    if supports_stdio_flag(binary) {
        &["app-server", "--stdio"]
    } else {
        &["app-server"]
    }
}

/// Config overrides applied to every Codex session this app starts.
///
/// `node_repl` is Codex's experimental code-mode, backed by a separate
/// `codex-code-mode-host` process. On Windows installs that host frequently
/// fails to launch (`MCP startup failed … os error 3`), which surfaces as an
/// error inside every session — for a tool that is redundant here anyway: the
/// agent already has a full shell, and upstream itself disables `node_repl`
/// whenever it substitutes its own REPL.
///
/// The stub `command` is never executed — `enabled=false` keeps the entry
/// from spawning — but it must be present: newer Codex validates every
/// `mcp_servers` table as a transport and rejects one that has neither
/// `command` nor `url` with `invalid transport`, which kills config loading
/// (and with it every session) outright.
pub fn session_config_args() -> &'static [&'static str] {
    &[
        "-c",
        "mcp_servers.node_repl.command=\"/usr/bin/true\"",
        "-c",
        "mcp_servers.node_repl.enabled=false",
    ]
}

/// Whether this binary's `app-server` accepts `--stdio`, cached per path.
///
/// When the probe itself fails the answer is `false`: a bare `app-server` is
/// valid on every version that has the subcommand at all (stdio is the
/// default transport throughout), while the flag is only valid on newer ones —
/// so omitting it is the safe direction. A binary too broken to print help
/// will fail at spawn anyway, where the driver's own error reporting is
/// better than ours.
fn supports_stdio_flag(binary: &Path) -> bool {
    static CACHE: Mutex<Option<HashMap<PathBuf, bool>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(&known) = cache.get(binary) {
        return known;
    }
    let supported = probe_help_mentions_stdio(binary);
    cache.insert(binary.to_owned(), supported);
    supported
}

fn probe_help_mentions_stdio(binary: &Path) -> bool {
    let mut command = std::process::Command::new(binary);
    command
        .args(["app-server", "--help"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // The full help text, not a tail: the flag can sit anywhere in it.
    match command.output() {
        Ok(output) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            text.contains("--stdio")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_that_cannot_run_defaults_to_flagless() {
        // Recent Codex takes no flag, so a failed probe must not add one.
        assert!(!supports_stdio_flag(Path::new("/definitely/not/a/codex")));
    }

    #[test]
    fn session_config_disables_the_builtin_node_repl() {
        let args = session_config_args();
        // The disabled entry still carries a stub command: newer Codex
        // rejects a transport-less `mcp_servers` table as `invalid transport`.
        assert_eq!(
            args,
            [
                "-c",
                "mcp_servers.node_repl.command=\"/usr/bin/true\"",
                "-c",
                "mcp_servers.node_repl.enabled=false",
            ]
        );
    }

    #[test]
    fn the_answer_is_cached() {
        let bogus = Path::new("/definitely/not/a/codex");
        assert!(!supports_stdio_flag(bogus));
        // Second call must come from the cache; identical answer either way.
        assert!(!supports_stdio_flag(bogus));
        assert_eq!(app_server_args(bogus), &["app-server"]);
    }
}
