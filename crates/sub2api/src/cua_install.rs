//! Unattended installation of the Windows Computer Use driver.
//!
//! On macOS the app ships its own Swift helper inside the bundle. On Windows
//! the native work — window capture, UI Automation, focus-free input — is done
//! by `cua-driver` (<https://github.com/trycua/cua>, MIT), which speaks the
//! same MCP-over-stdio protocol the helper does. It is fetched on demand
//! rather than bundled: the feature is opt-in and the driver is ~27 MB.
//!
//! The version is pinned and the archive is checksummed before extraction.
//! Every upstream release is flagged prerelease and the API moves quickly, so
//! a floating "latest" would break users between our own releases. Bumping
//! the pin means replacing the constants below with the new release's asset
//! digests (the GitHub releases API reports them) and re-running the Windows
//! hand-test matrix in `docs/windows.md`.
//!
//! Upstream's own `install.ps1` is deliberately not used: it is unpinned
//! remote script execution, and it registers a `cua-driver-serve` scheduled
//! task this app does not want — the driver is started per session by the
//! REPL and stops with it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::brand;
use crate::cli_detect;
use crate::cli_install::{self, InstallOutcome};
use crate::mcp_stdio::McpStdioClient;
use crate::node_install;

/// The pinned upstream version.
pub const CUA_DRIVER_VERSION: &str = "0.23.2";
/// The GitHub release tag that version was published under.
pub const CUA_DRIVER_TAG: &str = "cua-driver-rs-v0.23.2";
/// The driver executable's file name.
pub const CUA_DRIVER_EXECUTABLE: &str = "cua-driver.exe";
/// An explicit driver executable, overriding every other lookup.
pub const CUA_DRIVER_ENV: &str = "WAKU_CUA_DRIVER";

const GITHUB_RELEASE_BASE: &str = "https://github.com/trycua/cua/releases/download";

/// One release archive and the SHA-256 GitHub reports for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverAsset {
    pub name: &'static str,
    pub sha256: &'static str,
}

const WINDOWS_X86_64: DriverAsset = DriverAsset {
    name: "cua-driver-rs-0.23.2-windows-x86_64.zip",
    sha256: "acb0e44ba75ccc2669186665182a01b9517a71a82e81625b4f9f555b455e7a05",
};

const WINDOWS_ARM64: DriverAsset = DriverAsset {
    name: "cua-driver-rs-0.23.2-windows-arm64.zip",
    sha256: "6592ff3042855b1ddeae1facb60d8f464fa2385301f2b67256740fd102333f83",
};

/// The archive for the running architecture.
pub fn windows_asset() -> DriverAsset {
    if cfg!(target_arch = "aarch64") {
        WINDOWS_ARM64
    } else {
        WINDOWS_X86_64
    }
}

/// Where `asset` is fetched from: the upstream release, which is the only
/// copy there is.
///
/// This used to try the app's own release bucket first, so a network that
/// cannot reach GitHub had a second chance. Nothing was ever published
/// there, which made that attempt a guaranteed round trip to a 404 and one
/// more misleading line in every failure report. Mirroring the two archives
/// under `cua-driver/{CUA_DRIVER_TAG}/` and putting the bucket back ahead of
/// this is all it would take to restore it.
pub fn download_url(asset: &DriverAsset) -> String {
    format!("{GITHUB_RELEASE_BASE}/{CUA_DRIVER_TAG}/{}", asset.name)
}

/// Where an install currently is. Reported to the UI as it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CuaStage {
    ResolvingDownload,
    Downloading,
    Installing,
    Verifying,
}

/// Whether this build can install the driver unattended at all.
pub fn install_supported() -> bool {
    cfg!(target_os = "windows")
}

/// App-managed toolchain directory for the driver, versioned so a pin bump
/// installs beside the previous copy rather than over it.
pub fn managed_driver_dir() -> Option<PathBuf> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    Some(
        brand::data_dir()?
            .join("toolchains")
            .join(format!("cua-driver-{CUA_DRIVER_VERSION}-win-{arch}")),
    )
}

/// The driver executable inside the managed install.
pub fn managed_driver_path() -> Option<PathBuf> {
    managed_driver_dir().map(|directory| directory.join(CUA_DRIVER_EXECUTABLE))
}

/// Where upstream's own installer puts the driver, honoured so a user who
/// already installed it is not asked to download it again.
fn official_install_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())?;
    Some(
        PathBuf::from(local)
            .join("Programs")
            .join("Cua")
            .join("cua-driver")
            .join("bin")
            .join(CUA_DRIVER_EXECUTABLE),
    )
}

/// The driver executable to run, if any: the `WAKU_CUA_DRIVER` override
/// (which wins outright, even when it points nowhere), then the managed
/// install, then upstream's install location, then `PATH`.
pub fn resolve_driver() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os(CUA_DRIVER_ENV).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(configured);
        return path.is_file().then_some(path);
    }
    managed_driver_path()
        .filter(|path| path.is_file())
        .or_else(|| official_install_path().filter(|path| path.is_file()))
        .or_else(|| {
            cli_detect::find_executable_in("cua-driver", &cli_detect::default_search_dirs())
        })
}

/// A driver that was found, and what it says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriverDetection {
    pub path: PathBuf,
    /// The bare semantic version, when the driver could be asked.
    pub version: Option<String>,
}

impl DriverDetection {
    /// Whether this is exactly the pinned version.
    pub fn is_pinned_version(&self) -> bool {
        self.version.as_deref() == Some(CUA_DRIVER_VERSION)
    }
}

/// Find the driver and ask it for its version.
///
/// Blocking — it runs the driver. Keep it off the UI thread.
pub fn detect_driver() -> Option<DriverDetection> {
    let path = resolve_driver()?;
    let version = driver_version(&path);
    Some(DriverDetection { path, version })
}

/// `cua-driver --version`, falling back to the version the MCP handshake
/// reports when the flag is not understood.
fn driver_version(path: &Path) -> Option<String> {
    let dirs = cli_detect::default_search_dirs();
    if let Some(version) = cli_detect::probe_version(path, &dirs, cli_detect::PROBE_TIMEOUT)
        .version()
        .and_then(cli_install::parse_semantic_version)
    {
        return Some(version);
    }
    let deadline = Some(Instant::now() + Duration::from_secs(15));
    let client = McpStdioClient::spawn(
        path,
        &["mcp", "--direct"],
        &[],
        "waku",
        "Computer Use driver",
        deadline,
    )
    .ok()?;
    client
        .server_info()
        .get("version")
        .and_then(serde_json::Value::as_str)
        .and_then(cli_install::parse_semantic_version)
}

/// Download, verify and install the pinned driver into the managed
/// directory, reporting stages as they start.
///
/// Blocking — run it off the UI thread. A checksum mismatch aborts before
/// anything is extracted, and a failed install leaves no partial copy.
pub fn install_driver(mut report: impl FnMut(CuaStage)) -> InstallOutcome {
    if !install_supported() {
        return InstallOutcome {
            success: false,
            output: "the Computer Use driver is only installed automatically on Windows".to_owned(),
        };
    }
    report(CuaStage::ResolvingDownload);
    let asset = windows_asset();
    let Some(install_dir) = managed_driver_dir() else {
        return failed(vec!["no home directory".to_owned()]);
    };
    let staging = match node_install::staging_dir() {
        Ok(staging) => staging,
        Err(error) => return failed(vec![format!("{error:#}")]),
    };
    let archive = staging.join(asset.name);

    let mut failures = Vec::new();
    let source = download_url(&asset);
    report(CuaStage::Downloading);
    if let Err(error) = node_install::download(&source, &archive, &mut |_| {}) {
        failures.push(format!("{source}: {error:#}"));
        let _ = std::fs::remove_dir_all(&staging);
        return failed(failures);
    }

    report(CuaStage::Installing);
    let installed = install_archive(&archive, &staging, &install_dir, &asset);
    let _ = std::fs::remove_dir_all(&staging);
    if let Err(error) = installed {
        failures.push(format!("install: {error:#}"));
        return failed(failures);
    }

    report(CuaStage::Verifying);
    let expected = install_dir.join(CUA_DRIVER_EXECUTABLE);
    match detect_driver() {
        Some(detection) if detection.path == expected && detection.is_pinned_version() => {
            InstallOutcome {
                success: true,
                output: format!("installed cua-driver {CUA_DRIVER_VERSION} from {source}"),
            }
        }
        Some(detection) if detection.path == expected => {
            failures.push(format!(
                "installed, but the driver reports version {} (expected {CUA_DRIVER_VERSION})",
                detection.version.as_deref().unwrap_or("unknown")
            ));
            failed(failures)
        }
        Some(detection) => {
            failures.push(format!(
                "installed to {}, but {} is resolved first",
                expected.display(),
                detection.path.display()
            ));
            failed(failures)
        }
        None => {
            failures.push("installed, but the driver did not answer".to_owned());
            failed(failures)
        }
    }
}

/// Verify the archive's checksum, then expand it and move the directory
/// holding `cua-driver.exe` (with whatever workers and libraries sit beside
/// it) into `install_dir`. One PowerShell invocation, like the Node zip path.
fn install_archive(
    archive: &Path,
    staging: &Path,
    install_dir: &Path,
    asset: &DriverAsset,
) -> Result<()> {
    // The gap between the download landing and this running is where an
    // antivirus scanner takes it: the archive carries an unsigned executable
    // that injects input and captures the screen, which is what a remote
    // access trojan looks like from a heuristic's point of view. Saying so
    // here is the difference between an actionable message and a PowerShell
    // stack trace about a path that does not exist.
    if !archive.is_file() {
        return Err(anyhow!(
            "the downloaded archive is no longer at {} — an antivirus scanner most likely removed it; allow the file or install the driver manually",
            archive.display()
        ));
    }
    let script = format!(
        // Windows PowerShell encodes a redirected stream with the console
        // code page, so a localized error reaches us as mojibake unless the
        // stream is told to be UTF-8 first.
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
         $ErrorActionPreference='Stop'; \
         $hash = (Get-FileHash -LiteralPath {zip} -Algorithm SHA256).Hash; \
         if ($hash -ne {sha}) {{ throw ('checksum mismatch: expected ' + {sha} + ' but the download hashed to ' + $hash) }}; \
         $extract = Join-Path {staging} 'extract'; \
         Expand-Archive -LiteralPath {zip} -DestinationPath $extract -Force; \
         $exe = Get-ChildItem -LiteralPath $extract -Recurse -File -Filter {exe} | \
             Select-Object -First 1; \
         if (-not $exe) {{ throw ({exe} + ' was not in the archive') }}; \
         if (Test-Path {install}) {{ Remove-Item -LiteralPath {install} -Recurse -Force }}; \
         New-Item -ItemType Directory -Force (Split-Path {install}) | Out-Null; \
         Copy-Item -LiteralPath $exe.DirectoryName -Destination {install} -Recurse",
        zip = ps_quote(&archive.to_string_lossy()),
        sha = ps_quote(asset.sha256),
        staging = ps_quote(&staging.to_string_lossy()),
        exe = ps_quote(CUA_DRIVER_EXECUTABLE),
        install = ps_quote(&install_dir.to_string_lossy()),
    );
    let outcome = cli_install::run_program(
        "powershell.exe",
        &[
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ],
    );
    if !outcome.success {
        return Err(anyhow!("{}", outcome.output));
    }
    Ok(())
}

/// Quote a value for embedding in a single-quoted PowerShell string.
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn failed(failures: Vec<String>) -> InstallOutcome {
    InstallOutcome {
        success: false,
        output: failures.join("\n\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_carry_full_sha256_digests() {
        for asset in [WINDOWS_X86_64, WINDOWS_ARM64] {
            assert_eq!(asset.sha256.len(), 64, "{}", asset.name);
            assert!(asset.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(asset.name.contains(CUA_DRIVER_VERSION));
            assert!(asset.name.ends_with(".zip"));
        }
        assert!(CUA_DRIVER_TAG.ends_with(CUA_DRIVER_VERSION));
    }

    #[test]
    fn the_download_comes_from_the_pinned_upstream_release() {
        for asset in [WINDOWS_X86_64, WINDOWS_ARM64] {
            let url = download_url(&asset);
            assert_eq!(
                url,
                format!("{GITHUB_RELEASE_BASE}/{CUA_DRIVER_TAG}/{}", asset.name)
            );
            // The tag and the file name both carry the version, and a
            // mismatch between them would download the wrong archive and
            // fail the checksum rather than saying so.
            assert!(url.contains(CUA_DRIVER_VERSION), "{url}");
        }
    }

    #[test]
    fn managed_directory_is_versioned_and_under_the_app_data_dir() {
        let Some(directory) = managed_driver_dir() else {
            return;
        };
        let name = directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with(&format!("cua-driver-{CUA_DRIVER_VERSION}-win-")));
        assert!(directory.starts_with(brand::data_dir().unwrap().join("toolchains")));
        assert_eq!(
            managed_driver_path().unwrap().file_name().unwrap(),
            CUA_DRIVER_EXECUTABLE
        );
    }

    #[test]
    fn pinned_version_check_compares_the_bare_semver() {
        let detection = DriverDetection {
            path: PathBuf::from("cua-driver.exe"),
            version: Some(CUA_DRIVER_VERSION.to_owned()),
        };
        assert!(detection.is_pinned_version());
        let stale = DriverDetection {
            version: Some("0.8.3".to_owned()),
            ..detection.clone()
        };
        assert!(!stale.is_pinned_version());
        let unknown = DriverDetection {
            version: None,
            ..detection
        };
        assert!(!unknown.is_pinned_version());
    }

    /// The archive going missing between the download and the extract is
    /// what an antivirus scanner does to this file, and it used to surface as
    /// a PowerShell error about a path that does not exist. It is reported
    /// before PowerShell is ever started.
    #[test]
    fn a_vanished_archive_is_named_rather_than_handed_to_powershell() {
        let staging = std::env::temp_dir().join(format!("cua-install-test-{}", std::process::id()));
        let missing = staging.join("cua-driver-rs-0.0.0-windows-x86_64.zip");
        let error = install_archive(&missing, &staging, &staging.join("out"), &windows_asset())
            .expect_err("a missing archive cannot be installed");
        let text = format!("{error:#}");
        assert!(text.contains("no longer at"), "{text}");
        assert!(text.contains("antivirus"), "{text}");
    }

    #[test]
    fn install_script_embeds_quoted_paths_and_the_digest() {
        assert_eq!(ps_quote("C:\\it's here"), "'C:\\it''s here'");
    }
}
