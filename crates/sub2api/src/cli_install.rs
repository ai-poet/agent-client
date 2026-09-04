//! Detecting agent CLIs and assisting with their installation.
//!
//! Upstream expects the user to have installed and authenticated an agent CLI
//! before launching the app. This module closes that gap: it reports what is
//! missing and produces the exact command that fixes it.
//!
//! # Why commands rather than an installer
//!
//! The command is handed to the app's built-in terminal and runs in front of
//! the user. That keeps npm's own output — proxy failures, permission errors,
//! EACCES on a root-owned prefix — visible instead of collapsed into a generic
//! "install failed", and it leaves the user with a command they can rerun or
//! paste into an issue. Everything here is therefore a pure string-producing
//! function, which also makes it testable without touching the machine.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cli_detect::Probe;

/// Node major version the agent CLIs require.
pub const REQUIRED_NODE_MAJOR: u32 = 22;

/// Registry used when the user is likely behind the Great Firewall. npm's
/// default registry is frequently unreachable there, and a stalled install is
/// the single most common setup failure.
pub const MIRROR_REGISTRY: &str = "https://registry.npmmirror.com";

/// An installable agent CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CliDescriptor {
    /// Provider id, matching upstream's `ProviderKind::id()`.
    pub id: &'static str,
    /// Name shown in the UI.
    pub display_name: &'static str,
    /// npm package that provides the CLI.
    pub package: &'static str,
    /// Executable to look for on `PATH`.
    pub binary: &'static str,
}

/// CLIs this build knows how to install.
///
/// Package names match the ones the Electron client ships, so a machine set up
/// by either app looks the same.
pub const DESCRIPTORS: &[CliDescriptor] = &[
    CliDescriptor {
        id: "claude",
        display_name: "Claude Code",
        package: "@anthropic-ai/claude-code",
        binary: "claude",
    },
    CliDescriptor {
        id: "codex",
        display_name: "Codex CLI",
        package: "@openai/codex",
        binary: "codex",
    },
    // The three CLIs above and below are the supported set; OpenCode and Pi
    // installs were dropped deliberately — the app still detects and runs
    // them when the user installs them elsewhere.
    CliDescriptor {
        id: "grok",
        display_name: "Grok Build",
        package: "@xai-official/grok",
        binary: "grok",
    },
];

/// Look up a descriptor by provider id.
pub fn descriptor(id: &str) -> Option<&'static CliDescriptor> {
    DESCRIPTORS.iter().find(|descriptor| descriptor.id == id)
}

/// What we found for one CLI: whether a binary exists and whether it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub id: &'static str,
    pub probe: Probe,
}

impl Detection {
    /// A binary exists, runnable or not. A broken install is still an
    /// install — offering to `npm install` over it would not fix it.
    pub fn is_installed(&self) -> bool {
        self.probe.is_present()
    }

    pub fn is_runnable(&self) -> bool {
        self.probe.is_runnable()
    }

    pub fn path(&self) -> Option<&Path> {
        self.probe.path()
    }
}

/// Detect every known CLI in the default search directories, running each
/// one found. Blocking; see [`crate::cli_detect::snapshot`] for the full pass
/// the desktop uses.
pub fn detect_all() -> Vec<Detection> {
    let dirs = crate::cli_detect::default_search_dirs();
    DESCRIPTORS
        .iter()
        .map(|descriptor| Detection {
            id: descriptor.id,
            probe: crate::cli_detect::probe_named(descriptor.binary, &dirs, &dirs),
        })
        .collect()
}

/// Resolve an executable in the default search directories — the process
/// `PATH` plus every place [`crate::cli_detect::detection_dirs`] knows.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    crate::cli_detect::find_executable_in(name, &crate::cli_detect::default_search_dirs())
}

/// The npm launcher's file name on this platform.
pub fn npm_binary_name() -> &'static str {
    if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" }
}

/// `npm install -g <package>`, optionally pinned to a registry mirror.
///
/// The mirror command carries retry and timeout flags: without them npm's
/// default fetch can hang for minutes on a flaky route, which reads as "the
/// install froze".
pub fn install_command(package: &str, registry: Option<&str>) -> String {
    match registry {
        Some(registry) if !registry.trim().is_empty() => format!(
            "npm install -g {package} --registry={} --fetch-retries=2 --fetch-timeout=60000",
            registry.trim()
        ),
        _ => format!("npm install -g {package}"),
    }
}

/// Install commands to try, in order — the copyable form of
/// [`install_attempts`].
///
/// On Windows the mirror comes first for everyone, with the official registry
/// as the fallback — the audience this ships to reaches npmmirror far more
/// reliably than registry.npmjs.org, and a locale check would miss them
/// because Windows rarely sets `LANG`. Elsewhere the official registry comes
/// first, but the mirror still follows: a mainland macOS user is not rarer
/// than a mainland Windows user, only differently defaulted.
pub fn install_candidates(package: &str) -> Vec<String> {
    let pinned = format!("{package}@latest");
    let mirror = install_command(&pinned, Some(MIRROR_REGISTRY));
    let official = install_command(&pinned, None);
    if cfg!(target_os = "windows") {
        vec![mirror, official]
    } else {
        vec![official, mirror]
    }
}

/// Upper bound on one `npm install` run. npm's own retries live inside it;
/// past this it is stuck, not slow.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// What an install needs to know about the machine it runs on.
#[derive(Clone, Debug)]
pub struct InstallContext {
    /// Directory of the Node that detection found; npm is anchored to it.
    pub node_bin_dir: Option<PathBuf>,
    /// Every directory the app searches for binaries — the child's `PATH`
    /// and the yardstick for "is the result on PATH".
    pub search_dirs: Vec<PathBuf>,
    pub timeout: Duration,
}

impl InstallContext {
    pub fn new(node_bin_dir: Option<PathBuf>, search_dirs: Vec<PathBuf>) -> Self {
        Self {
            node_bin_dir,
            search_dirs,
            timeout: INSTALL_TIMEOUT,
        }
    }

    /// `search_dirs` with the Node directory first, so a shim finds *that*
    /// Node rather than whichever one the inherited `PATH` reaches.
    fn child_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(node) = &self.node_bin_dir {
            dirs.push(node.clone());
        }
        for dir in &self.search_dirs {
            if !dirs.contains(dir) {
                dirs.push(dir.clone());
            }
        }
        dirs
    }

    /// The npm launcher to run: the one beside the detected Node, else the
    /// first on the search directories, else the bare name for the child's
    /// `PATH` to resolve.
    fn npm_program(&self) -> PathBuf {
        if let Some(node) = &self.node_bin_dir {
            let beside_node = node.join(npm_binary_name());
            if beside_node.is_file() {
                return beside_node;
            }
        }
        crate::cli_detect::find_executable_in("npm", &self.search_dirs)
            .unwrap_or_else(|| PathBuf::from(npm_binary_name()))
    }
}

/// One `npm install` invocation, fully resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallAttempt {
    /// `"mirror"` or `"official"`, for labelling failures.
    pub label: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Prepended to the child's `PATH`.
    pub dirs: Vec<PathBuf>,
}

impl InstallAttempt {
    /// The command as the user would type it.
    pub fn display(&self) -> String {
        let mut line = self
            .program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "npm".to_owned());
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }

    fn command(&self) -> std::process::Command {
        let args: Vec<&str> = self.args.iter().map(String::as_str).collect();
        crate::cli_detect::command_for(&self.program, &args, &self.dirs)
    }
}

/// The attempts for `package`, in order: npm anchored to the detected Node,
/// the mirror and the official registry both present on every platform.
///
/// Anchoring matters with two Nodes on a machine — nvm's and Homebrew's,
/// say. A bare `npm` would install into whichever one the inherited `PATH`
/// reaches, while the app runs the other, and the CLI would land in a prefix
/// nothing here searches.
pub fn install_attempts(ctx: &InstallContext, package: &str) -> Vec<InstallAttempt> {
    let program = ctx.npm_program();
    let dirs = ctx.child_dirs();
    let pinned = format!("{package}@latest");
    let mirror = InstallAttempt {
        label: "mirror",
        program: program.clone(),
        args: vec![
            "install".to_owned(),
            "-g".to_owned(),
            pinned.clone(),
            format!("--registry={MIRROR_REGISTRY}"),
            "--fetch-retries=2".to_owned(),
            "--fetch-timeout=60000".to_owned(),
        ],
        dirs: dirs.clone(),
    };
    let official = InstallAttempt {
        label: "official",
        program,
        args: vec!["install".to_owned(), "-g".to_owned(), pinned],
        dirs,
    };
    if cfg!(target_os = "windows") {
        vec![mirror, official]
    } else {
        vec![official, mirror]
    }
}

/// What a failed install's output points at, when it points anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallHint {
    /// npm could not write its global prefix.
    Permission,
    /// The registry was unreachable.
    Network,
}

/// Read npm's failure for the two causes with a known fix.
pub fn classify_failure(output: &str) -> Option<InstallHint> {
    let upper = output.to_ascii_uppercase();
    if ["EACCES", "EPERM", "PERMISSION DENIED", "ACCESS IS DENIED", "OPERATION NOT PERMITTED"]
        .iter()
        .any(|marker| upper.contains(marker))
    {
        return Some(InstallHint::Permission);
    }
    if [
        "ETIMEDOUT",
        "ENOTFOUND",
        "ECONNRESET",
        "ECONNREFUSED",
        "EAI_AGAIN",
        "FETCH_ERROR",
        "NETWORK",
        "TIMED OUT",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
    {
        return Some(InstallHint::Network);
    }
    None
}

/// The commands that move npm's global prefix somewhere the user can write,
/// for the permission hint. Copyable, not run: changing a prefix is the
/// user's decision.
pub fn permission_fix_commands() -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec![
            "npm config set prefix \"%LOCALAPPDATA%\\npm-global\"".to_owned(),
            "setx PATH \"%LOCALAPPDATA%\\npm-global;%PATH%\"".to_owned(),
        ]
    } else {
        vec![
            "mkdir -p ~/.npm-global && npm config set prefix ~/.npm-global".to_owned(),
            "echo 'export PATH=\"$HOME/.npm-global/bin:$PATH\"' >> ~/.zshrc".to_owned(),
        ]
    }
}

/// How an install ended, after the binary was actually looked for and run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallVerdict {
    /// Found on the app's search directories and answering `--version`.
    Installed { path: PathBuf, version: String },
    /// Runs, but only from a directory nothing searched before — npm's
    /// global prefix. The directory is remembered for this app; the user's
    /// terminals may still need it on `PATH`.
    InstalledNotOnPath {
        bin_dir: PathBuf,
        path: PathBuf,
        version: String,
    },
    /// npm succeeded, the binary exists, and it does not run.
    InstalledNotRunnable { path: PathBuf, diagnostic: String },
    /// Every attempt failed; `output` carries each one's tail.
    Failed {
        output: String,
        hint: Option<InstallHint>,
    },
}

impl InstallVerdict {
    /// The CLI can be used from this app.
    pub fn is_usable(&self) -> bool {
        matches!(
            self,
            InstallVerdict::Installed { .. } | InstallVerdict::InstalledNotOnPath { .. }
        )
    }
}

/// Progress of one install, for a status row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallStage {
    /// Running the attempt with this label.
    Running { attempt: &'static str },
    /// npm exited 0; now looking for and running the binary.
    Verifying,
}

/// Install `descriptor` and verify the result.
///
/// Blocking — run it off the UI thread. Exit status 0 from npm is not the
/// verdict: the binary is then located in the search directories plus npm's
/// own global prefix, and run, so "installed" means "the app can start it".
pub fn install_cli(
    ctx: &InstallContext,
    descriptor: &CliDescriptor,
    mut report: impl FnMut(InstallStage),
) -> InstallVerdict {
    let attempts = install_attempts(ctx, descriptor.package);
    let mut failures: Vec<String> = Vec::new();
    let mut succeeded: Option<&InstallAttempt> = None;
    for attempt in &attempts {
        report(InstallStage::Running {
            attempt: attempt.label,
        });
        let run = crate::cli_detect::run_with_timeout(attempt.command(), ctx.timeout);
        if run.success {
            succeeded = Some(attempt);
            break;
        }
        failures.push(format!(
            "{} ({})\n{}",
            attempt.display(),
            attempt.label,
            last_meaningful_lines(&run.output, 6)
        ));
    }
    let Some(attempt) = succeeded else {
        let output = failures.join("\n\n");
        let hint = classify_failure(&output);
        return InstallVerdict::Failed { output, hint };
    };
    report(InstallStage::Verifying);
    verify_install(ctx, descriptor, attempt, &mut |bin_dir| {
        crate::cli_detect::remember_search_dir(bin_dir);
        crate::node_install::persist_windows_user_path(bin_dir);
    })
}

/// Locate and run the binary npm claims to have installed. `remember` is
/// told about a prefix directory that was not on the search list.
fn verify_install(
    ctx: &InstallContext,
    descriptor: &CliDescriptor,
    attempt: &InstallAttempt,
    remember: &mut dyn FnMut(&Path),
) -> InstallVerdict {
    let prefix_bin = npm_global_bin(attempt);
    let mut dirs = ctx.search_dirs.clone();
    if let Some(bin) = &prefix_bin
        && !dirs.contains(bin)
    {
        dirs.push(bin.clone());
    }
    let Some(path) = crate::cli_detect::find_executable_in(descriptor.binary, &dirs) else {
        return InstallVerdict::Failed {
            output: format!(
                "npm reported success, but `{}` was not found afterwards{}",
                descriptor.binary,
                prefix_bin
                    .map(|bin| format!(" (npm's global bin directory is {})", bin.display()))
                    .unwrap_or_default()
            ),
            hint: None,
        };
    };
    let mut child_dirs = ctx.child_dirs();
    if let Some(parent) = path.parent()
        && !child_dirs.iter().any(|dir| dir == parent)
    {
        child_dirs.push(parent.to_path_buf());
    }
    match crate::cli_detect::probe_version(&path, &child_dirs, crate::cli_detect::PROBE_TIMEOUT) {
        Probe::Found { version, .. } => {
            let on_search_dirs = ctx.search_dirs.iter().any(|dir| path.starts_with(dir));
            if on_search_dirs {
                return InstallVerdict::Installed { path, version };
            }
            let bin_dir = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.clone());
            remember(&bin_dir);
            InstallVerdict::InstalledNotOnPath {
                bin_dir,
                path,
                version,
            }
        }
        Probe::FoundButFailed { diagnostic, .. } => {
            InstallVerdict::InstalledNotRunnable { path, diagnostic }
        }
        Probe::NotFound => InstallVerdict::Failed {
            output: format!("`{}` vanished between install and verification", path.display()),
            hint: None,
        },
    }
}

/// `npm prefix -g` through the same npm the install used, as a bin directory.
fn npm_global_bin(attempt: &InstallAttempt) -> Option<PathBuf> {
    let run = crate::cli_detect::run_with_timeout(
        crate::cli_detect::command_for(&attempt.program, &["prefix", "-g"], &attempt.dirs),
        Duration::from_secs(30),
    );
    if !run.success {
        return None;
    }
    // The prefix is the last non-empty line; npm may print notices first.
    let prefix = run.output.lines().rev().find(|line| !line.trim().is_empty())?;
    npm_global_bin_from_prefix(prefix)
}

/// Run each candidate until one succeeds.
///
/// A failure keeps every attempt's tail in the output, labeled, so "the mirror
/// timed out and then the official registry refused the proxy" reads as two
/// distinct problems rather than one mystery.
pub fn run_candidates(commands: &[String]) -> InstallOutcome {
    let mut failures: Vec<String> = Vec::new();
    for command in commands {
        let outcome = run_command(command);
        if outcome.success {
            return outcome;
        }
        failures.push(format!("{command}\n{}", outcome.output));
    }
    InstallOutcome {
        success: false,
        output: failures.join("\n\n"),
    }
}

/// Extract the major version from `node --version` style output.
///
/// Accepts `v22.11.0`, `22.11.0`, and trailing whitespace.
pub fn parse_major_version(output: &str) -> Option<u32> {
    parse_semantic_version(output)?
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Pull the first `x.y.z` out of arbitrary CLI version output.
pub fn parse_semantic_version(output: &str) -> Option<String> {
    let mut digits = String::new();
    let mut dots = 0;
    for character in output.chars() {
        match character {
            '0'..='9' => digits.push(character),
            '.' if !digits.is_empty() && dots < 2 && !digits.ends_with('.') => {
                digits.push('.');
                dots += 1;
            }
            _ if dots == 2 && !digits.ends_with('.') => break,
            _ => {
                digits.clear();
                dots = 0;
            }
        }
    }
    let trimmed = digits.trim_end_matches('.');
    (trimmed.split('.').count() == 3).then(|| trimmed.to_owned())
}

/// Whether the detected Node satisfies [`REQUIRED_NODE_MAJOR`].
pub fn node_is_supported(version_output: &str) -> bool {
    parse_major_version(version_output).is_some_and(|major| major >= REQUIRED_NODE_MAJOR)
}

/// Resolve npm's global bin directory from `npm prefix -g` output.
///
/// On Windows the prefix *is* the bin directory; elsewhere binaries live in
/// `<prefix>/bin`.
pub fn npm_global_bin_from_prefix(prefix_output: &str) -> Option<PathBuf> {
    let prefix = prefix_output.trim();
    if prefix.is_empty() {
        return None;
    }
    let prefix = PathBuf::from(prefix);
    if cfg!(target_os = "windows") {
        Some(prefix)
    } else {
        Some(prefix.join("bin"))
    }
}

/// Entries that must be added to the running platform's `PATH` for `needed`
/// to be reachable.
pub fn missing_path_entries(current_path: &str, needed: &[String]) -> Vec<String> {
    missing_entries_with(
        current_path,
        needed,
        if cfg!(target_os = "windows") { ';' } else { ':' },
        cfg!(target_os = "windows"),
    )
}

/// The separator and case rules follow the `PATH` value's own format, not the
/// host: `windows_user_path_value` edits a Windows registry value and must
/// behave identically when this crate is compiled on macOS or Linux (where
/// the tests also run).
///
/// Comparison ignores trailing separators, and folds case only for Windows
/// values — Windows reports the same directory in several spellings, while
/// on Unix /opt/Tools and /opt/tools are different directories.
fn missing_entries_with(
    current_path: &str,
    needed: &[String],
    separator: char,
    fold_case: bool,
) -> Vec<String> {
    let normalize = |entry: &str| {
        let trimmed = entry.trim().trim_end_matches(['/', '\\']);
        if fold_case {
            trimmed.to_ascii_lowercase()
        } else {
            trimmed.to_owned()
        }
    };
    let existing: Vec<String> = current_path
        .split(separator)
        .map(normalize)
        .filter(|entry| !entry.is_empty())
        .collect();
    let mut missing = Vec::new();
    for entry in needed {
        let normalized = normalize(entry);
        if normalized.is_empty() {
            continue;
        }
        if !existing.contains(&normalized) && !missing.contains(entry) {
            missing.push(entry.clone());
        }
    }
    missing
}

/// Append entries to a Windows user `PATH` value without duplicating them.
/// Always Windows semantics (`;`, case-folded) — the value comes from the
/// Windows registry no matter where this code compiles.
pub fn windows_user_path_value(current_path: &str, additions: &[String]) -> String {
    let missing = missing_entries_with(current_path, additions, ';', true);
    if missing.is_empty() {
        return current_path.to_owned();
    }
    let mut value = current_path.trim_end_matches(';').to_owned();
    for entry in missing {
        if !value.is_empty() {
            value.push(';');
        }
        value.push_str(&entry);
    }
    value
}

/// PowerShell that broadcasts `WM_SETTINGCHANGE`.
///
/// Without it a `PATH` edit is invisible to already-running processes,
/// including the shell the user is about to type in — the classic "I installed
/// it but the command is not found" report.
pub fn broadcast_environment_change_command() -> &'static str {
    r#"$sig = '[DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'; $type = Add-Type -MemberDefinition $sig -Name 'Win32SendMessageTimeout' -Namespace Win32Functions -PassThru; [UIntPtr]$result = [UIntPtr]::Zero; $type::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result) | Out-Null"#
}

/// Result of running an install command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallOutcome {
    pub success: bool,
    /// Combined stdout and stderr, trimmed. npm puts its diagnostics on both.
    pub output: String,
}

/// Shell invocation for a command line.
///
/// The commands are shell strings (`npm install -g x --registry y`), and on
/// Windows `npm` is a `.cmd` shim that only a shell will resolve — so this
/// always goes through one rather than trying to exec the first word.
pub fn shell_invocation(command: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "windows") {
        ("cmd", vec!["/C".to_owned(), command.to_owned()])
    } else {
        ("sh", vec!["-c".to_owned(), command.to_owned()])
    }
}

/// Directories where the app-managed toolchains may live, existing ones only.
///
/// Prepended to `PATH` for everything this crate spawns and for the agent
/// processes: after a portable Node or Git install, the app's own inherited
/// `PATH` predates the install and would never find them — the `claude` shim
/// needs `node`, and the daemon's checkpoints shell out to a bare `git`.
pub fn node_runtime_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(managed) = crate::node_install::managed_node_bin_dir()
        && managed.is_dir()
    {
        dirs.push(managed);
    }
    if cfg!(target_os = "windows") {
        let program_files = PathBuf::from(r"C:\Program Files\nodejs");
        if program_files.is_dir() {
            dirs.push(program_files);
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let npm = PathBuf::from(appdata).join("npm");
            if npm.is_dir() {
                dirs.push(npm);
            }
        }
    }
    dirs
}

/// `current` with `extra` prepended, skipping entries already present.
pub(crate) fn augment_path(current: Option<std::ffi::OsString>, extra: &[PathBuf]) -> std::ffi::OsString {
    let existing: Vec<PathBuf> = current
        .as_deref()
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    let additions: Vec<&PathBuf> = extra
        .iter()
        .filter(|candidate| !existing.contains(candidate))
        .collect();
    std::env::join_paths(
        additions
            .into_iter()
            .cloned()
            .chain(existing),
    )
    .unwrap_or_else(|_| current.unwrap_or_default())
}

/// Prepend the managed toolchains to a command's `PATH`, and point Claude
/// Code at Git Bash when one is known.
///
/// Applied to every agent process the fork spawns, so a CLI installed against
/// the managed runtime keeps working without an app restart.
pub fn apply_node_runtime(command: &mut std::process::Command) {
    let dirs = crate::cli_detect::detection_dirs();
    if !dirs.is_empty() {
        let configured = command
            .get_envs()
            .find(|(name, _)| {
                name.to_str().is_some_and(|name| name.eq_ignore_ascii_case("path"))
            })
            .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
            .or_else(|| std::env::var_os("PATH"));
        command.env("PATH", augment_path(configured, &dirs));
    }

}

/// Switches that keep Claude Code's startup off the network. The CLI
/// otherwise checks for updates and phones home before it is ready for its
/// first prompt, and where those hosts are slow or blocked that is seconds
/// added to every cold start. The routed configuration already writes the
/// same switch into `settings.json`; this covers sessions that are not
/// routed through the service. A value the user set in their own
/// environment or settings wins over this default.
const CLAUDE_STARTUP_ENV: [(&str, &str); 1] = [("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")];

/// Environment for a provider process this app launches: the managed Node
/// runtime on `PATH` ([`apply_node_runtime`]) plus per-CLI startup switches.
pub fn apply_provider_launch_env(command: &mut std::process::Command, provider_id: &str) {
    apply_node_runtime(command);
    if provider_id != "claude" {
        return;
    }
    for (name, value) in CLAUDE_STARTUP_ENV {
        let already_set = command
            .get_envs()
            .any(|(candidate, _)| candidate.to_str().is_some_and(|c| c.eq_ignore_ascii_case(name)))
            || std::env::var_os(name).is_some();
        if !already_set {
            command.env(name, value);
        }
    }
}

/// Run a program directly — no shell — and capture its output.
///
/// Used where an argument would otherwise need shell quoting (PowerShell
/// scripts, paths with spaces); `cmd`'s quoting rules are not worth fighting.
pub fn run_program(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> InstallOutcome {
    let mut process = std::process::Command::new(program.as_ref());
    process.args(args);
    process.env("PATH", augment_path(std::env::var_os("PATH"), &crate::cli_detect::detection_dirs()));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    capture(process, &program.as_ref().to_string_lossy())
}

/// Run an install command and capture its output.
///
/// Blocking; callers run it off the UI thread. Output is captured rather than
/// streamed because the app has no terminal surface to stream into — a failure
/// is reported with npm's own message attached, which is what makes proxy and
/// permission errors diagnosable.
pub fn run_command(command: &str) -> InstallOutcome {
    let (program, args) = shell_invocation(command);
    let mut process = std::process::Command::new(program);
    process.args(args);
    // The freshly installed managed runtime must be visible to `npm` runs that
    // happen in this same app session.
    process.env("PATH", augment_path(std::env::var_os("PATH"), &crate::cli_detect::detection_dirs()));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    capture(process, program)
}

fn capture(mut process: std::process::Command, program: &str) -> InstallOutcome {
    match process.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let errors = String::from_utf8_lossy(&output.stderr);
            if !errors.trim().is_empty() {
                if !text.trim().is_empty() {
                    text.push('\n');
                }
                text.push_str(&errors);
            }
            InstallOutcome {
                success: output.status.success(),
                output: last_meaningful_lines(text.trim(), 6),
            }
        }
        Err(error) => InstallOutcome {
            success: false,
            output: format!("could not start {program}: {error}"),
        },
    }
}

/// Keep the tail of a command's output: npm's failure reason is at the end,
/// and the preceding progress lines are noise in a settings panel.
pub(crate) fn last_meaningful_lines(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}

/// A step the user must complete, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupStep {
    /// Node is missing or too old; nothing else can proceed.
    InstallNode { required_major: u32 },
    /// Install one agent CLI. `commands` are tried in order.
    InstallCli {
        id: &'static str,
        display_name: &'static str,
        commands: Vec<String>,
    },
}

/// Work out what the user has to do, given what was detected.
///
/// Returns an empty plan when the machine is already usable.
pub fn setup_plan(node_version_output: Option<&str>, detections: &[Detection]) -> Vec<SetupStep> {
    let mut steps = Vec::new();
    let node_ok = node_version_output.is_some_and(node_is_supported);
    if !node_ok {
        steps.push(SetupStep::InstallNode {
            required_major: REQUIRED_NODE_MAJOR,
        });
    }
    for detection in detections {
        if detection.is_installed() {
            continue;
        }
        let Some(descriptor) = descriptor(detection.id) else {
            continue;
        };
        steps.push(SetupStep::InstallCli {
            id: descriptor.id,
            display_name: descriptor.display_name,
            commands: install_candidates(descriptor.package),
        });
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(id: &'static str) -> Detection {
        Detection {
            id,
            probe: Probe::NotFound,
        }
    }

    fn installed(id: &'static str) -> Detection {
        Detection {
            id,
            probe: Probe::Found {
                path: PathBuf::from("/usr/local/bin/x"),
                version: "1.0.0".to_owned(),
            },
        }
    }

    fn broken(id: &'static str) -> Detection {
        Detection {
            id,
            probe: Probe::FoundButFailed {
                path: PathBuf::from("/usr/local/bin/x"),
                diagnostic: "node: command not found".to_owned(),
            },
        }
    }

    #[test]
    fn descriptors_are_unique_and_addressable() {
        for descriptor in DESCRIPTORS {
            assert_eq!(super::descriptor(descriptor.id), Some(descriptor));
            assert!(!descriptor.package.is_empty());
            assert!(!descriptor.binary.is_empty());
        }
        assert_eq!(super::descriptor("nope"), None);
    }

    #[test]
    fn install_command_pins_the_mirror_when_asked() {
        assert_eq!(
            install_command("@openai/codex", None),
            "npm install -g @openai/codex"
        );
        assert_eq!(
            install_command("@openai/codex", Some(MIRROR_REGISTRY)),
            "npm install -g @openai/codex --registry=https://registry.npmmirror.com --fetch-retries=2 --fetch-timeout=60000"
        );
        // A blank registry must not produce dangling flags.
        assert_eq!(
            install_command("@openai/codex", Some("  ")),
            "npm install -g @openai/codex"
        );
    }

    #[test]
    fn attempts_fall_back_between_mirror_and_official_on_every_platform() {
        let candidates = install_candidates("@openai/codex");
        assert_eq!(candidates.len(), 2);
        let mirror = candidates
            .iter()
            .position(|candidate| candidate.contains("--registry=https://registry.npmmirror.com"))
            .expect("a mirror attempt");
        let official = candidates
            .iter()
            .position(|candidate| candidate.ends_with("@openai/codex@latest"))
            .expect("an official attempt");
        assert!(candidates[mirror].contains("--fetch-retries=2"));
        // Mirror first where the audience needs it, official first elsewhere —
        // but both everywhere.
        if cfg!(target_os = "windows") {
            assert!(mirror < official);
        } else {
            assert!(official < mirror);
        }

        let ctx = InstallContext::new(None, Vec::new());
        let attempts = install_attempts(&ctx, "@openai/codex");
        let labels: Vec<&str> = attempts.iter().map(|attempt| attempt.label).collect();
        if cfg!(target_os = "windows") {
            assert_eq!(labels, ["mirror", "official"]);
        } else {
            assert_eq!(labels, ["official", "mirror"]);
        }
        assert!(attempts.iter().all(|attempt| attempt.display().starts_with("npm")));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-cli-install-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn executable(dir: &Path, stem: &str, body: &str) -> PathBuf {
        let path = if cfg!(target_os = "windows") {
            dir.join(format!("{stem}.cmd"))
        } else {
            dir.join(stem)
        };
        std::fs::write(&path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn install_attempts_anchor_npm_to_the_node_dir_and_prepend_it_to_path() {
        let node_dir = scratch("anchor");
        let npm = executable(&node_dir, "npm", "");
        let elsewhere = PathBuf::from("/somewhere/else");
        let ctx = InstallContext::new(Some(node_dir.clone()), vec![elsewhere.clone()]);

        let attempts = install_attempts(&ctx, "@anthropic-ai/claude-code");
        for attempt in &attempts {
            assert_eq!(attempt.program, npm, "npm beside the detected node");
            assert_eq!(attempt.dirs.first(), Some(&node_dir), "node dir first on PATH");
            assert!(attempt.dirs.contains(&elsewhere));
            assert!(attempt.args.contains(&"@anthropic-ai/claude-code@latest".to_owned()));
        }
        assert!(attempts[0].display().starts_with("npm"));
        let _ = std::fs::remove_dir_all(node_dir);
    }

    #[test]
    fn classify_failure_recognises_permission_and_network() {
        assert_eq!(
            classify_failure("npm ERR! code EACCES\nnpm ERR! syscall mkdir"),
            Some(InstallHint::Permission)
        );
        assert_eq!(
            classify_failure("Error: EPERM: operation not permitted, rename"),
            Some(InstallHint::Permission)
        );
        assert_eq!(
            classify_failure("npm ERR! code ETIMEDOUT\nnpm ERR! network request failed"),
            Some(InstallHint::Network)
        );
        assert_eq!(
            classify_failure("npm ERR! code ENOTFOUND registry.npmjs.org"),
            Some(InstallHint::Network)
        );
        assert_eq!(classify_failure("npm ERR! 404 Not Found"), None);
        assert!(!permission_fix_commands().is_empty());
    }

    #[test]
    fn verdict_is_not_on_path_when_binary_only_in_prefix_bin() {
        let root = scratch("verify");
        let node_dir = root.join("node");
        let prefix = root.join("prefix");
        let prefix_bin = npm_global_bin_from_prefix(&prefix.display().to_string()).unwrap();
        std::fs::create_dir_all(&node_dir).unwrap();
        std::fs::create_dir_all(&prefix_bin).unwrap();

        // A fake npm: installs "succeed", and `prefix -g` names our prefix.
        let npm_body = if cfg!(target_os = "windows") {
            format!(
                "@echo off\r\nif \"%1\"==\"prefix\" echo {}\r\nexit /b 0\r\n",
                prefix.display()
            )
        } else {
            format!(
                "#!/bin/sh\ncase \"$1\" in prefix) echo '{}';; esac\nexit 0\n",
                prefix.display()
            )
        };
        executable(&node_dir, "npm", &npm_body);
        // The CLI npm "installed", living only in the prefix's bin directory.
        let cli_body = if cfg!(target_os = "windows") {
            "@echo off\r\necho 1.2.3\r\n".to_owned()
        } else {
            "#!/bin/sh\necho 1.2.3\n".to_owned()
        };
        let cli = executable(&prefix_bin, "grok", &cli_body);

        let ctx = InstallContext::new(Some(node_dir.clone()), vec![node_dir.clone()]);
        let descriptor = descriptor("grok").unwrap();
        let attempt = install_attempts(&ctx, descriptor.package).remove(0);
        let mut remembered = Vec::new();
        let verdict = verify_install(&ctx, descriptor, &attempt, &mut |dir| {
            remembered.push(dir.to_path_buf());
        });

        assert_eq!(
            verdict,
            InstallVerdict::InstalledNotOnPath {
                bin_dir: prefix_bin.clone(),
                path: cli.clone(),
                version: "1.2.3".to_owned(),
            }
        );
        assert_eq!(remembered, vec![prefix_bin.clone()]);
        assert!(verdict.is_usable());

        // Once the prefix is among the search dirs, the same result is plain
        // "installed".
        let ctx = InstallContext::new(Some(node_dir.clone()), vec![node_dir, prefix_bin]);
        let verdict = verify_install(&ctx, descriptor, &attempt, &mut |_| {
            panic!("nothing to remember")
        });
        assert!(matches!(verdict, InstallVerdict::Installed { .. }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn run_candidates_stops_at_the_first_success() {
        let outcome = run_candidates(&["exit 1".to_owned(), "exit 0".to_owned()]);
        assert!(outcome.success, "output: {}", outcome.output);
    }

    #[test]
    fn run_candidates_reports_every_failed_attempt() {
        let outcome = run_candidates(&[
            "echo first-fail 1>&2 && exit 1".to_owned(),
            "echo second-fail 1>&2 && exit 1".to_owned(),
        ]);
        assert!(!outcome.success);
        // Both attempts must be readable, labeled by their command, so two
        // different failures do not collapse into one mystery.
        assert!(outcome.output.contains("first-fail"), "{}", outcome.output);
        assert!(outcome.output.contains("second-fail"), "{}", outcome.output);
        assert!(outcome.output.contains("exit 1"));
    }

    #[test]
    fn parses_node_versions() {
        assert_eq!(parse_major_version("v22.11.0\n"), Some(22));
        assert_eq!(parse_major_version("22.11.0"), Some(22));
        assert_eq!(parse_major_version("v18.20.4"), Some(18));
        assert_eq!(parse_major_version(""), None);
        assert_eq!(parse_major_version("not a version"), None);
    }

    #[test]
    fn parses_versions_out_of_noisy_output() {
        assert_eq!(
            parse_semantic_version("claude-code/1.2.34 darwin-arm64"),
            Some("1.2.34".to_owned())
        );
        assert_eq!(parse_semantic_version("v0.0.1"), Some("0.0.1".to_owned()));
        // Two components is not a semantic version.
        assert_eq!(parse_semantic_version("1.2"), None);
    }

    #[test]
    fn node_support_follows_the_required_major() {
        assert!(node_is_supported("v22.0.0"));
        assert!(node_is_supported("v24.1.0"));
        assert!(!node_is_supported("v20.11.1"));
        assert!(!node_is_supported(""));
    }

    #[test]
    fn npm_global_bin_resolves_per_platform() {
        let resolved = npm_global_bin_from_prefix("  /usr/local  ").expect("prefix");
        if cfg!(target_os = "windows") {
            assert_eq!(resolved, PathBuf::from("/usr/local"));
        } else {
            assert_eq!(resolved, PathBuf::from("/usr/local/bin"));
        }
        assert_eq!(npm_global_bin_from_prefix("   "), None);
    }

    #[test]
    fn missing_path_entries_ignores_case_and_trailing_separators() {
        let current = if cfg!(target_os = "windows") {
            r"C:\Windows;C:\Users\me\AppData\Roaming\npm\"
        } else {
            "/usr/bin:/usr/local/bin/"
        };
        let already = if cfg!(target_os = "windows") {
            r"c:\users\me\appdata\roaming\npm"
        } else {
            "/usr/local/bin"
        };
        assert!(missing_path_entries(current, &[already.to_owned()]).is_empty());

        let fresh = missing_path_entries(current, &["/opt/tools".to_owned()]);
        assert_eq!(fresh, vec!["/opt/tools".to_owned()]);
    }

    #[test]
    fn missing_path_entries_deduplicates_and_skips_blanks() {
        let entries = missing_path_entries(
            "/usr/bin",
            &["/opt/a".to_owned(), "/opt/a".to_owned(), "  ".to_owned()],
        );
        assert_eq!(entries, vec!["/opt/a".to_owned()]);
    }

    #[test]
    fn windows_path_append_is_idempotent() {
        let current = r"C:\Windows;C:\Tools";
        let once = windows_user_path_value(current, &[r"C:\Extra".to_owned()]);
        assert_eq!(once, r"C:\Windows;C:\Tools;C:\Extra");
        // Re-running setup must not keep growing PATH.
        let twice = windows_user_path_value(&once, &[r"C:\Extra".to_owned()]);
        assert_eq!(twice, once);
    }

    #[test]
    fn windows_path_append_handles_a_trailing_separator() {
        assert_eq!(
            windows_user_path_value(r"C:\Windows;", &[r"C:\Extra".to_owned()]),
            r"C:\Windows;C:\Extra"
        );
    }

    #[test]
    fn plan_is_empty_when_everything_is_present() {
        let detections: Vec<Detection> = DESCRIPTORS.iter().map(|d| installed(d.id)).collect();
        assert!(setup_plan(Some("v22.1.0"), &detections).is_empty());
    }

    #[test]
    fn plan_puts_node_first_because_npm_installs_depend_on_it() {
        let plan = setup_plan(Some("v18.0.0"), &[missing("claude")]);
        assert_eq!(
            plan.first(),
            Some(&SetupStep::InstallNode {
                required_major: REQUIRED_NODE_MAJOR
            })
        );
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn plan_lists_only_missing_clis_with_their_candidates() {
        let plan = setup_plan(Some("v22.0.0"), &[installed("claude"), missing("codex")]);
        assert_eq!(
            plan,
            vec![SetupStep::InstallCli {
                id: "codex",
                display_name: "Codex CLI",
                commands: install_candidates("@openai/codex"),
            }]
        );
    }

    #[test]
    fn absent_node_output_is_treated_as_missing_node() {
        let plan = setup_plan(None, &[]);
        assert_eq!(
            plan,
            vec![SetupStep::InstallNode {
                required_major: REQUIRED_NODE_MAJOR
            }]
        );
    }

    #[test]
    fn shell_invocation_uses_a_shell_so_npm_shims_resolve() {
        let (program, args) = shell_invocation("npm install -g x");
        if cfg!(target_os = "windows") {
            assert_eq!(program, "cmd");
            assert_eq!(args, vec!["/C".to_owned(), "npm install -g x".to_owned()]);
        } else {
            assert_eq!(program, "sh");
            assert_eq!(args, vec!["-c".to_owned(), "npm install -g x".to_owned()]);
        }
    }

    #[test]
    fn run_command_reports_success_and_failure() {
        let ok = run_command("exit 0");
        assert!(ok.success, "output: {}", ok.output);

        let failed = run_command("exit 3");
        assert!(!failed.success);
    }

    #[test]
    fn run_command_captures_stderr_because_npm_fails_there() {
        let outcome = run_command("echo boom 1>&2 && exit 1");
        assert!(!outcome.success);
        assert!(outcome.output.contains("boom"), "output: {}", outcome.output);
    }

    #[test]
    fn output_is_trimmed_to_the_tail_where_the_reason_lives() {
        let text = (1..=20).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let tail = last_meaningful_lines(&text, 3);
        assert_eq!(tail, "18\n19\n20");
        // Blank lines must not consume the budget.
        assert_eq!(last_meaningful_lines("a\n\n\nb", 2), "a\nb");
    }

    #[test]
    fn broadcast_command_targets_the_environment_topic() {
        let command = broadcast_environment_change_command();
        assert!(command.contains("SendMessageTimeout"));
        assert!(command.contains("'Environment'"));
    }

    #[test]
    fn claude_launches_with_nonessential_traffic_off_unless_the_user_decided() {
        let value = |command: &std::process::Command| {
            command
                .get_envs()
                .find(|(name, _)| *name == "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC")
                .and_then(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
        };

        let mut claude = std::process::Command::new("claude");
        apply_provider_launch_env(&mut claude, "claude");
        // The test process itself may carry the variable; either way the
        // child ends up with it set.
        assert!(value(&claude).is_some() || std::env::var_os("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC").is_some());

        let mut opted_in = std::process::Command::new("claude");
        opted_in.env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "0");
        apply_provider_launch_env(&mut opted_in, "claude");
        assert_eq!(value(&opted_in).as_deref(), Some("0"));

        let mut codex = std::process::Command::new("codex");
        apply_provider_launch_env(&mut codex, "codex");
        assert!(value(&codex).is_none());
    }

    #[test]
    fn plan_does_not_reinstall_a_broken_cli() {
        // A shim whose Node is gone is an install to repair, not a missing
        // CLI; `npm install` over it would leave it exactly as broken.
        let plan = setup_plan(Some("v22.0.0"), &[broken("claude"), missing("codex")]);
        assert_eq!(plan.len(), 1);
        assert!(matches!(plan[0], SetupStep::InstallCli { id: "codex", .. }));
        assert!(broken("claude").is_installed());
        assert!(!broken("claude").is_runnable());
    }
}
