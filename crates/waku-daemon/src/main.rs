use std::io::Write as _;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use waku_protocol::{DAEMON_TOKEN_ENV, DaemonReady, PROTOCOL_VERSION};

fn main() -> anyhow::Result<()> {
    // Fork: rename legacy upstream-named storage (~/.waku etc.) before the
    // settings and task stores resolve their paths. The desktop runs the same
    // migration, but the daemon can also start standalone.
    for warning in sub2api::migrate::migrate_legacy_storage() {
        eprintln!("warning: {warning}");
    }
    // Fork: the engine's warnings and any panic go to a log file beside the
    // state database. After the migration, which would otherwise find the
    // new data directory already there.
    let state_path = waku_core::persistence::StateStore::default_path();
    logging::init(&state_path.with_file_name("logs"));
    let arguments = Arguments::parse(std::env::args().skip(1))?;
    let token =
        std::env::var(DAEMON_TOKEN_ENV).context("Waku daemon authentication token is missing")?;
    // The bearer capability belongs only to this server process. Remove it
    // before any provider or workspace subprocess can inherit the daemon's
    // environment.
    unsafe { std::env::remove_var(DAEMON_TOKEN_ENV) };
    let listener = TcpListener::bind(&arguments.bind)
        .with_context(|| format!("could not bind Waku daemon to {}", arguments.bind))?;
    let address = listener.local_addr()?;
    ensure_bind_allowed(address, arguments.allow_non_loopback)?;
    let ready = DaemonReady {
        address: address.to_string(),
        protocol_version: PROTOCOL_VERSION,
        pid: std::process::id(),
    };
    println!("{}", serde_json::to_string(&ready)?);
    std::io::stdout().flush()?;

    let shutdown = Arc::new(AtomicBool::new(false));
    if let Some(parent_pid) = arguments.parent_pid {
        let monitor_shutdown = shutdown.clone();
        std::thread::Builder::new()
            .name("waku-daemon-parent".into())
            .spawn(move || {
                while !monitor_shutdown.load(Ordering::Acquire) {
                    if !process_is_alive(parent_pid) {
                        monitor_shutdown.store(true, Ordering::Release);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })?;
    }

    let task_path = waku_core::persistence::StateStore::default_path();
    let settings = waku_core::DaemonSettingsStore::open_with_legacy(
        waku_core::DaemonSettings::default_path(),
        [task_path.with_file_name("settings.json")],
    )
    .context("could not load daemon settings")?;
    let task_store = waku_core::persistence::StateStore::daemon(task_path);
    waku_core::serve(
        listener,
        token,
        Arc::new(waku_core::daemon::WakuBackend::new(settings, task_store)?),
        shutdown,
        waku_core::ServerOptions {
            allowed_origins: arguments.allowed_origins.into_iter().collect(),
            allow_shutdown: arguments.parent_pid.is_some(),
        },
    )
}

/// Fork addition: written inline rather than as `logging.rs` because
/// `waku-debug-daemon` `include!`s this file from `src/bin`, where a `mod`
/// declaration would look for its file.
mod logging {
    //! Fork addition: the daemon's log file.
    //!
    //! The built-in agent's engine and bridge report through `tracing` — retries,
    //! MCP servers that would not start or calls that timed out, a transcript
    //! that could not be read, a permission rule that could not be saved — and
    //! nothing installed a subscriber, so all of it was dropped. This writes
    //! warnings and errors to `logs/daemon.log` beside the state database, and
    //! panics with them: the daemon's stderr is the desktop's, where nobody reads
    //! it.

    use std::fs::{self, File, OpenOptions};
    use std::path::Path;
    use std::sync::Mutex;

    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::prelude::*;

    /// A log past this size is moved aside to `daemon.log.1` when the daemon
    /// starts, replacing the one before, so at most two runs' worth is kept.
    const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;

    /// Overrides the filter, in `tracing_subscriber`'s `Targets` syntax:
    /// `info`, or `warn,claurst_query=debug`.
    const FILTER_ENV: &str = "WAKU_LOG";

    /// Install the subscriber and the panic hook. Never fails the daemon: with no
    /// log file it runs as it did before, unlogged.
    pub fn init(directory: &Path) {
        let Some(file) = open(directory) else {
            return;
        };
        let filter = std::env::var(FILTER_ENV)
            .ok()
            .and_then(|value| value.parse::<Targets>().ok())
            .unwrap_or_else(|| Targets::new().with_default(LevelFilter::WARN));
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .with_target(true);
        if tracing_subscriber::registry()
            .with(layer.with_filter(filter))
            .try_init()
            .is_err()
        {
            return;
        }
        log_panics();
    }

    fn open(directory: &Path) -> Option<File> {
        fs::create_dir_all(directory).ok()?;
        let path = directory.join("daemon.log");
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > ROTATE_AT_BYTES) {
            let _ = fs::rename(&path, directory.join("daemon.log.1"));
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        // Errors quote what the agent was doing; keep them the owner's, as the
        // transcripts are.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(&path).ok()
    }

    /// A panic anywhere in the daemon — a tool, the engine, a driver — goes to
    /// the log with its location before the default hook prints it.
    fn log_panics() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(target: "panic", "{info}");
            previous(info);
        }));
    }
}

fn ensure_bind_allowed(address: SocketAddr, allow_non_loopback: bool) -> anyhow::Result<()> {
    if address.ip().is_loopback() || allow_non_loopback {
        return Ok(());
    }
    bail!(
        "refusing non-loopback daemon bind {address}; pass --allow-non-loopback only after configuring authentication and exact browser origins"
    )
}

struct Arguments {
    bind: String,
    parent_pid: Option<u32>,
    allowed_origins: Vec<String>,
    allow_non_loopback: bool,
}

impl Arguments {
    fn parse(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut bind = "127.0.0.1:0".to_owned();
        let mut parent_pid = None;
        let mut allowed_origins = Vec::new();
        let mut allow_non_loopback = false;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--bind" => {
                    bind = arguments
                        .next()
                        .ok_or_else(|| anyhow!("--bind requires an address"))?;
                }
                "--parent-pid" => {
                    parent_pid = Some(
                        arguments
                            .next()
                            .ok_or_else(|| anyhow!("--parent-pid requires a process id"))?
                            .parse()
                            .context("--parent-pid is not a valid process id")?,
                    );
                }
                "--allow-origin" => {
                    let origin = arguments
                        .next()
                        .filter(|origin| !origin.trim().is_empty())
                        .ok_or_else(|| anyhow!("--allow-origin requires an origin"))?;
                    allowed_origins.push(origin);
                }
                "--allow-non-loopback" => {
                    allow_non_loopback = true;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: {} [--bind ADDRESS] [--allow-non-loopback] [--parent-pid PID] [--allow-origin ORIGIN]...",
                        env!("CARGO_BIN_NAME")
                    );
                    std::process::exit(0);
                }
                unknown => bail!("unknown argument {unknown:?}"),
            }
        }
        Ok(Self {
            bind,
            parent_pid,
            allowed_origins,
            allow_non_loopback,
        })
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Windows reuses process ids, so the handle is opened for the narrowest
/// right that answers the question and closed immediately. A pid that no
/// longer exists fails to open; one that has exited but is still held open by
/// another handle reports an exit code instead of `STILL_ACTIVE`.
#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Read before any other call can overwrite the thread's last error.
            let open_error = std::io::Error::last_os_error().raw_os_error();
            return !parent_is_gone(open_error);
        }
        let mut exit_code = 0_u32;
        let read = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        // A failed read leaves the parent's state unknown; outliving the app
        // is the safer error than shutting a live daemon down.
        read == 0 || exit_code == STILL_ACTIVE as u32
    }
}

/// Only "no such process" means the parent is gone. `ERROR_ACCESS_DENIED`
/// and every other `OpenProcess` failure leave its state unknown, and
/// outliving the app is the safer error than shutting a live daemon down —
/// the same call the Unix branch makes for `EPERM`.
#[cfg(windows)]
fn parent_is_gone(open_error: Option<i32>) -> bool {
    use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;

    open_error == Some(ERROR_INVALID_PARAMETER as i32)
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_loopback_listener_requires_an_explicit_flag() {
        assert!(ensure_bind_allowed("127.0.0.1:3000".parse().unwrap(), false).is_ok());
        assert!(ensure_bind_allowed("[::1]:3000".parse().unwrap(), false).is_ok());
        assert!(ensure_bind_allowed("0.0.0.0:3000".parse().unwrap(), false).is_err());
        assert!(ensure_bind_allowed("[::]:3000".parse().unwrap(), false).is_err());
        assert!(ensure_bind_allowed("0.0.0.0:3000".parse().unwrap(), true).is_ok());
    }

    #[test]
    fn parses_repeated_browser_origin_allowlist_entries() {
        let arguments = Arguments::parse([
            "--allow-origin".into(),
            "https://app.waku.test".into(),
            "--allow-origin".into(),
            "http://localhost:3000".into(),
        ])
        .unwrap();

        assert_eq!(
            arguments.allowed_origins,
            ["https://app.waku.test", "http://localhost:3000"]
        );
        assert!(!arguments.allow_non_loopback);
    }

    #[test]
    fn parses_explicit_non_loopback_opt_in() {
        let arguments = Arguments::parse(["--allow-non-loopback".into()]).unwrap();
        assert!(arguments.allow_non_loopback);
    }

    #[cfg(windows)]
    #[test]
    fn only_a_missing_process_counts_as_a_dead_parent() {
        // ERROR_INVALID_PARAMETER: the pid names no process.
        assert!(parent_is_gone(Some(87)));
        // ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, or no code at all: unknown.
        assert!(!parent_is_gone(Some(5)));
        assert!(!parent_is_gone(Some(6)));
        assert!(!parent_is_gone(None));
    }

    #[cfg(windows)]
    #[test]
    fn liveness_probe_answers_for_real_processes() {
        assert!(process_is_alive(std::process::id()));
        // The System Idle Process cannot be opened; OpenProcess reports
        // ERROR_INVALID_PARAMETER for it, which is the "gone" answer.
        assert!(!process_is_alive(0));
        // The System process may refuse the handle; refusal means alive.
        assert!(process_is_alive(4));
    }
}
