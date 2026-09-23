//! Which shell the Bash tool runs commands in, and how to say so.
//!
//! Fork addition (Waku). Upstream ran the Bash tool through `cmd /C` on
//! Windows while the system prompt described the shell as PowerShell, asked
//! for Unix syntax, and then asked for `dir` and `type` — three shells, none
//! of them the one executing, so `ls | head`, `2>/dev/null` and
//! `Get-ChildItem` all failed alike. Models are trained on bash, and Claude
//! Code and Pi both run their Bash tool through Git Bash on Windows; so does
//! this engine now, whenever Git for Windows is installed. The prompt and the
//! tool description read the same answer from here, so they cannot disagree
//! with the executor again.

use std::path::{Path, PathBuf};

/// The variable Claude Code reads for the same purpose. Honoured so a user who
/// already pointed Claude Code at their bash does not have to say it twice.
pub const GIT_BASH_PATH_ENV: &str = "CLAUDE_CODE_GIT_BASH_PATH";

/// What executes a Bash tool call on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashToolShell {
    /// `bash` on macOS and Linux, through a PTY.
    Bash,
    /// Git for Windows' bash, without a PTY.
    GitBash,
    /// `cmd.exe`, because this Windows machine has no bash.
    Cmd,
}

/// The shell behind the Bash tool here.
pub fn bash_tool_shell() -> BashToolShell {
    if !cfg!(windows) {
        BashToolShell::Bash
    } else if windows_bash().is_some() {
        BashToolShell::GitBash
    } else {
        BashToolShell::Cmd
    }
}

/// Git Bash's `bash.exe`, when one is installed. Always `None` off Windows.
///
/// Resolved once per process: installing Git for Windows is picked up at the
/// next restart, which is also when Claude Code would notice it.
pub fn windows_bash() -> Option<&'static Path> {
    #[cfg(windows)]
    {
        static BASH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        BASH.get_or_init(|| find_windows_bash(&WindowsLookup::from_env(), |path| path.is_file()))
            .as_deref()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Where to look for bash on Windows, separated from the environment so the
/// order can be tested anywhere.
#[derive(Debug, Clone, Default)]
pub struct WindowsLookup {
    /// [`GIT_BASH_PATH_ENV`], when set.
    pub explicit: Option<PathBuf>,
    /// Install roots holding a `Git` directory: `%ProgramFiles%`,
    /// `%ProgramFiles(x86)%`, and the per-user `%LOCALAPPDATA%\Programs`.
    pub install_roots: Vec<PathBuf>,
    /// The `PATH` directories, in order.
    pub path_dirs: Vec<PathBuf>,
}

impl WindowsLookup {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let mut install_roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(|name| var(name).map(PathBuf::from))
            .collect();
        if let Some(local) = var("LOCALAPPDATA") {
            install_roots.push(PathBuf::from(local).join("Programs"));
        }
        Self {
            explicit: var(GIT_BASH_PATH_ENV).map(PathBuf::from),
            install_roots,
            path_dirs: var("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
        }
    }
}

/// The resolution order, following Pi's and Claude Code's: an explicit path;
/// Git for Windows in its usual install locations; the bash beside a `git.exe`
/// on `PATH` (Git installed somewhere else); then any other `bash.exe` on
/// `PATH` (MSYS2, Cygwin, a Scoop shim).
///
/// Never the WSL launcher in `System32`: it runs commands inside a Linux VM
/// that cannot see the Windows paths this session is working in.
pub fn find_windows_bash(lookup: &WindowsLookup, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(explicit) = lookup.explicit.as_ref().filter(|path| exists(path)) {
        return Some(explicit.clone());
    }
    for root in &lookup.install_roots {
        let candidate = root.join("Git").join("bin").join("bash.exe");
        if exists(&candidate) {
            return Some(candidate);
        }
    }
    // `git.exe` sits in `<root>\cmd`, `<root>\bin` or `<root>\mingw64\bin`;
    // the bash that sets up Git's environment is `<root>\bin\bash.exe`.
    for dir in &lookup.path_dirs {
        if !exists(&dir.join("git.exe")) {
            continue;
        }
        for root in dir.ancestors().skip(1).take(2) {
            let candidate = root.join("bin").join("bash.exe");
            if exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    lookup
        .path_dirs
        .iter()
        .map(|dir| dir.join("bash.exe"))
        .find(|candidate| exists(candidate) && !is_wsl_launcher(candidate))
}

/// The WSL `bash.exe` Windows puts in `System32` (or its 32-bit view), and the
/// Store alias under `WindowsApps`.
fn is_wsl_launcher(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    normalized.contains("\\windows\\system32\\")
        || normalized.contains("\\windows\\sysnative\\")
        || normalized.contains("\\windowsapps\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn exists_in(files: &[&str]) -> impl Fn(&Path) -> bool {
        let files: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        move |path: &Path| files.contains(path)
    }

    fn lookup(explicit: Option<&str>, roots: &[&str], path: &[&str]) -> WindowsLookup {
        WindowsLookup {
            explicit: explicit.map(PathBuf::from),
            install_roots: roots.iter().map(PathBuf::from).collect(),
            path_dirs: path.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn the_standard_install_is_found() {
        let found = find_windows_bash(
            &lookup(None, &["C:/Program Files"], &[]),
            exists_in(&["C:/Program Files/Git/bin/bash.exe"]),
        );
        assert_eq!(found, Some(PathBuf::from("C:/Program Files/Git/bin/bash.exe")));
    }

    #[test]
    fn an_explicit_path_wins_and_a_missing_one_is_ignored() {
        let files = ["D:/tools/bash.exe", "C:/Program Files/Git/bin/bash.exe"];
        let explicit = find_windows_bash(
            &lookup(Some("D:/tools/bash.exe"), &["C:/Program Files"], &[]),
            exists_in(&files),
        );
        assert_eq!(explicit, Some(PathBuf::from("D:/tools/bash.exe")));

        let stale = find_windows_bash(
            &lookup(Some("D:/gone/bash.exe"), &["C:/Program Files"], &[]),
            exists_in(&files),
        );
        assert_eq!(stale, Some(PathBuf::from("C:/Program Files/Git/bin/bash.exe")));
    }

    /// Git installed somewhere unusual is found through its `git.exe`.
    #[test]
    fn bash_is_found_beside_git_on_the_path() {
        let found = find_windows_bash(
            &lookup(None, &[], &["E:/dev/Git/cmd"]),
            exists_in(&["E:/dev/Git/cmd/git.exe", "E:/dev/Git/bin/bash.exe"]),
        );
        assert_eq!(found, Some(PathBuf::from("E:/dev/Git/bin/bash.exe")));

        let mingw = find_windows_bash(
            &lookup(None, &[], &["E:/dev/Git/mingw64/bin"]),
            exists_in(&["E:/dev/Git/mingw64/bin/git.exe", "E:/dev/Git/bin/bash.exe"]),
        );
        assert_eq!(mingw, Some(PathBuf::from("E:/dev/Git/bin/bash.exe")));
    }

    /// The WSL launcher would run the command inside a Linux VM that cannot
    /// see the session's Windows paths.
    #[test]
    fn the_wsl_launcher_is_never_used() {
        let found = find_windows_bash(
            &lookup(None, &[], &["C:/Windows/System32"]),
            exists_in(&["C:/Windows/System32/bash.exe"]),
        );
        assert_eq!(found, None);

        let msys = find_windows_bash(
            &lookup(None, &[], &["C:/Windows/System32", "C:/msys64/usr/bin"]),
            exists_in(&["C:/Windows/System32/bash.exe", "C:/msys64/usr/bin/bash.exe"]),
        );
        assert_eq!(msys, Some(PathBuf::from("C:/msys64/usr/bin/bash.exe")));
    }

    #[test]
    fn nothing_installed_finds_nothing() {
        assert_eq!(
            find_windows_bash(&lookup(None, &["C:/Program Files"], &["C:/bin"]), exists_in(&[])),
            None
        );
    }

    #[test]
    fn off_windows_the_bash_tool_runs_bash() {
        if !cfg!(windows) {
            assert_eq!(bash_tool_shell(), BashToolShell::Bash);
            assert!(windows_bash().is_none());
        }
    }
}
