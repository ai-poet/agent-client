//! Where the agent CLIs and the Node runtime are looked for, and how their
//! health is probed.
//!
//! Fork addition, modelled on cc-switch's tool detection. Two things it does
//! that a plain `PATH` walk does not:
//!
//! - **It looks where version managers put things.** A GUI-launched app
//!   inherits a `PATH` that predates nvm, fnm, Volta, mise, pnpm, Scoop and
//!   the app's own managed runtime, all of which extend `PATH` only from a
//!   shell profile. [`version_manager_dirs`] enumerates their conventional
//!   directories so a CLI installed through any of them is found without a
//!   restart; the desktop merges these with the login-shell `PATH` that
//!   waku-core captures.
//! - **It runs the binary.** Finding `claude.cmd` proves nothing when the Node
//!   it needs is too old or gone; [`probe_version`] distinguishes *not
//!   installed* from *installed but not runnable* and keeps the CLI's own
//!   diagnostic, which is what the user needs to fix it. Every probe has a
//!   timeout, because a hung shim must not hang the settings page.
//!
//! Nothing here touches the UI; the desktop runs [`snapshot`] on a background
//! thread and renders the stored result.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cli_install::{self, Detection, SetupStep, parse_semantic_version};
use crate::env_conflicts::{self, EnvConflict};

/// How long a `--version` probe may take before it is killed.
///
/// cc-switch uses the same budget: long enough for a cold Node start on a
/// slow disk, short enough that a wedged shim does not stall detection.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What running a binary revealed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Probe {
    /// The binary answered `--version`.
    Found { path: PathBuf, version: String },
    /// The binary exists but `--version` failed, timed out, or said nothing —
    /// typically a shim whose Node is missing or too old. `diagnostic` is the
    /// tail of what it printed.
    FoundButFailed { path: PathBuf, diagnostic: String },
    /// No such binary in any searched directory.
    NotFound,
}

impl Probe {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Probe::Found { path, .. } | Probe::FoundButFailed { path, .. } => Some(path),
            Probe::NotFound => None,
        }
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            Probe::Found { version, .. } => Some(version),
            _ => None,
        }
    }

    pub fn diagnostic(&self) -> Option<&str> {
        match self {
            Probe::FoundButFailed { diagnostic, .. } => Some(diagnostic),
            _ => None,
        }
    }

    /// A file was found, runnable or not.
    pub fn is_present(&self) -> bool {
        !matches!(self, Probe::NotFound)
    }

    pub fn is_runnable(&self) -> bool {
        matches!(self, Probe::Found { .. })
    }
}

// --- search directories ---------------------------------------------------

/// Directories the common Node version managers and package managers install
/// into, existing ones only. `env` is injected so the list is testable.
///
/// Order matters where a name can exist twice: a version manager's active
/// installation should win over a stale global prefix, so managers come
/// before the bare npm prefixes.
pub fn version_manager_dirs(
    home: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let env_dir = |name: &str| env(name).map(PathBuf::from);

    if cfg!(target_os = "windows") {
        // nvm-for-windows: NVM_SYMLINK is the active version; NVM_HOME holds
        // every installed version as a subdirectory.
        push_existing(&mut dirs, env_dir("NVM_SYMLINK"));
        if let Some(nvm_home) = env_dir("NVM_HOME") {
            push_existing(&mut dirs, Some(nvm_home.clone()));
            for child in version_subdirs(&nvm_home, None) {
                push_existing(&mut dirs, Some(child));
            }
        }
        push_existing(&mut dirs, env_dir("PNPM_HOME"));
        push_existing(&mut dirs, env_dir("VOLTA_HOME").map(|volta| volta.join("bin")));
        if let Some(local) = env_dir("LOCALAPPDATA") {
            push_existing(&mut dirs, Some(local.join("Volta").join("bin")));
            push_existing(&mut dirs, Some(local.join("pnpm")));
            for install in version_subdirs(&local.join("fnm").join("node-versions"), Some("installation")) {
                push_existing(&mut dirs, Some(install));
            }
            // Official standalone installers that do not use npm at all.
            push_existing(&mut dirs, Some(local.join("Programs").join("claude")));
            push_existing(
                &mut dirs,
                Some(local.join("Programs").join("OpenAI").join("Codex").join("bin")),
            );
            push_existing(&mut dirs, Some(local.join("Programs").join("nodejs")));
        }
        if let Some(home) = home {
            push_existing(&mut dirs, Some(home.join("scoop").join("shims")));
            push_existing(&mut dirs, Some(home.join(".volta").join("bin")));
        }
        if let Some(program_data) = env_dir("ProgramData") {
            push_existing(&mut dirs, Some(program_data.join("scoop").join("shims")));
        }
        if let Some(appdata) = env_dir("APPDATA") {
            push_existing(&mut dirs, Some(appdata.join("npm")));
        }
        return dirs;
    }

    let Some(home) = home else {
        return dirs;
    };
    // nvm: every installed version, newest first, so a stale one does not
    // shadow the one the user actually runs.
    let nvm_dir = env_dir("NVM_DIR").unwrap_or_else(|| home.join(".nvm"));
    for version in version_subdirs(&nvm_dir.join("versions").join("node"), Some("bin")) {
        push_existing(&mut dirs, Some(version));
    }
    // fnm keeps installs under its data dir and links the active one through
    // per-shell multishell directories.
    let fnm_dir = env_dir("FNM_DIR").unwrap_or_else(|| home.join(".local/share/fnm"));
    for install in version_subdirs(&fnm_dir.join("node-versions"), Some("installation/bin")) {
        push_existing(&mut dirs, Some(install));
    }
    for shell in version_subdirs(&home.join(".local/state/fnm_multishells"), Some("bin")) {
        push_existing(&mut dirs, Some(shell));
    }
    push_existing(&mut dirs, Some(home.join(".local/share/mise/shims")));
    for install in version_subdirs(&home.join(".local/share/mise/installs/node"), Some("bin")) {
        push_existing(&mut dirs, Some(install));
    }
    push_existing(
        &mut dirs,
        Some(
            env_dir("VOLTA_HOME")
                .unwrap_or_else(|| home.join(".volta"))
                .join("bin"),
        ),
    );
    push_existing(&mut dirs, env_dir("PNPM_HOME"));
    push_existing(&mut dirs, Some(home.join(".local/share/pnpm")));
    push_existing(&mut dirs, Some(home.join("n/bin")));
    push_existing(&mut dirs, Some(home.join(".npm-global/bin")));
    push_existing(&mut dirs, Some(home.join(".local/bin")));
    push_existing(&mut dirs, Some(home.join(".bun/bin")));
    push_existing(&mut dirs, Some(PathBuf::from("/opt/homebrew/bin")));
    push_existing(&mut dirs, Some(PathBuf::from("/usr/local/bin")));
    dirs
}

/// Subdirectories of `base` (optionally joined with `suffix`), sorted so the
/// highest version-looking name comes first. Non-version names sort last in
/// their original order.
fn version_subdirs(base: &Path, suffix: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut named: Vec<(Option<(u64, u64, u64)>, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = parse_semantic_version(&name).and_then(|version| {
                let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
                Some((parts.next()??, parts.next()??, parts.next()??))
            });
            let path = match suffix {
                Some(suffix) => entry.path().join(suffix),
                None => entry.path(),
            };
            (version, path)
        })
        .collect();
    named.sort_by(|a, b| b.0.cmp(&a.0));
    named.into_iter().map(|(_, path)| path).collect()
}

fn push_existing(dirs: &mut Vec<PathBuf>, candidate: Option<PathBuf>) {
    if let Some(candidate) = candidate
        && candidate.is_dir()
        && !dirs.contains(&candidate)
    {
        dirs.push(candidate);
    }
}

/// Every directory this crate adds to a `PATH` walk: the app-managed
/// runtime, the version managers, and whatever a previous install taught us.
///
/// waku-core appends this to its own search paths, so provider detection,
/// the CLI-setup section, and every spawned agent agree on where to look.
pub fn detection_dirs() -> Vec<PathBuf> {
    let mut dirs = cli_install::node_runtime_dirs();
    let env = |name: &str| std::env::var_os(name);
    for dir in version_manager_dirs(dirs::home_dir().as_deref(), &env) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    for dir in remembered_dirs() {
        if dir.is_dir() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Everything this crate searches: the user's shell `PATH`, the process
/// `PATH`, then [`detection_dirs`]. Resolving the shell `PATH` spawns a
/// shell the first time — never call this from a frame.
pub fn default_search_dirs() -> Vec<PathBuf> {
    let mut dirs = shell_path_dirs();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    for dir in detection_dirs() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

// --- the user's shell PATH ------------------------------------------------

/// The `PATH` the user's own terminal would have, resolved once per process.
///
/// A GUI-launched app inherits the launcher's `PATH`, not the one the
/// user's shell profile builds — and nvm, fnm, Homebrew, Volta and the
/// PowerShell-profile managers extend `PATH` only there. The daemon runs an
/// equivalent probe in waku-core for provider detection; this is the
/// desktop-side twin, so both processes look in the same places. Unix asks
/// the login shell to print its environment (`$SHELL -ilc /usr/bin/env`,
/// falling back to a non-interactive login shell when an rc file blocks);
/// Windows asks PowerShell for the profile-loaded `PATH` plus the fresh
/// user and machine registry values, which the inherited `PATH` can predate.
pub fn shell_path_dirs() -> Vec<PathBuf> {
    static SHELL_PATH: OnceLock<Vec<PathBuf>> = OnceLock::new();
    SHELL_PATH.get_or_init(resolve_shell_path_dirs).clone()
}

const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(not(target_os = "windows"))]
fn resolve_shell_path_dirs() -> Vec<PathBuf> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));
    for args in [["-i", "-l", "-c"].as_slice(), ["-l", "-c"].as_slice()] {
        let mut command = Command::new(&shell);
        command.args(args).arg("/usr/bin/env");
        let run = run_with_timeout(command, SHELL_PATH_TIMEOUT);
        if !run.success {
            continue;
        }
        // `env` prints every variable; take the PATH line whose value is an
        // absolute path, which skips an rc file's banner text.
        if let Some(value) = run
            .output
            .lines()
            .filter_map(|line| line.strip_prefix("PATH="))
            .find(|value| value.starts_with('/'))
        {
            return dedup(std::env::split_paths(value).collect());
        }
    }
    Vec::new()
}

#[cfg(target_os = "windows")]
fn resolve_shell_path_dirs() -> Vec<PathBuf> {
    const SCRIPT: &str = "\
$ErrorActionPreference = 'Continue'
Write-Output ('WAKU_PATH=' + [Environment]::GetEnvironmentVariable('PATH'))
foreach ($t in @('User', 'Machine')) {
  $v = [Environment]::GetEnvironmentVariable('PATH', $t)
  if ($v) { Write-Output ('WAKU_' + $t.ToUpper() + '=' + [Environment]::ExpandEnvironmentVariables($v)) }
}";
    // Profile-loaded first (fnm/Volta/nvm extend PATH there), then a
    // profile-free PowerShell on a short leash for the registry values alone.
    for (shell, load_profile, timeout) in [
        ("pwsh.exe", true, SHELL_PATH_TIMEOUT),
        ("powershell.exe", true, SHELL_PATH_TIMEOUT),
        ("powershell.exe", false, Duration::from_secs(2)),
    ] {
        let mut command = Command::new(shell);
        command.args(["-NoLogo", "-NonInteractive"]);
        if !load_profile {
            command.arg("-NoProfile");
        }
        command.args(["-Command", SCRIPT]);
        let run = run_with_timeout(command, timeout);
        if !run.success {
            continue;
        }
        let mut dirs = Vec::new();
        for prefix in ["WAKU_PATH=", "WAKU_USER=", "WAKU_MACHINE="] {
            if let Some(value) = run
                .output
                .lines()
                .find_map(|line| line.trim().strip_prefix(prefix))
            {
                dirs.extend(std::env::split_paths(value));
            }
        }
        if !dirs.is_empty() {
            return dedup(dirs);
        }
    }
    Vec::new()
}

/// Deduplicate, keeping first occurrences; case-insensitive on Windows,
/// where `PATH` matching is.
fn dedup(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    dirs.into_iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .filter(|dir| {
            let key = if cfg!(target_os = "windows") {
                dir.to_string_lossy().to_ascii_lowercase()
            } else {
                dir.to_string_lossy().into_owned()
            };
            seen.insert(key)
        })
        .collect()
}

// --- remembered directories -----------------------------------------------

/// Directories an install revealed — an npm global prefix that is on no
/// `PATH` yet — persisted so the next launch and the daemon see them too.
#[derive(Default, Deserialize, Serialize)]
struct RememberedDirs {
    #[serde(default)]
    dirs: Vec<PathBuf>,
}

static REMEMBERED: OnceLock<RwLock<Vec<PathBuf>>> = OnceLock::new();

fn remembered_store() -> &'static RwLock<Vec<PathBuf>> {
    REMEMBERED.get_or_init(|| RwLock::new(load_remembered_dirs()))
}

fn remembered_path() -> Option<PathBuf> {
    crate::brand::data_dir().map(|dir| dir.join("search-dirs.json"))
}

fn load_remembered_dirs() -> Vec<PathBuf> {
    remembered_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<RememberedDirs>(&raw).ok())
        .map(|stored| stored.dirs)
        .unwrap_or_default()
}

/// Directories learned from earlier installs, in the order they were added.
pub fn remembered_dirs() -> Vec<PathBuf> {
    remembered_store()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Record `dir` for this process and every later one. Idempotent.
pub fn remember_search_dir(dir: &Path) {
    let mut store = remembered_store()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if remember_in(&mut store, dir) {
        persist_remembered(remembered_path().as_deref(), &store);
    }
}

/// Add `dir` to `store` unless it is already there; reports whether it was.
fn remember_in(store: &mut Vec<PathBuf>, dir: &Path) -> bool {
    if store.iter().any(|known| known == dir) {
        return false;
    }
    store.push(dir.to_path_buf());
    true
}

fn persist_remembered(path: Option<&Path>, dirs: &[PathBuf]) {
    if let Some(path) = path
        && let Ok(encoded) = serde_json::to_string_pretty(&RememberedDirs {
            dirs: dirs.to_vec(),
        })
    {
        let _ = crate::global_config::atomic_write_private(path, encoded.as_bytes());
    }
}

// --- locating executables -------------------------------------------------

/// Find `name` in `dirs`, honouring `PATHEXT` on Windows.
///
/// On Windows the suffixed names are tried before the bare one: a global npm
/// install writes an extensionless POSIX shim beside `claude.cmd`, and
/// `CreateProcess` cannot run that shim. Same order as waku-core.
pub fn find_executable_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let candidates = executable_candidates(name);
    dirs.iter().find_map(|directory| {
        candidates
            .iter()
            .map(|candidate| directory.join(candidate))
            .find(|full| is_executable_file(full))
    })
}

/// Names to try for `name`: on Windows each `PATHEXT` suffix first, then the
/// bare name; elsewhere just the name.
pub(crate) fn executable_candidates(name: &str) -> Vec<String> {
    if !cfg!(target_os = "windows") {
        return vec![name.to_owned()];
    }
    let extensions = std::env::var("PATHEXT")
        .ok()
        .filter(|configured| !configured.trim().is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_owned());
    let mut candidates: Vec<String> = extensions
        .split(';')
        .map(str::trim)
        .filter(|extension| extension.starts_with('.'))
        .map(|extension| format!("{name}{}", extension.to_ascii_lowercase()))
        .collect();
    candidates.push(name.to_owned());
    candidates
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

// --- running with a timeout ------------------------------------------------

/// What a bounded command run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRun {
    pub success: bool,
    pub timed_out: bool,
    /// Combined stdout and stderr, in arrival order per stream.
    pub output: String,
}

/// Run `command` to completion or `timeout`, whichever comes first, draining
/// both pipes meanwhile so a chatty child never blocks on a full buffer.
///
/// On timeout the whole process tree is killed: a `.cmd` shim runs Node as a
/// grandchild, and killing only `cmd.exe` would leave that Node — and the
/// hang — behind.
pub fn run_with_timeout(mut command: Command, timeout: Duration) -> CommandRun {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    detach_console(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CommandRun {
                success: false,
                timed_out: false,
                output: format!("could not start the command: {error}"),
            };
        }
    };
    let stdout = child.stdout.take().map(drain_in_background);
    let stderr = child.stderr.take().map(drain_in_background);

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    kill_tree(&mut child);
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };

    let mut output = stdout
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if let Some(errors) = stderr.and_then(|handle| handle.join().ok())
        && !errors.trim().is_empty()
    {
        if !output.trim().is_empty() {
            output.push('\n');
        }
        output.push_str(&errors);
    }
    if timed_out {
        if !output.trim().is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("timed out after {}s", timeout.as_secs()));
    }
    CommandRun {
        success: status.is_some_and(|status| status.success()),
        timed_out,
        output: output.trim().to_owned(),
    }
}

fn drain_in_background<R: Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

fn kill_tree(child: &mut std::process::Child) {
    #[cfg(target_os = "windows")]
    {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/PID", &child.id().to_string(), "/T", "/F"]);
        detach_console(&mut taskkill);
        let _ = taskkill
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

pub(crate) fn detach_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

/// The `PATH` value a probe or install child runs with: `dirs` first, then
/// whatever this process inherited, deduplicated.
pub fn child_path(dirs: &[PathBuf]) -> OsString {
    let mut all: Vec<PathBuf> = dirs.to_vec();
    if let Some(inherited) = std::env::var_os("PATH") {
        all.extend(std::env::split_paths(&inherited));
    }
    let mut seen = HashSet::new();
    all.retain(|dir| seen.insert(dir.clone()));
    std::env::join_paths(all).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// A command that runs `path` with `args`, going through `cmd.exe` for
/// Windows batch shims, which `CreateProcess` cannot start directly.
pub(crate) fn command_for(path: &Path, args: &[&str], child_dirs: &[PathBuf]) -> Command {
    let mut command = if is_windows_command_script(path) {
        let mut command = Command::new("cmd");
        command.args(["/D", "/S", "/C"]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt as _;
            let mut line = format!("call \"{}\"", path.display());
            for arg in args {
                line.push(' ');
                line.push_str(arg);
            }
            command.raw_arg(line);
        }
        #[cfg(not(target_os = "windows"))]
        {
            command.arg(path).args(args);
        }
        command
    } else {
        let mut command = Command::new(path);
        command.args(args);
        command
    };
    command.env("PATH", child_path(child_dirs));
    command
}

fn is_windows_command_script(path: &Path) -> bool {
    cfg!(target_os = "windows")
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            })
}

// --- probing --------------------------------------------------------------

/// Run `path --version` and classify the answer.
pub fn probe_version(path: &Path, child_dirs: &[PathBuf], timeout: Duration) -> Probe {
    let run = run_with_timeout(command_for(path, &["--version"], child_dirs), timeout);
    if run.success {
        let version = parse_semantic_version(&run.output)
            .map(|version| {
                // Keep the CLI's own spelling when it is just the version.
                let first = run.output.lines().next().unwrap_or_default().trim();
                if first.starts_with('v') && first[1..] == version {
                    first.to_owned()
                } else {
                    version
                }
            })
            .or_else(|| {
                let first = run.output.lines().next()?.trim();
                (!first.is_empty()).then(|| first.to_owned())
            });
        return match version {
            Some(version) => Probe::Found {
                path: path.to_path_buf(),
                version,
            },
            None => Probe::FoundButFailed {
                path: path.to_path_buf(),
                diagnostic: "answered `--version` with no output".to_owned(),
            },
        };
    }
    let diagnostic = cli_install::last_meaningful_lines(&run.output, 4);
    Probe::FoundButFailed {
        path: path.to_path_buf(),
        diagnostic: if diagnostic.is_empty() {
            "exited with an error and no output".to_owned()
        } else {
            diagnostic
        },
    }
}

/// Find `name` in `dirs` and probe it. `child_dirs` is what the probe runs
/// with; passing the same list lets a shim find its runtime.
pub fn probe_named(name: &str, dirs: &[PathBuf], child_dirs: &[PathBuf]) -> Probe {
    match find_executable_in(name, dirs) {
        Some(path) => probe_version(&path, child_dirs, PROBE_TIMEOUT),
        None => Probe::NotFound,
    }
}

/// What one detection pass found, at one point in time.
#[derive(Clone, Debug)]
pub struct EnvironmentSnapshot {
    pub node: Probe,
    pub npm: Probe,
    pub clis: Vec<Detection>,
    pub plan: Vec<SetupStep>,
    pub conflicts: Vec<EnvConflict>,
    /// The directories that were searched, for installs to reuse.
    pub dirs: Vec<PathBuf>,
}

impl EnvironmentSnapshot {
    pub fn detection(&self, id: &str) -> Option<&Detection> {
        self.clis.iter().find(|detection| detection.id == id)
    }

    /// The directory of the Node that answered, for anchoring npm.
    pub fn node_bin_dir(&self) -> Option<PathBuf> {
        self.node
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }
}

/// One blocking detection pass over `dirs`. Spawns up to five processes,
/// each bounded by [`PROBE_TIMEOUT`] — run it off the UI thread.
pub fn snapshot(dirs: &[PathBuf]) -> EnvironmentSnapshot {
    let node = probe_named("node", dirs, dirs);
    // A CLI shim needs *that* Node first on its PATH, not whichever one the
    // inherited PATH happens to reach.
    let mut child_dirs: Vec<PathBuf> = Vec::new();
    if let Some(node_dir) = node.path().and_then(Path::parent) {
        child_dirs.push(node_dir.to_path_buf());
    }
    child_dirs.extend(dirs.iter().cloned());
    let npm = probe_named(cli_install::npm_binary_name(), dirs, &child_dirs);
    let clis: Vec<Detection> = cli_install::DESCRIPTORS
        .iter()
        .map(|descriptor| Detection {
            id: descriptor.id,
            probe: probe_named(descriptor.binary, dirs, &child_dirs),
        })
        .collect();
    let plan = cli_install::setup_plan(node.version(), &clis);
    EnvironmentSnapshot {
        node,
        npm,
        clis,
        plan,
        conflicts: env_conflicts::scan(),
        dirs: dirs.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-cli-detect-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// A runnable script: `.cmd` on Windows, an executable shell script
    /// elsewhere. `body` is the platform-appropriate script text.
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = if cfg!(target_os = "windows") {
            dir.join(format!("{name}.cmd"))
        } else {
            dir.join(name)
        };
        std::fs::write(&path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn echo_version(version: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("@echo off\r\necho {version}\r\n")
        } else {
            format!("#!/bin/sh\necho {version}\n")
        }
    }

    fn fail_with(message: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("@echo off\r\necho {message} 1>&2\r\nexit /b 2\r\n")
        } else {
            format!("#!/bin/sh\necho {message} >&2\nexit 2\n")
        }
    }

    fn hang() -> String {
        if cfg!(target_os = "windows") {
            "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n".to_owned()
        } else {
            "#!/bin/sh\nsleep 30\n".to_owned()
        }
    }

    #[test]
    fn version_manager_dirs_reads_env_overrides_and_skips_missing() {
        let root = scratch("vm-dirs");
        let home = root.join("home");
        let custom_pnpm = root.join("pnpm-home");
        std::fs::create_dir_all(&custom_pnpm).unwrap();
        let env = |name: &str| -> Option<OsString> {
            match name {
                "PNPM_HOME" => Some(custom_pnpm.clone().into_os_string()),
                _ => None,
            }
        };

        if cfg!(target_os = "windows") {
            let dirs = version_manager_dirs(Some(&home), &env);
            assert!(dirs.contains(&custom_pnpm));
            // Nothing else exists under this home, so nothing else is listed.
            assert!(dirs.iter().all(|dir| dir.is_dir()));
        } else {
            let old = home.join(".nvm/versions/node/v20.1.0/bin");
            let new = home.join(".nvm/versions/node/v22.4.0/bin");
            std::fs::create_dir_all(&old).unwrap();
            std::fs::create_dir_all(&new).unwrap();
            std::fs::create_dir_all(home.join(".npm-global/bin")).unwrap();
            let dirs = version_manager_dirs(Some(&home), &env);
            let position = |needle: &Path| dirs.iter().position(|dir| dir == needle);
            assert!(position(&new) < position(&old), "newest nvm version first: {dirs:?}");
            assert!(position(&old) < position(&home.join(".npm-global/bin")));
            assert!(dirs.contains(&custom_pnpm));
            assert!(!dirs.iter().any(|dir| dir.ends_with(".volta/bin")));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn find_executable_in_prefers_a_windows_shim_over_the_posix_one() {
        let dir = scratch("find");
        std::fs::write(dir.join("claude"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(dir.join("claude"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        std::fs::write(dir.join("claude.cmd"), "@echo off\r\n").unwrap();

        let found = find_executable_in("claude", &[dir.clone()]).expect("found");
        if cfg!(target_os = "windows") {
            assert_eq!(found, dir.join("claude.cmd"));
        } else {
            assert_eq!(found, dir.join("claude"));
        }
        assert!(find_executable_in("codex", &[dir.clone()]).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn probe_version_reports_found_failed_and_timeout() {
        let dir = scratch("probe");
        let good = script(&dir, "good", &echo_version("v22.11.0"));
        let bad = script(&dir, "bad", &fail_with("node: not found"));
        let slow = script(&dir, "slow", &hang());

        assert_eq!(
            probe_version(&good, &[], PROBE_TIMEOUT),
            Probe::Found {
                path: good.clone(),
                version: "v22.11.0".to_owned()
            }
        );
        match probe_version(&bad, &[], PROBE_TIMEOUT) {
            Probe::FoundButFailed { path, diagnostic } => {
                assert_eq!(path, bad);
                assert!(diagnostic.contains("not found"), "{diagnostic}");
            }
            other => panic!("expected a failed probe, got {other:?}"),
        }
        let started = Instant::now();
        match probe_version(&slow, &[], Duration::from_secs(1)) {
            Probe::FoundButFailed { diagnostic, .. } => {
                assert!(diagnostic.contains("timed out"), "{diagnostic}");
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(probe_named("absent", &[dir.clone()], &[]), Probe::NotFound);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn run_with_timeout_drains_large_output() {
        let dir = scratch("drain");
        let body = if cfg!(target_os = "windows") {
            "@echo off\r\nfor /L %%i in (1,1,3000) do echo line %%i 0123456789012345678901234567890123456789\r\n".to_owned()
        } else {
            "#!/bin/sh\ni=0\nwhile [ $i -lt 3000 ]; do echo line $i 0123456789012345678901234567890123456789; i=$((i+1)); done\n".to_owned()
        };
        let chatty = script(&dir, "chatty", &body);
        let run = run_with_timeout(command_for(&chatty, &[], &[]), PROBE_TIMEOUT);
        assert!(run.success, "{}", run.output);
        assert!(!run.timed_out);
        assert!(run.output.contains("line 2999"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn executable_candidates_put_suffixed_names_first_on_windows() {
        let candidates = executable_candidates("claude");
        assert_eq!(candidates.last().map(String::as_str), Some("claude"));
        if cfg!(target_os = "windows") {
            assert!(candidates.iter().any(|candidate| candidate == "claude.cmd"));
            assert_ne!(candidates.first().map(String::as_str), Some("claude"));
        } else {
            assert_eq!(candidates, vec!["claude".to_owned()]);
        }
    }

    #[test]
    fn remembered_dirs_are_deduplicated_and_round_trip_through_the_file() {
        let dir = scratch("remember");
        let file = dir.join("search-dirs.json");
        let mut store = Vec::new();
        assert!(remember_in(&mut store, &dir.join("a")));
        assert!(!remember_in(&mut store, &dir.join("a")));
        assert!(remember_in(&mut store, &dir.join("b")));
        persist_remembered(Some(&file), &store);
        let restored: RememberedDirs =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(restored.dirs, vec![dir.join("a"), dir.join("b")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn child_path_puts_the_given_dirs_first_without_duplicates() {
        let dir = scratch("child-path");
        let joined = child_path(&[dir.clone(), dir.clone()]);
        let parts: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(parts.first(), Some(&dir));
        assert_eq!(parts.iter().filter(|part| **part == dir).count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
