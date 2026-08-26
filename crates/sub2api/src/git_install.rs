//! Silent Git for Windows installation, ported from the Electron client.
//!
//! Why Git belongs in the setup flow at all:
//!
//! * Task checkpoints and branch operations shell out to a bare `git`; without
//!   it the rewind feature — a headline capability — silently has nothing to
//!   work with.
//! * Claude Code's mature shell tool is Git Bash. Since 2.x it falls back to a
//!   PowerShell tool when bash is missing, but that fallback is marked
//!   *preview* by Anthropic's own binary, and any skill or hook declaring
//!   `shell: bash` fails outright.
//!
//! Install strategy, in the old client's order:
//!
//! 1. **PortableGit** self-extracting 7z from the npmmirror listing into an
//!    app-managed directory — no elevation, no installer UI.
//! 2. The full **Git for Windows installer** from the mirror, silent with
//!    `/CURRENTUSER` so it installs per-user without elevation.
//! 3. **winget** (`Git.Git`).
//! 4. The **GitHub direct** latest installer, for machines that can reach
//!    github.com but not the mirror.
//!
//! Windows-only by design: macOS ships git via the Xcode command line tools
//! prompt and Linux via the package manager.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::brand;
use crate::cli_install;
use crate::node_install::{self, MirrorEntry, NodeStage};

/// npmmirror directory of Git for Windows releases.
pub const GIT_MIRROR_LISTING_URL: &str =
    "https://registry.npmmirror.com/-/binary/git-for-windows/";

/// Fallback when the mirror is unreachable but GitHub is not.
pub const GIT_DIRECT_INSTALLER_URL: &str =
    "https://github.com/git-for-windows/git/releases/latest/download/Git-64-bit.exe";

/// Whether this build installs Git unattended (Windows x64 only, matching the
/// published asset names).
pub fn install_supported() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
}

/// App-managed PortableGit directory.
pub fn managed_git_dir() -> Option<PathBuf> {
    Some(brand::data_dir()?.join("toolchains").join("portable-git"))
}

/// Locations where `git.exe`'s directory may be, existing ones only.
///
/// Fed into the spawned-process `PATH` so the daemon's bare `git` calls work
/// as soon as an install finishes.
pub fn git_path_dirs() -> Vec<PathBuf> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let Some(managed) = managed_git_dir() {
        candidates.push(managed.join("cmd"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs").join("Git").join("cmd"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Git\cmd"));
    candidates.into_iter().filter(|dir| dir.is_dir()).collect()
}

/// Locate Git Bash, avoiding WSL's `System32\bash.exe`, which is not a POSIX
/// shell in the sense Claude Code needs.
///
/// Order matches the old client: an explicit override, then the managed
/// PortableGit, then the per-user and machine installs.
pub fn find_git_bash() -> Option<PathBuf> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    if let Some(configured) = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH") {
        let configured = PathBuf::from(configured);
        if configured.is_file() {
            return Some(configured);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(managed) = managed_git_dir() {
        candidates.push(managed.join("bin").join("bash.exe"));
        candidates.push(managed.join("usr").join("bin").join("bash.exe"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let programs = PathBuf::from(local).join("Programs").join("Git");
        candidates.push(programs.join("bin").join("bash.exe"));
        candidates.push(programs.join("usr").join("bin").join("bash.exe"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"));
    candidates.into_iter().find(|path| path.is_file())
}

/// `git --version` from `PATH` or any known install location.
pub fn detect_git() -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(on_path) = cli_install::find_executable("git") {
        candidates.push(on_path);
    }
    for dir in git_path_dirs() {
        candidates.push(dir.join("git.exe"));
    }
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let outcome = cli_install::run_program(&candidate, &["--version"]);
        if outcome.success && !outcome.output.trim().is_empty() {
            return Some(outcome.output.trim().to_owned());
        }
    }
    None
}

/// Whether the machine is complete for our purposes: a runnable `git` *and* a
/// real Git Bash. The full installer provides both; WSL or a bare MinGit could
/// satisfy one without the other.
pub fn git_is_complete() -> bool {
    if !cfg!(target_os = "windows") {
        // Elsewhere git alone is the requirement; bash is always present.
        return detect_git().is_some();
    }
    detect_git().is_some() && find_git_bash().is_some()
}

/// Install Git for Windows unattended, reporting stages as they start.
///
/// Blocking — run it off the UI thread.
pub fn install_git(mut report: impl FnMut(NodeStage)) -> cli_install::InstallOutcome {
    if !install_supported() {
        return cli_install::InstallOutcome {
            success: false,
            output: "automatic Git installation is only supported on Windows x64; \
                     install Git with your package manager"
                .to_owned(),
        };
    }

    let mut failures: Vec<String> = Vec::new();

    match install_portable(&mut report) {
        Ok(output) => return verified(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("portable: {error:#}")),
    }
    match install_from_installer_url(resolve_installer_url(&mut report), &mut report) {
        Ok(output) => return verified(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("installer: {error:#}")),
    }
    match install_winget(&mut report) {
        Ok(output) => return verified(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("winget: {error:#}")),
    }
    match install_from_installer_url(Ok(GIT_DIRECT_INSTALLER_URL.to_owned()), &mut report) {
        Ok(output) => return verified(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("github: {error:#}")),
    }
    cli_install::InstallOutcome {
        success: false,
        output: failures.join("\n\n"),
    }
}

fn verified(
    output: String,
    report: &mut impl FnMut(NodeStage),
    failures: &mut Vec<String>,
) -> cli_install::InstallOutcome {
    report(NodeStage::Verifying);
    if git_is_complete() {
        let version = detect_git().unwrap_or_default();
        return cli_install::InstallOutcome {
            success: true,
            output: format!("{output}\n{version}").trim().to_owned(),
        };
    }
    failures.push("installed, but git or Git Bash still did not answer".to_owned());
    cli_install::InstallOutcome {
        success: false,
        output: std::mem::take(failures).join("\n\n"),
    }
}

// --- release listing -----------------------------------------------------

/// `v2.51.0.windows.1/` → `(2, 51, 0, 1)`.
///
/// The `windows.N` re-release counter participates in ordering: `windows.2`
/// supersedes `windows.1` of the same version.
pub fn parse_release_dir(name: &str) -> Option<(u64, u64, u64, u64)> {
    let rest = name.trim_end_matches('/').strip_prefix('v')?;
    let (version, revision) = rest.split_once(".windows.")?;
    let mut parts = version.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let revision: u64 = revision.trim_end_matches('/').parse().ok()?;
    Some((major, minor, patch, revision))
}

/// URL of the newest release directory in the mirror listing.
pub fn latest_release_dir(entries: &[MirrorEntry]) -> Option<String> {
    entries
        .iter()
        .filter(|entry| entry.is_dir && !entry.url.is_empty())
        .filter_map(|entry| Some((parse_release_dir(&entry.name)?, &entry.url)))
        .max_by_key(|(version, _)| *version)
        .map(|(_, url)| url.clone())
}

/// Pick an asset such as `PortableGit-2.51.0-64-bit.7z.exe` out of a release
/// directory listing.
pub fn release_asset_url(entries: &[MirrorEntry], prefix: &str, suffix: &str) -> Option<String> {
    entries
        .iter()
        .filter(|entry| !entry.is_dir && !entry.url.is_empty())
        .find(|entry| {
            let name = entry.name.as_str();
            name.starts_with(prefix)
                && name.ends_with(suffix)
                // The middle must be a version, not e.g. "-rc1" prereleases.
                && name[prefix.len()..name.len() - suffix.len()]
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
        })
        .map(|entry| entry.url.clone())
}

fn latest_release_entries(report: &mut impl FnMut(NodeStage)) -> Result<Vec<MirrorEntry>> {
    report(NodeStage::ResolvingDownload);
    let releases = node_install::fetch_listing_at(GIT_MIRROR_LISTING_URL)?;
    let directory = latest_release_dir(&releases)
        .ok_or_else(|| anyhow!("the mirror listing had no Git releases"))?;
    node_install::fetch_listing_at(&directory)
}

fn resolve_installer_url(report: &mut impl FnMut(NodeStage)) -> Result<String> {
    let entries = latest_release_entries(report)?;
    release_asset_url(&entries, "Git-", "-64-bit.exe")
        .ok_or_else(|| anyhow!("the release had no 64-bit installer"))
}

// --- the installers ------------------------------------------------------

/// Silent-install switches for the Git for Windows installer.
///
/// `/CURRENTUSER` is the load-bearing one: it installs per-user, which is what
/// lets the silent mode succeed without an elevation prompt it cannot show.
const INSTALLER_ARGS: &[&str] = &[
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/SP-",
    "/NOCANCEL",
    "/CURRENTUSER",
    "/CLOSEAPPLICATIONS",
    "/RESTARTAPPLICATIONS",
    "/o:PathOption=Cmd",
    "/o:BashTerminalOption=MinTTY",
];

fn install_portable(report: &mut impl FnMut(NodeStage)) -> Result<String> {
    let entries = latest_release_entries(report)?;
    let url = release_asset_url(&entries, "PortableGit-", "-64-bit.7z.exe")
        .ok_or_else(|| anyhow!("the release had no PortableGit archive"))?;

    let staging = node_install::staging_dir()?;
    let archive = staging.join("portable-git.7z.exe");
    node_install::download(&url, &archive, report)?;

    report(NodeStage::Installing { method: "portable" });
    let install_dir = managed_git_dir().ok_or_else(|| anyhow!("no home directory"))?;
    let _ = std::fs::remove_dir_all(&install_dir);
    if let Some(parent) = install_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The archive is a 7-Zip self-extractor; -gm2 suppresses its GUI. The
    // -InstallPath value doubles its backslashes, matching the old client.
    let install_path_argument = format!(
        "-InstallPath={}",
        install_dir.to_string_lossy().replace('\\', "\\\\")
    );
    let outcome = cli_install::run_program(
        &archive,
        &["-y", "-gm2", &install_path_argument],
    );
    let _ = std::fs::remove_dir_all(&staging);
    if !outcome.success {
        return Err(anyhow!("extraction failed: {}", outcome.output));
    }
    Ok(format!("installed PortableGit from {url}"))
}

fn install_from_installer_url(
    url: Result<String>,
    report: &mut impl FnMut(NodeStage),
) -> Result<String> {
    let url = url?;
    let staging = node_install::staging_dir()?;
    let installer = staging.join("git-installer.exe");
    node_install::download(&url, &installer, report)?;

    report(NodeStage::Installing { method: "installer" });
    let outcome = cli_install::run_program(&installer, INSTALLER_ARGS);
    let _ = std::fs::remove_dir_all(&staging);
    if !outcome.success {
        return Err(anyhow!("the installer failed: {}", outcome.output));
    }
    Ok(format!("installed Git from {url}"))
}

fn install_winget(report: &mut impl FnMut(NodeStage)) -> Result<String> {
    if cli_install::find_executable("winget").is_none() {
        return Err(anyhow!("winget is not available"));
    }
    report(NodeStage::Installing { method: "winget" });
    let outcome = cli_install::run_program(
        "winget",
        &[
            "install",
            "--id",
            "Git.Git",
            "-e",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ],
    );
    if !outcome.success {
        return Err(anyhow!("winget failed: {}", outcome.output));
    }
    Ok("installed Git via winget".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_entry(name: &str) -> MirrorEntry {
        MirrorEntry {
            name: name.to_owned(),
            is_dir: true,
            url: format!("https://mirror.example/git/{name}"),
        }
    }

    fn file_entry(name: &str) -> MirrorEntry {
        MirrorEntry {
            name: name.to_owned(),
            is_dir: false,
            url: format!("https://mirror.example/git/v/{name}"),
        }
    }

    #[test]
    fn parses_release_directory_names() {
        assert_eq!(parse_release_dir("v2.51.0.windows.1/"), Some((2, 51, 0, 1)));
        assert_eq!(parse_release_dir("v2.51.0.windows.2"), Some((2, 51, 0, 2)));
        assert_eq!(parse_release_dir("v2.9.5.windows.10/"), Some((2, 9, 5, 10)));
        assert_eq!(parse_release_dir("docs/"), None);
        assert_eq!(parse_release_dir("v2.51.windows.1/"), None);
        assert_eq!(parse_release_dir("v2.51.0-rc1.windows.1/"), None);
    }

    #[test]
    fn newest_release_wins_numerically_including_the_rerelease_counter() {
        let entries = vec![
            dir_entry("v2.9.5.windows.1/"),
            dir_entry("v2.51.0.windows.1/"),
            dir_entry("v2.51.0.windows.2/"),
            dir_entry("v2.10.0.windows.1/"),
            file_entry("README.md"),
        ];
        assert_eq!(
            latest_release_dir(&entries),
            Some("https://mirror.example/git/v2.51.0.windows.2/".to_owned())
        );
    }

    #[test]
    fn picks_the_right_assets_out_of_a_release() {
        let entries = vec![
            file_entry("Git-2.51.0-64-bit.exe"),
            file_entry("Git-2.51.0-arm64.exe"),
            file_entry("PortableGit-2.51.0-64-bit.7z.exe"),
            file_entry("MinGit-2.51.0-64-bit.zip"),
            dir_entry("something/"),
        ];
        assert_eq!(
            release_asset_url(&entries, "Git-", "-64-bit.exe"),
            Some("https://mirror.example/git/v/Git-2.51.0-64-bit.exe".to_owned())
        );
        assert_eq!(
            release_asset_url(&entries, "PortableGit-", "-64-bit.7z.exe"),
            Some("https://mirror.example/git/v/PortableGit-2.51.0-64-bit.7z.exe".to_owned())
        );
        // A prerelease-style middle must not match.
        let prerelease = vec![file_entry("Git-2.52.0-rc1-64-bit.exe")];
        assert_eq!(release_asset_url(&prerelease, "Git-", "-64-bit.exe"), None);
    }

    #[test]
    fn installer_args_stay_per_user_and_silent() {
        assert!(INSTALLER_ARGS.contains(&"/CURRENTUSER"));
        assert!(INSTALLER_ARGS.contains(&"/VERYSILENT"));
        assert!(INSTALLER_ARGS.contains(&"/NORESTART"));
    }

    #[test]
    fn unsupported_platforms_fail_cleanly() {
        if !install_supported() {
            let outcome = install_git(|_| {});
            assert!(!outcome.success);
            assert!(outcome.output.contains("package manager"));
        }
    }

    #[test]
    fn managed_dir_lives_under_the_toolchains_root() {
        let dir = managed_git_dir().expect("dir");
        let text = dir.to_string_lossy().replace('\\', "/");
        assert!(text.ends_with("/toolchains/portable-git"), "{text}");
    }
}
