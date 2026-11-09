//! Generic executable lookup and bounded process probes.
//!
//! These utilities are shared by the optional Computer Use driver and MCP
//! processes. They do not detect, install, or configure agent CLIs.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::cli_install::{last_meaningful_lines, parse_semantic_version};

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const DRAIN_GRACE: Duration = Duration::from_secs(2);
const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    Found { path: PathBuf, version: String },
    FoundButFailed { path: PathBuf, diagnostic: String },
    NotFound,
}

impl Probe {
    pub fn version(&self) -> Option<&str> {
        match self {
            Probe::Found { version, .. } => Some(version),
            _ => None,
        }
    }
}

/// The user's login-shell PATH followed by the process PATH. The shell probe
/// is cached and must only be reached from background work.
pub fn default_search_dirs() -> Vec<PathBuf> {
    let mut dirs = shell_path_dirs();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dedup(dirs)
}

fn shell_path_dirs() -> Vec<PathBuf> {
    static SHELL_PATH: OnceLock<Vec<PathBuf>> = OnceLock::new();
    SHELL_PATH.get_or_init(resolve_shell_path_dirs).clone()
}

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

pub fn find_executable_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let candidates = executable_candidates(name);
    dirs.iter().find_map(|directory| {
        candidates
            .iter()
            .map(|candidate| directory.join(candidate))
            .find(|full| is_executable_file(full))
    })
}

fn executable_candidates(name: &str) -> Vec<String> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRun {
    pub success: bool,
    pub timed_out: bool,
    pub output: String,
}

/// Run a command with drained output pipes and kill its process tree on
/// timeout. Always called from background work.
pub fn run_with_timeout(mut command: Command, timeout: Duration) -> CommandRun {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    detach_console(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
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
            Ok(None) if started.elapsed() >= timeout => {
                timed_out = true;
                kill_tree(&mut child);
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };

    let collect = |drain: Option<Receiver<String>>| -> String {
        let Some(drain) = drain else {
            return String::new();
        };
        if timed_out {
            drain.recv_timeout(DRAIN_GRACE).unwrap_or_default()
        } else {
            drain.recv().unwrap_or_default()
        }
    };
    let mut output = collect(stdout);
    let errors = collect(stderr);
    if !errors.trim().is_empty() {
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

fn drain_in_background<R: Read + Send + 'static>(mut reader: R) -> Receiver<String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receiver
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
    #[cfg(unix)]
    {
        // SAFETY: signal the process group created by run_with_timeout.
        unsafe {
            libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
        }
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

fn child_path(dirs: &[PathBuf]) -> OsString {
    let mut all: Vec<PathBuf> = dirs.to_vec();
    if let Some(inherited) = std::env::var_os("PATH") {
        all.extend(std::env::split_paths(&inherited));
    }
    std::env::join_paths(dedup(all))
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

fn command_for(path: &Path, args: &[&str], child_dirs: &[PathBuf]) -> Command {
    let mut command = if cfg!(target_os = "windows")
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            }) {
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
        command
    } else {
        let mut command = Command::new(path);
        command.args(args);
        command
    };
    command.env("PATH", child_path(child_dirs));
    command
}

pub fn probe_version(path: &Path, child_dirs: &[PathBuf], timeout: Duration) -> Probe {
    let run = run_with_timeout(command_for(path, &["--version"], child_dirs), timeout);
    if run.success {
        let version = parse_semantic_version(&run.output).or_else(|| {
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
    let diagnostic = last_meaningful_lines(&run.output, 4);
    Probe::FoundButFailed {
        path: path.to_path_buf(),
        diagnostic: if diagnostic.is_empty() {
            "exited with an error and no output".to_owned()
        } else {
            diagnostic
        },
    }
}
