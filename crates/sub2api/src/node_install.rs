//! Silent Node.js installation, ported from the Electron client.
//!
//! The target user starts with nothing installed; if Node cannot be put on the
//! machine unattended, nothing else in the setup flow means anything. The
//! strategy is the old client's, in the same order:
//!
//! * **Windows**: portable zip into an app-managed toolchain directory (no
//!   elevation needed), falling back to the mirror MSI (elevated machines),
//!   falling back to winget. The managed directory is added to the user's
//!   persisted `PATH` and to every process this app spawns.
//! * **macOS**: official tarball from the mirror into the managed directory.
//! * **Linux**: not automated — distro package managers own Node there, same
//!   as the old client (`cli.automaticNodeUnsupported`).
//!
//! Download URLs come from npmmirror's JSON directory listing for
//! `latest-v22.x`, picking the highest version present. The mirror is the
//! primary because that is what the audience can actually reach; the assets on
//! it are byte-identical mirrors of nodejs.org.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::brand;
use crate::cli_install::{self, InstallOutcome, REQUIRED_NODE_MAJOR};
use crate::http;

/// npmmirror directory listing for the newest v22 assets.
pub const NODE_MIRROR_LISTING_URL: &str =
    "https://registry.npmmirror.com/-/binary/node/latest-v22.x/";

/// Where an install currently is. Reported to the UI as it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStage {
    ResolvingDownload,
    Downloading,
    /// `method` is `"zip"`, `"msi"`, `"winget"`, or `"tarball"`.
    Installing { method: &'static str },
    Verifying,
}

/// Whether this build can install Node unattended at all.
pub fn install_supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

/// App-managed toolchain directory for the Node runtime.
///
/// Same shape as the old client's `<home>/toolchains/node22-win-x64`, under
/// this app's own data directory so uninstalling the app leaves nothing
/// orphaned in a shared location.
pub fn managed_node_dir() -> Option<PathBuf> {
    let platform = if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    Some(
        brand::data_dir()?
            .join("toolchains")
            .join(format!("node22-{platform}-{arch}")),
    )
}

/// Directory holding the `node` executable inside the managed install.
///
/// Windows zips are flat; Unix tarballs put binaries under `bin/`.
pub fn managed_node_bin_dir() -> Option<PathBuf> {
    let root = managed_node_dir()?;
    Some(if cfg!(target_os = "windows") {
        root
    } else {
        root.join("bin")
    })
}

/// `node --version` from anywhere we know to look: the process `PATH`, the
/// managed runtime, the MSI's install location, and every version-manager
/// directory [`crate::cli_detect::detection_dirs`] enumerates.
///
/// Detection must see what installation produced, in the same app session,
/// or a successful install would still render as "not found".
pub fn detect_node() -> Option<String> {
    detect_node_in(&crate::cli_detect::default_search_dirs())
}

/// [`detect_node`] over an explicit directory list — the desktop passes the
/// login-shell-aware list waku-core maintains.
pub fn detect_node_in(dirs: &[PathBuf]) -> Option<String> {
    crate::cli_detect::probe_named("node", dirs, dirs)
        .version()
        .map(str::to_owned)
}

/// `npm --version` from the same set of places [`detect_node`] looks.
///
/// The old client's runtime check reported both (`node -v && npm -v`): a Node
/// without a working npm cannot install any agent, so showing only the Node
/// version would claim health the machine does not have.
pub fn detect_npm() -> Option<String> {
    let dirs = crate::cli_detect::default_search_dirs();
    crate::cli_detect::probe_named(cli_install::npm_binary_name(), &dirs, &dirs)
        .version()
        .map(str::to_owned)
}

/// Install Node 22 unattended, reporting stages as they start.
///
/// Blocking — run it off the UI thread. Fallbacks accumulate their failures so
/// "the zip route and the MSI route failed differently" stays diagnosable.
pub fn install_node(mut report: impl FnMut(NodeStage)) -> InstallOutcome {
    if !install_supported() {
        return InstallOutcome {
            success: false,
            output: "automatic Node installation is not supported on this platform; \
                     install Node 22 with your package manager"
                .to_owned(),
        };
    }

    report(NodeStage::ResolvingDownload);
    let listing = match fetch_listing() {
        Ok(listing) => listing,
        Err(error) => {
            return InstallOutcome {
                success: false,
                output: format!("could not read the Node download listing: {error:#}"),
            };
        }
    };

    let mut failures: Vec<String> = Vec::new();

    if cfg!(target_os = "macos") {
        match install_macos(&listing, &mut report) {
            Ok(output) => return verified_outcome(output, &mut report, &mut failures),
            Err(error) => failures.push(format!("tarball: {error:#}")),
        }
        return failed(failures);
    }

    // Windows: portable zip first — it needs no elevation, which the MSI's
    // silent mode cannot ask for.
    match install_windows_zip(&listing, &mut report) {
        Ok(output) => return verified_outcome(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("zip: {error:#}")),
    }
    match install_windows_msi(&listing, &mut report) {
        Ok(output) => return verified_outcome(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("msi: {error:#}")),
    }
    match install_windows_winget(&mut report) {
        Ok(output) => return verified_outcome(output, &mut report, &mut failures),
        Err(error) => failures.push(format!("winget: {error:#}")),
    }
    failed(failures)
}

/// Confirm the install actually produced a usable Node before declaring
/// success; a truthful "verified v22.x" beats an optimistic "done".
fn verified_outcome(
    output: String,
    report: &mut impl FnMut(NodeStage),
    failures: &mut Vec<String>,
) -> InstallOutcome {
    report(NodeStage::Verifying);
    match detect_node() {
        Some(version) if cli_install::node_is_supported(&version) => InstallOutcome {
            success: true,
            output: format!("{output}\n{version}").trim().to_owned(),
        },
        Some(version) => {
            failures.push(format!(
                "installed, but `node --version` reports {version} (need {REQUIRED_NODE_MAJOR}+)"
            ));
            failed(std::mem::take(failures))
        }
        None => {
            failures.push("installed, but node did not answer `--version`".to_owned());
            failed(std::mem::take(failures))
        }
    }
}

fn failed(failures: Vec<String>) -> InstallOutcome {
    InstallOutcome {
        success: false,
        output: failures.join("\n\n"),
    }
}

// --- download listing ---------------------------------------------------

/// One row of npmmirror's directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorEntry {
    pub name: String,
    pub is_dir: bool,
    pub url: String,
}

fn fetch_listing() -> Result<Vec<MirrorEntry>> {
    fetch_listing_at(NODE_MIRROR_LISTING_URL)
}

/// Fetch and parse any npmmirror directory listing.
pub(crate) fn fetch_listing_at(url: &str) -> Result<Vec<MirrorEntry>> {
    let response = http::Request::new().timeout_seconds(30).send(url)?;
    if !response.is_success() {
        return Err(anyhow!(
            "the mirror listing answered with status {}",
            response.status
        ));
    }
    parse_listing(&response.body)
}

/// Parse the listing JSON: an array of `{name, type, url}` objects.
pub fn parse_listing(body: &str) -> Result<Vec<MirrorEntry>> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).context("the mirror listing was not JSON")?;
    let entries = parsed
        .as_array()
        .ok_or_else(|| anyhow!("the mirror listing was not a JSON array"))?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            Some(MirrorEntry {
                name: entry.get("name")?.as_str()?.to_owned(),
                is_dir: entry.get("type").and_then(|t| t.as_str()) == Some("dir"),
                url: entry.get("url").and_then(|u| u.as_str()).unwrap_or("").to_owned(),
            })
        })
        .collect())
}

/// `node-v22.11.0-win-x64.zip` → `(22, 11, 0)`, checking prefix and suffix.
fn parse_asset_version(name: &str, suffix: &str) -> Option<(u64, u64, u64)> {
    let rest = name.strip_prefix("node-v")?;
    let version_text = rest.strip_suffix(suffix)?;
    let mut parts = version_text.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || major != u64::from(REQUIRED_NODE_MAJOR) {
        return None;
    }
    Some((major, minor, patch))
}

/// URL of the newest asset whose name ends with `suffix`.
pub fn latest_asset_url(entries: &[MirrorEntry], suffix: &str) -> Option<String> {
    entries
        .iter()
        .filter(|entry| !entry.is_dir && !entry.url.is_empty())
        .filter_map(|entry| Some((parse_asset_version(&entry.name, suffix)?, &entry.url)))
        .max_by_key(|(version, _)| *version)
        .map(|(_, url)| url.clone())
}

fn windows_zip_suffix() -> &'static str {
    if cfg!(target_arch = "aarch64") { "-win-arm64.zip" } else { "-win-x64.zip" }
}

fn windows_msi_suffix() -> &'static str {
    if cfg!(target_arch = "aarch64") { "-arm64.msi" } else { "-x64.msi" }
}

// Referenced only by the macOS installer, which is cfg-gated.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn darwin_tarball_suffix() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "-darwin-arm64.tar.gz"
    } else {
        "-darwin-x64.tar.gz"
    }
}

// --- the platform installers --------------------------------------------

/// A fresh staging directory of its own for one install.
///
/// It lives under the app's own `toolchains/.staging` rather than the system
/// temp dir, and every call gets a new one. A file that curl had just
/// written once vanished from a per-process `%TEMP%` folder before it could
/// be checked: temp cleaners sweep `%TEMP%` and nothing else, and a folder
/// shared by every install in the process let one install's cleanup take
/// another's download with it.
pub(crate) fn staging_dir() -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let root = brand::data_dir()
        .map(|data| data.join("toolchains").join(".staging"))
        .unwrap_or_else(|| std::env::temp_dir().join("toolchain-install"));
    let dir = root.join(name);
    // A previous process with the same id may have died mid-install.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    Ok(dir)
}

/// How many times a download whose file is gone afterwards is fetched again.
const DOWNLOAD_ATTEMPTS: u32 = 3;

/// Prefix of the line curl's `-w` prints, so it can be found among errors.
const WRITE_OUT_PREFIX: &str = "download-result: ";

pub(crate) fn download(
    url: &str,
    destination: &Path,
    report: &mut impl FnMut(NodeStage),
) -> Result<()> {
    report(NodeStage::Downloading);
    let write_out = format!("{WRITE_OUT_PREFIX}%{{http_code}} %{{size_download}} %{{filename_effective}}\\n");
    download_with(url, destination, std::time::Duration::from_secs(1), || {
        cli_install::run_program(
            http::CURL_PATH,
            &[
                "-fsSL",
                "--connect-timeout",
                "20",
                "--retry",
                "2",
                "--retry-delay",
                "1",
                "-w",
                &write_out,
                "-o",
                &destination.to_string_lossy(),
                url,
            ],
        )
    })
}

/// [`download`] with the fetch injected, so the landing check and its retry
/// can be tested without a network.
fn download_with(
    url: &str,
    destination: &Path,
    pause: std::time::Duration,
    mut fetch: impl FnMut() -> InstallOutcome,
) -> Result<()> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let outcome = fetch();
        if !outcome.success {
            // curl's own `--retry` has already covered the network.
            return Err(anyhow!("download failed: {}", outcome.output));
        }
        // A zero exit is not proof the bytes are still there: curl creates
        // the file even for an empty body, yet the archive has been seen to
        // be gone a moment after curl exited. Checking here turns that into
        // another attempt, and finally into a sentence naming the download
        // rather than a missing-path error two steps later.
        match std::fs::metadata(destination) {
            Ok(metadata) if metadata.len() > 0 => return Ok(()),
            Ok(_) => return Err(anyhow!("{url} answered with an empty file")),
            Err(_) if attempts < DOWNLOAD_ATTEMPTS => {
                std::thread::sleep(pause);
                // The whole folder may have gone with it, and curl does not
                // create directories.
                if let Some(parent) = destination.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            Err(error) => {
                return Err(VanishedDownload {
                    url: url.to_owned(),
                    path: destination.to_path_buf(),
                    attempts,
                    error,
                    write_out: parse_write_out(&outcome.output),
                    folder: FolderState::read(destination.parent()),
                }
                .into());
            }
        }
    }
}

/// What curl's `-w` line reported about the transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WriteOut {
    status: u16,
    bytes: u64,
    filename: String,
}

/// Find and parse the `-w` line among curl's captured output.
fn parse_write_out(output: &str) -> Option<WriteOut> {
    let line = output
        .lines()
        .find_map(|line| line.trim().strip_prefix(WRITE_OUT_PREFIX))?;
    let mut parts = line.splitn(3, ' ');
    let status = parts.next()?.parse().ok()?;
    let bytes = parts.next()?.parse().ok()?;
    let filename = parts.next().unwrap_or("").trim().to_owned();
    Some(WriteOut {
        status,
        bytes,
        filename,
    })
}

/// What was left in the download's folder once the file was found missing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FolderState {
    Gone,
    Holds(Vec<String>),
}

impl FolderState {
    const LISTED: usize = 10;

    fn read(folder: Option<&Path>) -> Self {
        let Some(entries) = folder.and_then(|folder| std::fs::read_dir(folder).ok()) else {
            return Self::Gone;
        };
        let mut names: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Self::Holds(names)
    }
}

/// curl exited 0, yet the file was not there afterwards, on every attempt.
#[derive(Debug)]
struct VanishedDownload {
    url: String,
    path: PathBuf,
    attempts: u32,
    error: std::io::Error,
    write_out: Option<WriteOut>,
    folder: FolderState,
}

impl std::fmt::Display for VanishedDownload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} reported success", self.url)?;
        if let Some(write_out) = &self.write_out {
            write!(
                formatter,
                " (HTTP {}, {} bytes written to {})",
                write_out.status, write_out.bytes, write_out.filename
            )?;
        }
        write!(
            formatter,
            " but nothing was at {} afterwards ({}), {} attempt{}; ",
            self.path.display(),
            self.error,
            self.attempts,
            if self.attempts == 1 { "" } else { "s" }
        )?;
        match &self.folder {
            FolderState::Gone => write!(formatter, "the folder itself is gone"),
            FolderState::Holds(names) if names.is_empty() => {
                write!(formatter, "the folder is empty")
            }
            FolderState::Holds(names) => {
                let shown = &names[..names.len().min(FolderState::LISTED)];
                write!(formatter, "the folder holds: {}", shown.join(", "))?;
                if names.len() > shown.len() {
                    write!(formatter, " and {} more", names.len() - shown.len())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for VanishedDownload {}

#[cfg(target_os = "macos")]
fn install_macos(entries: &[MirrorEntry], report: &mut impl FnMut(NodeStage)) -> Result<String> {
    let url = latest_asset_url(entries, darwin_tarball_suffix())
        .ok_or_else(|| anyhow!("the mirror listing had no macOS tarball"))?;
    let staging = staging_dir()?;
    let archive = staging.join("node22.tar.gz");
    download(&url, &archive, report)?;

    report(NodeStage::Installing { method: "tarball" });
    let install_dir = managed_node_dir().ok_or_else(|| anyhow!("no home directory"))?;
    let _ = std::fs::remove_dir_all(&install_dir);
    std::fs::create_dir_all(&install_dir)
        .with_context(|| format!("could not create {}", install_dir.display()))?;
    let outcome = cli_install::run_program(
        "/usr/bin/tar",
        &[
            "-xzf",
            &archive.to_string_lossy(),
            "--strip-components",
            "1",
            "-C",
            &install_dir.to_string_lossy(),
        ],
    );
    let _ = std::fs::remove_dir_all(&staging);
    if !outcome.success {
        return Err(anyhow!("extraction failed: {}", outcome.output));
    }
    Ok(format!("installed Node from {url}"))
}

#[cfg(not(target_os = "macos"))]
fn install_macos(_entries: &[MirrorEntry], _report: &mut impl FnMut(NodeStage)) -> Result<String> {
    Err(anyhow!("not macOS"))
}

/// Quote a value for embedding in a single-quoted PowerShell string.
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn install_windows_zip(entries: &[MirrorEntry], report: &mut impl FnMut(NodeStage)) -> Result<String> {
    if !cfg!(target_os = "windows") {
        return Err(anyhow!("not Windows"));
    }
    let url = latest_asset_url(entries, windows_zip_suffix())
        .ok_or_else(|| anyhow!("the mirror listing had no Windows zip"))?;
    let staging = staging_dir()?;
    let archive = staging.join("node22.zip");
    download(&url, &archive, report)?;

    report(NodeStage::Installing { method: "zip" });
    let install_dir = managed_node_dir().ok_or_else(|| anyhow!("no home directory"))?;
    // Expand into staging, find the directory that contains node.exe (the
    // archive nests everything under node-v22.x-win-x64/), then move it into
    // the managed location. Mirrors the old client's PowerShell step.
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         Expand-Archive -LiteralPath {zip} -DestinationPath {staging} -Force; \
         $dir = Get-ChildItem -LiteralPath {staging} -Directory | \
             Where-Object {{ Test-Path (Join-Path $_.FullName 'node.exe') }} | \
             Select-Object -First 1; \
         if (-not $dir) {{ throw 'node.exe was not in the archive' }}; \
         if (Test-Path {install}) {{ Remove-Item -LiteralPath {install} -Recurse -Force }}; \
         New-Item -ItemType Directory -Force (Split-Path {install}) | Out-Null; \
         Copy-Item -LiteralPath $dir.FullName -Destination {install} -Recurse",
        zip = ps_quote(&archive.to_string_lossy()),
        staging = ps_quote(&staging.to_string_lossy()),
        install = ps_quote(&install_dir.to_string_lossy()),
    );
    let outcome = cli_install::run_program(
        "powershell.exe",
        &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script],
    );
    let _ = std::fs::remove_dir_all(&staging);
    if !outcome.success {
        return Err(anyhow!("extraction failed: {}", outcome.output));
    }

    // Persist the managed directory on the user PATH and broadcast the change,
    // so the user's own terminals see node too — not just processes this app
    // spawns with its augmented PATH.
    persist_windows_user_path(&install_dir);
    Ok(format!("installed Node from {url}"))
}

fn install_windows_msi(entries: &[MirrorEntry], report: &mut impl FnMut(NodeStage)) -> Result<String> {
    if !cfg!(target_os = "windows") {
        return Err(anyhow!("not Windows"));
    }
    let url = latest_asset_url(entries, windows_msi_suffix())
        .ok_or_else(|| anyhow!("the mirror listing had no Windows MSI"))?;
    let staging = staging_dir()?;
    let installer = staging.join("node22.msi");
    download(&url, &installer, report)?;

    report(NodeStage::Installing { method: "msi" });
    // /qn is fully silent, which also means it cannot raise a UAC prompt: on a
    // non-elevated session this exits 1603/1625, and the caller moves on.
    let outcome = cli_install::run_program(
        "msiexec.exe",
        &["/i", &installer.to_string_lossy(), "/qn", "/norestart"],
    );
    let _ = std::fs::remove_dir_all(&staging);
    if !outcome.success {
        return Err(anyhow!("msiexec failed: {}", outcome.output));
    }
    Ok(format!("installed Node from {url}"))
}

fn install_windows_winget(report: &mut impl FnMut(NodeStage)) -> Result<String> {
    if !cfg!(target_os = "windows") {
        return Err(anyhow!("not Windows"));
    }
    if cli_install::find_executable("winget").is_none() {
        return Err(anyhow!("winget is not available"));
    }
    report(NodeStage::Installing { method: "winget" });
    let outcome = cli_install::run_program(
        "winget",
        &[
            "install",
            "--id",
            "OpenJS.NodeJS.LTS",
            "-e",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ],
    );
    if !outcome.success {
        return Err(anyhow!("winget failed: {}", outcome.output));
    }
    Ok("installed Node via winget".to_owned())
}

/// Append `directory` to the persisted user `PATH` and broadcast the change.
///
/// Best-effort: a failure here only affects the user's own future terminals,
/// not this app, which injects the directory into every process it spawns.
pub(crate) fn persist_windows_user_path(directory: &Path) {
    if !cfg!(target_os = "windows") {
        return;
    }
    let script = format!(
        "$entry = {entry}; \
         $current = [Environment]::GetEnvironmentVariable('Path', 'User'); \
         if ($null -eq $current) {{ $current = '' }}; \
         $parts = $current.Split(';') | Where-Object {{ $_ -ne '' }}; \
         if (-not ($parts -contains $entry)) {{ \
             [Environment]::SetEnvironmentVariable('Path', (($current.TrimEnd(';') + ';' + $entry).Trim(';')), 'User'); \
             {broadcast} \
         }}",
        entry = ps_quote(&directory.to_string_lossy()),
        broadcast = cli_install::broadcast_environment_change_command(),
    );
    let _ = cli_install::run_program(
        "powershell.exe",
        &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script],
    );
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn entry(name: &str) -> MirrorEntry {
        MirrorEntry {
            name: name.to_owned(),
            is_dir: false,
            url: format!("https://mirror.example/{name}"),
        }
    }

    #[test]
    fn parses_a_mirror_listing() {
        let body = r#"[
            {"name":"node-v22.11.0-win-x64.zip","type":"file","url":"https://m/a.zip"},
            {"name":"docs/","type":"dir","url":"https://m/docs/"},
            {"name":"SHASUMS256.txt","type":"file"}
        ]"#;
        let entries = parse_listing(body).expect("parse");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "node-v22.11.0-win-x64.zip");
        assert!(entries[1].is_dir);
        // A missing url must not drop the entry into a panic.
        assert_eq!(entries[2].url, "");
        assert!(parse_listing("{\"not\":\"array\"}").is_err());
        assert!(parse_listing("nonsense").is_err());
    }

    #[test]
    fn picks_the_highest_version_of_the_right_asset() {
        let entries = vec![
            entry("node-v22.9.0-win-x64.zip"),
            entry("node-v22.11.0-win-x64.zip"),
            entry("node-v22.10.1-win-x64.zip"),
            entry("node-v22.11.0-x64.msi"),
            entry("node-v22.11.0-darwin-arm64.tar.gz"),
        ];
        assert_eq!(
            latest_asset_url(&entries, "-win-x64.zip"),
            Some("https://mirror.example/node-v22.11.0-win-x64.zip".to_owned())
        );
        assert_eq!(
            latest_asset_url(&entries, "-x64.msi"),
            Some("https://mirror.example/node-v22.11.0-x64.msi".to_owned())
        );
        assert_eq!(
            latest_asset_url(&entries, "-darwin-arm64.tar.gz"),
            Some("https://mirror.example/node-v22.11.0-darwin-arm64.tar.gz".to_owned())
        );
        assert_eq!(latest_asset_url(&entries, "-linux-x64.tar.xz"), None);
    }

    #[test]
    fn version_ordering_is_numeric_not_lexicographic() {
        // 22.9 must lose to 22.10: string comparison would get this wrong.
        let entries = vec![
            entry("node-v22.9.0-win-x64.zip"),
            entry("node-v22.10.0-win-x64.zip"),
        ];
        assert_eq!(
            latest_asset_url(&entries, "-win-x64.zip"),
            Some("https://mirror.example/node-v22.10.0-win-x64.zip".to_owned())
        );
    }

    #[test]
    fn directories_and_other_majors_are_ignored() {
        let mut directory = entry("node-v22.11.0-win-x64.zip");
        directory.is_dir = true;
        let entries = vec![
            directory,
            entry("node-v23.1.0-win-x64.zip"),
            entry("node-v21.7.0-win-x64.zip"),
        ];
        // Only major 22 qualifies, and the dir row is not a download.
        assert_eq!(latest_asset_url(&entries, "-win-x64.zip"), None);
    }

    #[test]
    fn managed_dir_is_platform_and_arch_specific() {
        let dir = managed_node_dir().expect("dir");
        let text = dir.to_string_lossy().replace('\\', "/");
        assert!(text.contains("/toolchains/node22-"), "{text}");
        if cfg!(target_os = "windows") {
            assert!(text.ends_with("win-x64") || text.ends_with("win-arm64"));
            assert_eq!(managed_node_bin_dir(), Some(dir));
        } else {
            assert_eq!(managed_node_bin_dir(), Some(dir.join("bin")));
        }
    }

    #[test]
    fn ps_quoting_doubles_single_quotes() {
        assert_eq!(ps_quote("C:\\a b"), "'C:\\a b'");
        assert_eq!(ps_quote("it's"), "'it''s'");
    }

    fn succeeded(output: &str) -> InstallOutcome {
        InstallOutcome {
            success: true,
            output: output.to_owned(),
        }
    }

    #[test]
    fn every_install_gets_its_own_staging_folder() {
        let first = staging_dir().expect("first");
        let second = staging_dir().expect("second");
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        if let Some(data) = brand::data_dir() {
            let root = data.join("toolchains").join(".staging");
            assert!(first.starts_with(&root), "{}", first.display());
        }
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    /// The archive was once gone a moment after curl exited 0. One more
    /// fetch is what gets the install through when it happens again.
    #[test]
    fn a_download_that_vanishes_is_fetched_again() {
        let staging = staging_dir().expect("staging");
        let archive = staging.join("driver.zip");
        let mut calls = 0;
        download_with("https://example/driver.zip", &archive, Duration::ZERO, || {
            calls += 1;
            if calls == 2 {
                std::fs::write(&archive, b"zip").unwrap();
            }
            succeeded("")
        })
        .expect("the second attempt lands");
        assert_eq!(calls, 2);
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn a_download_that_keeps_vanishing_says_what_curl_saw_and_what_is_left() {
        let staging = staging_dir().expect("staging");
        let archive = staging.join("driver.zip");
        std::fs::write(staging.join("leftover.txt"), b"x").unwrap();
        let mut calls = 0;
        let error = download_with("https://example/driver.zip", &archive, Duration::ZERO, || {
            calls += 1;
            succeeded("download-result: 200 27636131 C:\\a b\\driver.zip\n")
        })
        .expect_err("nothing ever lands");
        assert_eq!(calls, DOWNLOAD_ATTEMPTS);
        let vanished = error.downcast_ref::<VanishedDownload>().expect("typed");
        assert_eq!(vanished.attempts, DOWNLOAD_ATTEMPTS);
        let text = error.to_string();
        assert!(text.contains("HTTP 200, 27636131 bytes written to C:\\a b\\driver.zip"), "{text}");
        assert!(text.contains("3 attempts"), "{text}");
        assert!(text.contains("the folder holds: leftover.txt"), "{text}");
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn a_folder_that_went_with_the_download_is_recreated_and_reported() {
        let staging = staging_dir().expect("staging");
        let archive = staging.join("driver.zip");
        let mut calls = 0;
        let error = download_with("https://example/driver.zip", &archive, Duration::ZERO, || {
            calls += 1;
            // Every later attempt finds the folder back in place.
            if calls > 1 {
                assert!(staging.is_dir(), "attempt {calls} has no folder to write into");
            }
            let _ = std::fs::remove_dir_all(&staging);
            succeeded("")
        })
        .expect_err("nothing ever lands");
        assert_eq!(calls, DOWNLOAD_ATTEMPTS);
        assert!(error.to_string().contains("the folder itself is gone"), "{error}");
    }

    #[test]
    fn empty_files_and_failed_fetches_are_not_retried() {
        let staging = staging_dir().expect("staging");
        let archive = staging.join("driver.zip");
        let mut calls = 0;
        let error = download_with("https://example/driver.zip", &archive, Duration::ZERO, || {
            calls += 1;
            std::fs::write(&archive, b"").unwrap();
            succeeded("")
        })
        .expect_err("empty");
        assert_eq!(calls, 1);
        assert!(error.to_string().contains("empty file"), "{error}");

        calls = 0;
        let error = download_with("https://example/driver.zip", &archive, Duration::ZERO, || {
            calls += 1;
            InstallOutcome {
                success: false,
                output: "curl: (22) The requested URL returned error: 404".to_owned(),
            }
        })
        .expect_err("failed");
        assert_eq!(calls, 1);
        assert!(error.to_string().starts_with("download failed: curl: (22)"), "{error}");
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn parses_the_write_out_line() {
        assert_eq!(
            parse_write_out("download-result: 200 27636131 C:\\Users\\a b\\x.zip\n"),
            Some(WriteOut {
                status: 200,
                bytes: 27636131,
                filename: "C:\\Users\\a b\\x.zip".to_owned(),
            })
        );
        // Found among curl's error lines, and tolerant of CRLF.
        assert_eq!(
            parse_write_out("curl: (22) nope\r\ndownload-result: 404 0 \r\n"),
            Some(WriteOut {
                status: 404,
                bytes: 0,
                filename: String::new(),
            })
        );
        assert_eq!(parse_write_out("download-result: 200"), None);
        assert_eq!(parse_write_out("download-result: x y z"), None);
        assert_eq!(parse_write_out("nonsense"), None);
    }

    #[test]
    fn unsupported_platforms_fail_cleanly() {
        if !install_supported() {
            let outcome = install_node(|_| {});
            assert!(!outcome.success);
            assert!(outcome.output.contains("package manager"));
        }
    }
}
