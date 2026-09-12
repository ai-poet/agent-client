use std::collections::HashSet;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::SystemTime;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DaemonClient;
use waku_protocol::{
    APP_EXECUTABLE_ENV, Command, DAEMON_TOKEN_ENV, DaemonReady, DaemonSettings, PROTOCOL_VERSION,
    ReplayCursor, ResponsePayload,
};
const START_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const REBUILD_POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const DEFAULT_EXPOSED_DAEMON_PORT: u16 = 34_123;
/// Refused redials against one dropped connection before the supervisor gives
/// up on the process and replaces it: six polls, about three seconds.
const LOCAL_RECONNECT_ATTEMPTS: u32 = 6;
/// A socket that drops again this soon after a redial is being killed by
/// something a redial cannot fix (an oversized message, say), so the next
/// dial waits instead of hammering the daemon.
const REDIAL_STORM_WINDOW: Duration = Duration::from_secs(5);

/// Desktop-owned launch configuration for the daemon it supervises.
///
/// Provider settings belong to the daemon and live in `settings.json`; this
/// is an app preference because it controls how the desktop launches its own
/// child process. The bearer token is intentionally stable across daemon-only
/// rebuilds and desktop relaunches so a configured web client keeps working.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct DaemonExposureSettings {
    pub enabled: bool,
    pub port: u16,
    pub allowed_origins: Vec<String>,
    pub token: String,
}

impl Default for DaemonExposureSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_EXPOSED_DAEMON_PORT,
            allowed_origins: vec!["http://localhost:3001".into()],
            token: Self::new_token(),
        }
    }
}

impl DaemonExposureSettings {
    pub fn new_token() -> String {
        Uuid::new_v4().simple().to_string()
    }

    pub fn ensure_token(&mut self) -> bool {
        if !self.token.trim().is_empty() {
            return false;
        }
        self.token = Self::new_token();
        true
    }

    pub fn allowed_origins_text(&self) -> String {
        self.allowed_origins.join(", ")
    }

    pub fn with_allowed_origins_text(mut self, text: &str) -> anyhow::Result<Self> {
        self.allowed_origins = parse_allowed_origins(text)?;
        Ok(self)
    }

    pub fn validate(mut self) -> anyhow::Result<Self> {
        if self.port == 0 {
            bail!("daemon port must be between 1 and 65535");
        }
        if self.token.trim().is_empty() {
            bail!("daemon authentication token is empty");
        }
        self.allowed_origins = parse_allowed_origins(&self.allowed_origins_text())?;
        Ok(self)
    }

    fn bind_address(&self) -> String {
        if self.enabled {
            format!("0.0.0.0:{}", self.port)
        } else {
            "127.0.0.1:0".into()
        }
    }
}

/// Parse the comma-separated exact browser origins edited by the desktop.
/// Browser Origin headers contain only an HTTP(S) origin, never a path.
pub fn parse_allowed_origins(text: &str) -> anyhow::Result<Vec<String>> {
    let mut origins = Vec::new();
    let mut seen = HashSet::new();
    for candidate in text
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let url = url::Url::parse(candidate)
            .with_context(|| format!("invalid browser origin {candidate:?}"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!(
                "browser origin {candidate:?} must be an exact http:// or https:// origin without a path"
            );
        }
        let origin = url.origin().ascii_serialization();
        if origin == "null" {
            bail!("browser origin {candidate:?} is not a network origin");
        }
        if seen.insert(origin.clone()) {
            origins.push(origin);
        }
    }
    Ok(origins)
}

pub struct DaemonProcess {
    client: DaemonClient,
    child: Child,
    /// Loopback endpoint the desktop dialed. Kept so the supervisor can reopen
    /// the socket to a process that is still running instead of replacing it.
    client_address: String,
    token: String,
}

impl DaemonProcess {
    pub fn spawn(executable: &Path) -> anyhow::Result<Self> {
        Self::spawn_configured(executable, DaemonExposureSettings::default())
    }

    fn spawn_configured(
        executable: &Path,
        settings: DaemonExposureSettings,
    ) -> anyhow::Result<Self> {
        let settings = settings.validate()?;
        let token = settings.token.clone();
        let app_executable = std::env::current_exe().context("could not locate Waku executable")?;
        let mut command = ProcessCommand::new(executable);
        // The desktop is a GUI-subsystem binary on Windows, so a console
        // child would get a console window of its own. `stderr` still reaches
        // the app's inherited handle.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;

            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
            .arg("--bind")
            .arg(settings.bind_address())
            .arg("--parent-pid")
            .arg(std::process::id().to_string());
        if settings.enabled {
            command.arg("--allow-non-loopback");
        }
        for origin in &settings.allowed_origins {
            command.arg("--allow-origin").arg(origin);
        }
        let mut child = command
            .env(DAEMON_TOKEN_ENV, &token)
            .env(APP_EXECUTABLE_ENV, app_executable)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("could not launch {}", executable.display()))?;
        let stdout = child
            .stdout
            .take()
            .context("Waku daemon did not expose its readiness stream")?;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("waku-daemon-ready".into())
            .spawn(move || {
                let mut line = String::new();
                let result = BufReader::new(stdout)
                    .read_line(&mut line)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| {
                        if bytes == 0 {
                            bail!("Waku daemon exited before becoming ready")
                        }
                        serde_json::from_str::<DaemonReady>(&line).map_err(anyhow::Error::from)
                    });
                let _ = ready_tx.send(result);
            })
            .context("could not start Waku daemon readiness reader")?;
        let ready = match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(ready)) => ready,
            Ok(Err(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("timed out waiting for Waku daemon: {error}");
            }
        };
        if ready.protocol_version != PROTOCOL_VERSION {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "daemon protocol {} does not match desktop protocol {}",
                ready.protocol_version,
                PROTOCOL_VERSION
            );
        }
        let client_address = match desktop_client_address(&ready.address) {
            Ok(address) => address,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let client = match DaemonClient::connect(&client_address, token.clone()) {
            Ok(client) => client,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            client,
            child,
            client_address,
            token,
        })
    }

    pub fn client(&self) -> DaemonClient {
        self.client.clone()
    }

    /// Address and bearer token for redialing this process.
    fn endpoint(&self) -> (String, String) {
        (self.client_address.clone(), self.token.clone())
    }

    /// Install a replacement connection to the same process.
    fn adopt_client(&mut self, client: DaemonClient) {
        self.client = client;
    }

    #[cfg(test)]
    fn for_test(client: DaemonClient, child: Child, client_address: String, token: String) -> Self {
        Self {
            client,
            child,
            client_address,
            token,
        }
    }

    fn has_exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    fn stop(&mut self) {
        self.client.shutdown();
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn desktop_client_address(address: &str) -> anyhow::Result<String> {
    let address = address
        .parse::<std::net::SocketAddr>()
        .with_context(|| format!("Waku daemon returned an invalid address {address:?}"))?;
    let ip = if address.ip().is_unspecified() {
        if address.is_ipv4() {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        } else {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        }
    } else {
        address.ip()
    };
    Ok(std::net::SocketAddr::new(ip, address.port()).to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExecutableStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl ExecutableStamp {
    fn read(path: &Path) -> anyhow::Result<Self> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("could not inspect {}", path.display()))?;
        Ok(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

struct SupervisorInner {
    executable: Option<PathBuf>,
    target: Mutex<DaemonTarget>,
    exposure: Mutex<Option<DaemonExposureSettings>>,
    restart: Mutex<()>,
    settings: Mutex<DaemonSettings>,
    persisted_settings: Mutex<Option<DaemonSettings>>,
    settings_updates: Sender<DaemonSettings>,
    client_updates: Mutex<Vec<Sender<DaemonClient>>>,
    running: AtomicBool,
}

enum DaemonTarget {
    Local(DaemonProcess),
    Restarting(DaemonClient),
    Remote {
        client: DaemonClient,
        address: String,
        token: String,
    },
}

impl DaemonTarget {
    fn client(&self) -> DaemonClient {
        match self {
            Self::Local(process) => process.client(),
            Self::Restarting(client) => client.clone(),
            Self::Remote { client, .. } => client.clone(),
        }
    }

    /// Where to redial when this target's socket drops. `Restarting` has no
    /// endpoint: its process is already being replaced.
    fn reconnect_endpoint(&self) -> Option<(String, String)> {
        match self {
            Self::Local(process) => Some(process.endpoint()),
            Self::Remote { address, token, .. } => Some((address.clone(), token.clone())),
            Self::Restarting(_) => None,
        }
    }
}

/// Owns the current daemon and, in development, swaps it after a successful
/// rebuild without requiring the desktop process to relaunch.
#[derive(Clone)]
pub struct DaemonSupervisor {
    inner: Arc<SupervisorInner>,
}

impl DaemonSupervisor {
    pub fn spawn(executable: &Path, watch_for_rebuilds: bool) -> anyhow::Result<Self> {
        Self::spawn_configured(
            executable,
            watch_for_rebuilds,
            DaemonExposureSettings::default(),
        )
    }

    pub fn spawn_configured(
        executable: &Path,
        watch_for_rebuilds: bool,
        exposure: DaemonExposureSettings,
    ) -> anyhow::Result<Self> {
        let exposure = exposure.validate()?;
        let process = DaemonProcess::spawn_configured(executable, exposure.clone())?;
        let settings = read_settings(&process.client())?;
        let initial_stamp = ExecutableStamp::read(executable)?;
        let supervisor = Self::from_target(
            DaemonTarget::Local(process),
            Some(executable.to_owned()),
            Some(exposure),
            settings,
        )?;
        let weak_inner = Arc::downgrade(&supervisor.inner);
        std::thread::Builder::new()
            .name("waku-daemon-supervisor".into())
            .spawn(move || monitor_daemon(weak_inner, Some(initial_stamp), watch_for_rebuilds))
            .context("could not start Waku daemon supervisor")?;
        Ok(supervisor)
    }

    /// Connect to a daemon managed on another host (or by an external local
    /// service manager). Dropping the desktop never shuts this daemon down.
    pub fn connect(address: &str, token: String) -> anyhow::Result<Self> {
        let client = DaemonClient::connect(address, token.clone())?;
        let settings = read_settings(&client)?;
        let supervisor = Self::from_target(
            DaemonTarget::Remote {
                client,
                address: address.to_owned(),
                token,
            },
            None,
            None,
            settings,
        )?;
        let weak_inner = Arc::downgrade(&supervisor.inner);
        std::thread::Builder::new()
            .name("waku-remote-daemon-supervisor".into())
            .spawn(move || monitor_daemon(weak_inner, None, false))
            .context("could not start remote Waku daemon supervisor")?;
        Ok(supervisor)
    }

    fn from_target(
        target: DaemonTarget,
        executable: Option<PathBuf>,
        exposure: Option<DaemonExposureSettings>,
        settings: DaemonSettings,
    ) -> anyhow::Result<Self> {
        let (settings_updates, settings_update_rx) = unbounded();
        let inner = Arc::new(SupervisorInner {
            executable,
            target: Mutex::new(target),
            exposure: Mutex::new(exposure),
            restart: Mutex::new(()),
            settings: Mutex::new(settings),
            // The desktop sends one normalized snapshot after it has migrated
            // the legacy combined settings document into app.json.
            persisted_settings: Mutex::new(None),
            settings_updates,
            client_updates: Mutex::new(Vec::new()),
            running: AtomicBool::new(true),
        });
        let weak_inner = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("waku-daemon-settings".into())
            .spawn(move || persist_settings(weak_inner, settings_update_rx))
            .context("could not start Waku daemon settings writer")?;
        Ok(Self { inner })
    }

    pub fn client(&self) -> DaemonClient {
        self.inner.target.lock().client()
    }

    /// Subscribe to the active daemon connection. The current client is sent
    /// immediately, followed by each replacement after a managed restart.
    pub fn subscribe_clients(&self) -> Receiver<DaemonClient> {
        let (updates, receiver) = unbounded();
        // Holding the target lock through registration makes the initial send
        // atomic with respect to replacement: a subscriber sees either the old
        // client followed by the new one, or the new client directly.
        let target = self.inner.target.lock();
        self.inner.client_updates.lock().push(updates.clone());
        let _ = updates.send(target.client());
        receiver
    }

    pub fn is_remote(&self) -> bool {
        self.inner.executable.is_none()
    }

    pub fn settings(&self) -> DaemonSettings {
        self.inner.settings.lock().clone()
    }

    /// Restart only the desktop-managed daemon with a new listener policy.
    /// The caller should run this off the UI thread.
    pub fn reconfigure(&self, exposure: DaemonExposureSettings) -> anyhow::Result<()> {
        let exposure = exposure.validate()?;
        let executable = self
            .inner
            .executable
            .as_ref()
            .context("the connected daemon is managed outside Waku Desktop")?
            .clone();
        let _restart = self.inner.restart.lock();
        let previous = self
            .inner
            .exposure
            .lock()
            .clone()
            .context("managed daemon launch settings are unavailable")?;
        match replace_local_daemon(&self.inner, &executable, &exposure) {
            Ok(()) => {
                *self.inner.exposure.lock() = Some(exposure);
                queue_settings_refresh(&self.inner);
                Ok(())
            }
            Err(error) => {
                let restore = replace_local_daemon(&self.inner, &executable, &previous);
                if restore.is_ok() {
                    queue_settings_refresh(&self.inner);
                    Err(error)
                } else {
                    Err(error.context(format!(
                        "the previous daemon configuration also failed to restart: {:#}",
                        restore.unwrap_err()
                    )))
                }
            }
        }
    }

    /// Replace the desktop-managed daemon with its current launch settings.
    /// The graceful stop (up to a second) and the readiness wait (up to
    /// fifteen) run inline, so the caller should run this off the UI thread.
    pub fn restart(&self) -> anyhow::Result<()> {
        let executable = self
            .inner
            .executable
            .as_ref()
            .context("the connected daemon is managed outside Waku Desktop")?
            .clone();
        let _restart = self.inner.restart.lock();
        let exposure = self
            .inner
            .exposure
            .lock()
            .clone()
            .context("managed daemon launch settings are unavailable")?;
        replace_local_daemon(&self.inner, &executable, &exposure)?;
        queue_settings_refresh(&self.inner);
        Ok(())
    }

    /// Queue a daemon settings update without blocking the desktop UI thread.
    pub fn update_settings(&self, settings: DaemonSettings) -> anyhow::Result<()> {
        *self.inner.settings.lock() = settings.clone();
        if self.inner.persisted_settings.lock().as_ref() == Some(&settings) {
            return Ok(());
        }
        self.inner
            .settings_updates
            .send(settings)
            .map_err(|_| anyhow::anyhow!("Waku daemon settings writer is closed"))
    }
}

impl Drop for DaemonSupervisor {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.inner.running.store(false, Ordering::Release);
        }
    }
}

/// What the supervisor does about a local daemon this poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalRecovery {
    None,
    /// The process is alive; only the desktop's socket to it dropped.
    Reconnect,
    /// The process is gone, was rebuilt, or has refused the socket too long.
    Restart,
}

fn plan_local_recovery(
    process_exited: bool,
    socket_disconnected: bool,
    executable_changed: bool,
    refused_redials: u32,
) -> LocalRecovery {
    if process_exited || executable_changed || refused_redials >= LOCAL_RECONNECT_ATTEMPTS {
        LocalRecovery::Restart
    } else if socket_disconnected {
        LocalRecovery::Reconnect
    } else {
        LocalRecovery::None
    }
}

/// Redial bookkeeping for the connection the supervisor currently holds.
#[derive(Default)]
struct RedialState {
    /// The connection the counters describe; a published replacement resets
    /// them.
    connection: Option<DaemonClient>,
    /// Whether `connection` has already been seen disconnected, so one drop
    /// counts once rather than once per poll.
    drop_seen: bool,
    /// Consecutive refused dials for `connection`.
    refused: u32,
    /// When a redial last succeeded.
    last_redial: Option<Instant>,
    /// Drops that followed a redial within `REDIAL_STORM_WINDOW`.
    storm_drops: u32,
    /// The next dial waits until this instant.
    not_before: Option<Instant>,
}

impl RedialState {
    fn observe(&mut self, client: &DaemonClient, disconnected: bool, now: Instant) {
        if !self
            .connection
            .as_ref()
            .is_some_and(|tracked| tracked.same_connection(client))
        {
            self.connection = Some(client.clone());
            self.drop_seen = false;
            self.refused = 0;
        }
        if disconnected && !self.drop_seen {
            self.drop_seen = true;
            let recurring = self
                .last_redial
                .is_some_and(|at| now.duration_since(at) < REDIAL_STORM_WINDOW);
            self.storm_drops = if recurring {
                self.storm_drops.saturating_add(1)
            } else {
                0
            };
            self.not_before =
                (self.storm_drops > 0).then(|| now + redial_backoff(self.storm_drops));
        }
    }

    fn backing_off(&self, now: Instant) -> bool {
        self.not_before.is_some_and(|at| now < at)
    }

    fn redialed(&mut self, now: Instant) {
        self.refused = 0;
        self.last_redial = Some(now);
        self.not_before = None;
    }

    fn dial_refused(&mut self) {
        self.refused = self.refused.saturating_add(1);
    }

    fn restarted(&mut self) {
        *self = Self::default();
    }
}

/// One, two, four, then eight seconds.
fn redial_backoff(storm_drops: u32) -> Duration {
    Duration::from_secs(1 << storm_drops.saturating_sub(1).min(3))
}

/// Install a replacement connection and fan it out to subscribers. The target
/// lock is held across the fan-out so this stays atomic with respect to
/// `subscribe_clients`: a subscriber sees the previous client followed by this
/// one, or this one alone, never this one twice. Lock order everywhere is
/// `restart`, then `target`, then `client_updates`.
fn publish_client(
    inner: &SupervisorInner,
    client: DaemonClient,
    install: impl FnOnce(&mut DaemonTarget),
) {
    let mut target = inner.target.lock();
    install(&mut target);
    inner
        .client_updates
        .lock()
        .retain(|subscriber| subscriber.send(client.clone()).is_ok());
}

/// Redial the target's endpoint with the dropped connection's replay cursors
/// and publish the replacement. Returns whether a replacement was installed;
/// `Ok(false)` means the target had already moved on from `disconnected`.
///
/// The restart lock is held throughout so a concurrent `reconfigure` cannot be
/// overwritten. The target lock is taken only for the check and the install,
/// never across the blocking dial: the UI thread takes it on every save.
fn reopen_socket(
    inner: &SupervisorInner,
    disconnected: &DaemonClient,
    connect: &mut impl FnMut(&str, String, Vec<ReplayCursor>) -> anyhow::Result<DaemonClient>,
) -> anyhow::Result<bool> {
    let _restart = inner.restart.lock();
    let endpoint = {
        let target = inner.target.lock();
        if !target.client().same_connection(disconnected) || !disconnected.is_disconnected() {
            return Ok(false);
        }
        target.reconnect_endpoint()
    };
    let Some((address, token)) = endpoint else {
        return Ok(false);
    };
    let replacement = connect(&address, token, disconnected.last_sequences())?;
    publish_client(inner, replacement.clone(), |target| match target {
        DaemonTarget::Local(process) => process.adopt_client(replacement),
        DaemonTarget::Remote { client, .. } => *client = replacement,
        // Every path that produces `Restarting` holds the restart lock, so
        // the target cannot have changed since the check above.
        DaemonTarget::Restarting(_) => {}
    });
    Ok(true)
}

fn monitor_daemon(
    weak_inner: std::sync::Weak<SupervisorInner>,
    mut active_stamp: Option<ExecutableStamp>,
    watch_for_rebuilds: bool,
) {
    let mut redials = RedialState::default();
    let mut dial = |address: &str, token: String, resume_from: Vec<ReplayCursor>| {
        DaemonClient::connect_with_resume(address, token, resume_from)
    };
    loop {
        std::thread::sleep(REBUILD_POLL_INTERVAL);
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };
        if !inner.running.load(Ordering::Acquire) {
            return;
        }
        // Snapshot under the target lock. The dial, the spawn and the process
        // drop below all run with it released: the UI thread takes this lock
        // on every save through `DaemonSupervisor::client`.
        let (client, process_exited) = {
            let mut target = inner.target.lock();
            let process_exited = match &mut *target {
                DaemonTarget::Local(process) => process.has_exited(),
                DaemonTarget::Restarting(_) => true,
                DaemonTarget::Remote { .. } => false,
            };
            (target.client(), process_exited)
        };
        let now = Instant::now();
        let socket_disconnected = client.is_disconnected();
        redials.observe(&client, socket_disconnected, now);
        let Some(executable) = inner.executable.as_ref() else {
            // A remote daemon is redialed for as long as the desktop runs;
            // there is no process here to replace.
            if socket_disconnected && !redials.backing_off(now) {
                match reopen_socket(&inner, &client, &mut dial) {
                    Ok(true) => redials.redialed(now),
                    Ok(false) => {}
                    Err(_) => redials.dial_refused(),
                }
            }
            continue;
        };
        let observed_stamp = ExecutableStamp::read(executable).ok();
        let executable_changed = watch_for_rebuilds
            && observed_stamp.is_some_and(|observed| Some(observed) != active_stamp);
        match plan_local_recovery(
            process_exited,
            socket_disconnected,
            executable_changed,
            redials.refused,
        ) {
            LocalRecovery::None => {}
            LocalRecovery::Reconnect => {
                if redials.backing_off(now) {
                    continue;
                }
                match reopen_socket(&inner, &client, &mut dial) {
                    Ok(true) => redials.redialed(now),
                    Ok(false) => {}
                    Err(error) => {
                        redials.dial_refused();
                        eprintln!(
                            "could not reopen the Waku daemon socket (attempt {}): {error:#}",
                            redials.refused
                        );
                    }
                }
            }
            LocalRecovery::Restart => {
                let _restart = inner.restart.lock();
                let Some(exposure) = inner.exposure.lock().clone() else {
                    return;
                };
                match replace_local_daemon(&inner, executable, &exposure) {
                    Ok(()) => redials.restarted(),
                    Err(error) => {
                        eprintln!("could not restart rebuilt Waku daemon: {error:#}");
                        continue;
                    }
                }
                queue_settings_refresh(&inner);
                if let Some(observed_stamp) = observed_stamp {
                    active_stamp = Some(observed_stamp);
                }
            }
        }
    }
}

fn replace_local_daemon(
    inner: &SupervisorInner,
    executable: &Path,
    exposure: &DaemonExposureSettings,
) -> anyhow::Result<()> {
    let previous = {
        let mut target = inner.target.lock();
        match &*target {
            DaemonTarget::Remote { .. } => {
                bail!("the connected daemon is managed outside Waku Desktop")
            }
            DaemonTarget::Restarting(_) => None,
            DaemonTarget::Local(process) => {
                let disconnected = process.client();
                let previous =
                    std::mem::replace(&mut *target, DaemonTarget::Restarting(disconnected));
                match previous {
                    DaemonTarget::Local(process) => Some(process),
                    _ => unreachable!("local daemon target changed while locked"),
                }
            }
        }
    };
    // Dropping can wait briefly for graceful shutdown, but the target lock is
    // already released so UI actions never block behind process teardown.
    drop(previous);
    let replacement = DaemonProcess::spawn_configured(executable, exposure.clone())?;
    let client = replacement.client();
    publish_client(inner, client, |target| {
        *target = DaemonTarget::Local(replacement);
    });
    Ok(())
}

fn queue_settings_refresh(inner: &SupervisorInner) {
    let settings = inner.settings.lock().clone();
    *inner.persisted_settings.lock() = None;
    let _ = inner.settings_updates.send(settings);
}

fn read_settings(client: &DaemonClient) -> anyhow::Result<DaemonSettings> {
    match client.request(Uuid::nil(), Uuid::nil(), Command::GetSettings)? {
        ResponsePayload::Settings { settings } => Ok(settings),
        _ => bail!("Waku daemon returned an invalid settings response"),
    }
}

fn persist_settings(
    weak_inner: std::sync::Weak<SupervisorInner>,
    updates: Receiver<DaemonSettings>,
) {
    while let Ok(mut settings) = updates.recv() {
        while let Ok(newer) = updates.try_recv() {
            settings = newer;
        }
        loop {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            if !inner.running.load(Ordering::Acquire) {
                return;
            }
            let desired = inner.settings.lock().clone();
            if desired != settings {
                settings = desired;
            }
            let client = inner.target.lock().client();
            let result = client.request(
                Uuid::nil(),
                Uuid::nil(),
                Command::UpdateSettings {
                    settings: settings.clone(),
                },
            );
            match result {
                Ok(ResponsePayload::Ack) => {
                    *inner.persisted_settings.lock() = Some(settings);
                    break;
                }
                Ok(_) => {
                    eprintln!("Waku daemon returned an invalid settings update response");
                }
                Err(error) => {
                    eprintln!("could not persist Waku daemon settings: {error:#}");
                }
            }
            drop(inner);
            std::thread::sleep(REBUILD_POLL_INTERVAL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_origins_are_exact_and_deduplicated() {
        assert_eq!(
            parse_allowed_origins(
                "https://app.waku.test, http://localhost:3001, https://app.waku.test"
            )
            .unwrap(),
            ["https://app.waku.test", "http://localhost:3001"]
        );
        assert!(parse_allowed_origins("https://app.waku.test/path").is_err());
        assert!(parse_allowed_origins("ws://app.waku.test").is_err());
    }

    #[test]
    fn desktop_uses_loopback_to_reach_an_unspecified_listener() {
        assert_eq!(
            desktop_client_address("0.0.0.0:34123").unwrap(),
            "127.0.0.1:34123"
        );
        assert_eq!(desktop_client_address("[::]:34123").unwrap(), "[::1]:34123");
    }

    #[test]
    fn a_live_process_with_a_dead_socket_is_redialed_before_it_is_replaced() {
        assert_eq!(
            plan_local_recovery(false, false, false, 0),
            LocalRecovery::None
        );
        assert_eq!(
            plan_local_recovery(false, true, false, 0),
            LocalRecovery::Reconnect
        );
        assert_eq!(
            plan_local_recovery(false, true, false, LOCAL_RECONNECT_ATTEMPTS - 1),
            LocalRecovery::Reconnect
        );
        assert_eq!(
            plan_local_recovery(false, true, false, LOCAL_RECONNECT_ATTEMPTS),
            LocalRecovery::Restart
        );
        assert_eq!(
            plan_local_recovery(true, true, false, 0),
            LocalRecovery::Restart
        );
        assert_eq!(
            plan_local_recovery(false, false, true, 0),
            LocalRecovery::Restart
        );
    }

    #[test]
    fn a_socket_that_keeps_dropping_after_redials_backs_off() {
        let start = Instant::now();
        let first = DaemonClient::disconnected_for_test(Vec::new());
        let mut redials = RedialState::default();
        redials.observe(&first, true, start);
        assert!(
            !redials.backing_off(start),
            "a first drop is redialed at once"
        );
        redials.redialed(start);

        // The replacement drops right away: wait one second, then two.
        let second = DaemonClient::disconnected_for_test(Vec::new());
        let dropped_again = start + Duration::from_secs(1);
        redials.observe(&second, true, dropped_again);
        assert!(redials.backing_off(dropped_again + Duration::from_millis(900)));
        assert!(!redials.backing_off(dropped_again + Duration::from_millis(1100)));
        redials.redialed(dropped_again + Duration::from_secs(1));
        let third = DaemonClient::disconnected_for_test(Vec::new());
        let dropped_thrice = dropped_again + Duration::from_secs(2);
        redials.observe(&third, true, dropped_thrice);
        assert!(redials.backing_off(dropped_thrice + Duration::from_millis(1900)));
        assert!(!redials.backing_off(dropped_thrice + Duration::from_millis(2100)));

        // A socket that held for longer than the window starts over.
        redials.redialed(dropped_thrice + Duration::from_secs(2));
        let fourth = DaemonClient::disconnected_for_test(Vec::new());
        let much_later = dropped_thrice + Duration::from_secs(60);
        redials.observe(&fourth, true, much_later);
        assert!(!redials.backing_off(much_later));

        // Refusals count per connection and a restart clears everything.
        redials.dial_refused();
        redials.dial_refused();
        assert_eq!(redials.refused, 2);
        redials.observe(&fourth, true, much_later + Duration::from_secs(1));
        assert_eq!(redials.refused, 2, "the same connection keeps its count");
        redials.restarted();
        assert_eq!(redials.refused, 0);
        assert!(!redials.backing_off(much_later));

        assert_eq!(redial_backoff(1), Duration::from_secs(1));
        assert_eq!(redial_backoff(2), Duration::from_secs(2));
        assert_eq!(redial_backoff(3), Duration::from_secs(4));
        assert_eq!(redial_backoff(4), Duration::from_secs(8));
        assert_eq!(redial_backoff(9), Duration::from_secs(8));
    }

    /// A child that stays alive until the supervisor stops it, standing in
    /// for a daemon process whose socket is what the test manipulates.
    fn idle_child() -> Child {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;

            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            ProcessCommand::new("ping")
                .args(["-n", "61", "127.0.0.1"])
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("ping is available on Windows")
        }
        #[cfg(not(windows))]
        {
            ProcessCommand::new("sleep")
                .arg("60")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("sleep is available")
        }
    }

    fn local_supervisor(process: DaemonProcess) -> DaemonSupervisor {
        DaemonSupervisor::from_target(
            DaemonTarget::Local(process),
            Some(PathBuf::from("waku-daemon-that-does-not-exist")),
            Some(DaemonExposureSettings::default()),
            DaemonSettings::default(),
        )
        .unwrap()
    }

    #[test]
    fn reopening_a_local_socket_redials_the_same_process_and_publishes_the_client() {
        let cursor = ReplayCursor {
            session_id: Uuid::from_u128(1),
            runtime_id: Uuid::from_u128(2),
            epoch: Uuid::from_u128(3),
            sequence: 5,
        };
        let dropped = DaemonClient::disconnected_for_test(vec![cursor]);
        let replacement = DaemonClient::disconnected_for_test(Vec::new());
        let supervisor = local_supervisor(DaemonProcess::for_test(
            dropped.clone(),
            idle_child(),
            "127.0.0.1:4312".into(),
            "token-a".into(),
        ));
        let clients = supervisor.subscribe_clients();
        assert!(
            clients
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .same_connection(&dropped)
        );

        let dials = Arc::new(Mutex::new(Vec::new()));
        let recorded = dials.clone();
        let mut connect = |address: &str, token: String, resume_from: Vec<ReplayCursor>| {
            recorded
                .lock()
                .push((address.to_owned(), token, resume_from));
            Ok(replacement.clone())
        };
        assert!(reopen_socket(&supervisor.inner, &dropped, &mut connect).unwrap());
        {
            let dials = dials.lock();
            assert_eq!(dials.len(), 1);
            assert_eq!(dials[0].0, "127.0.0.1:4312");
            assert_eq!(dials[0].1, "token-a");
            assert_eq!(dials[0].2.len(), 1);
            assert_eq!(dials[0].2[0].session_id, Uuid::from_u128(1));
            assert_eq!(dials[0].2[0].sequence, 5);
        }
        assert!(supervisor.client().same_connection(&replacement));
        assert!(matches!(
            &*supervisor.inner.target.lock(),
            DaemonTarget::Local(_)
        ));
        assert!(
            clients
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .same_connection(&replacement)
        );

        // A stale handle is not redialed: the target already moved on.
        assert!(!reopen_socket(&supervisor.inner, &dropped, &mut connect).unwrap());
        assert_eq!(dials.lock().len(), 1);
    }

    #[test]
    fn a_restarting_target_is_never_redialed() {
        let dropped = DaemonClient::disconnected_for_test(Vec::new());
        let supervisor = DaemonSupervisor::from_target(
            DaemonTarget::Restarting(dropped.clone()),
            Some(PathBuf::from("waku-daemon-that-does-not-exist")),
            Some(DaemonExposureSettings::default()),
            DaemonSettings::default(),
        )
        .unwrap();
        let mut connect =
            |_: &str, _: String, _: Vec<ReplayCursor>| -> anyhow::Result<DaemonClient> {
                panic!("a restarting daemon has no endpoint to dial")
            };
        assert!(!reopen_socket(&supervisor.inner, &dropped, &mut connect).unwrap());
    }

    /// The smallest daemon a `DaemonClient` will talk to: it answers the
    /// handshake, settings reads, and session attachment, and closes every
    /// socket when told to stop.
    struct FakeDaemon {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl FakeDaemon {
        fn serve(listener: std::net::TcpListener, token: &'static str, runtime_id: Uuid) -> Self {
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread = std::thread::spawn(move || {
                let mut connections = Vec::new();
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let stop = thread_stop.clone();
                            connections.push(std::thread::spawn(move || {
                                fake_connection(stream, token, runtime_id, stop)
                            }));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
                for connection in connections {
                    let _ = connection.join();
                }
            });
            Self {
                stop,
                thread: Some(thread),
            }
        }

        fn stop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    impl Drop for FakeDaemon {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn fake_connection(
        stream: std::net::TcpStream,
        token: &str,
        runtime_id: Uuid,
        stop: Arc<AtomicBool>,
    ) {
        use tungstenite::Message;
        use waku_protocol::{ClientMessage, ResponseOutcome, ServerMessage};

        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(25)))
            .unwrap();
        loop {
            if stop.load(Ordering::Acquire) {
                let _ = socket.close(None);
                let _ = socket.flush();
                return;
            }
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let message: ClientMessage = serde_json::from_str(text.as_ref()).unwrap();
                    let reply = match message {
                        ClientMessage::Hello { token: offered, .. } => {
                            assert_eq!(offered, token);
                            ServerMessage::Hello {
                                protocol_version: PROTOCOL_VERSION,
                                daemon_version: "test".into(),
                            }
                        }
                        ClientMessage::Request(request) => {
                            if request.request_id.is_nil() {
                                continue;
                            }
                            let payload = match request.command {
                                Command::GetSettings => ResponsePayload::Settings {
                                    settings: DaemonSettings::default(),
                                },
                                Command::AttachSession => ResponsePayload::SessionRuntime {
                                    runtime_id: Some(runtime_id),
                                    supports_steer: true,
                                },
                                _ => ResponsePayload::Ack,
                            };
                            ServerMessage::Response {
                                request_id: request.request_id,
                                outcome: ResponseOutcome::Ok { payload },
                            }
                        }
                        ClientMessage::Shutdown => {
                            let _ = socket.close(None);
                            return;
                        }
                    };
                    socket
                        .send(Message::Text(serde_json::to_string(&reply).unwrap().into()))
                        .unwrap();
                }
                Ok(Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return,
            }
        }
    }

    #[test]
    fn local_supervisor_redials_a_live_process() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        // Keep the port reserved between the two fake daemons, as the remote
        // reconnect test in waku-core does.
        let spare = listener.try_clone().unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let runtime_id = Uuid::new_v4();
        let mut first = FakeDaemon::serve(listener, "secret", runtime_id);

        let client = DaemonClient::connect(&address, "secret".into()).unwrap();
        let supervisor = local_supervisor(DaemonProcess::for_test(
            client,
            idle_child(),
            address.clone(),
            "secret".into(),
        ));
        let weak_inner = Arc::downgrade(&supervisor.inner);
        std::thread::spawn(move || monitor_daemon(weak_inner, None, false));
        let clients = supervisor.subscribe_clients();
        let initial = clients.recv_timeout(Duration::from_secs(1)).unwrap();

        first.stop();
        let _second = FakeDaemon::serve(spare, "secret", runtime_id);

        let replacement = clients.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(!initial.same_connection(&replacement));
        assert!(initial.is_disconnected());
        assert!(matches!(
            replacement
                .request(Uuid::new_v4(), Uuid::nil(), Command::AttachSession)
                .unwrap(),
            ResponsePayload::SessionRuntime {
                runtime_id: Some(attached),
                supports_steer: true,
            } if attached == runtime_id
        ));
        match &mut *supervisor.inner.target.lock() {
            DaemonTarget::Local(process) => assert!(!process.has_exited()),
            _ => panic!("a redial must keep the process it dialed"),
        }
        assert!(supervisor.client().same_connection(&replacement));
    }

    #[test]
    fn local_supervisor_replaces_the_process_after_refused_redials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut daemon = FakeDaemon::serve(listener, "secret", Uuid::new_v4());

        let client = DaemonClient::connect(&address, "secret".into()).unwrap();
        let supervisor = local_supervisor(DaemonProcess::for_test(
            client,
            idle_child(),
            address,
            "secret".into(),
        ));
        let weak_inner = Arc::downgrade(&supervisor.inner);
        std::thread::spawn(move || monitor_daemon(weak_inner, None, false));

        // Every redial is refused once the listener is gone, so after the
        // attempt budget the supervisor falls back to replacing the process,
        // which fails here because the executable does not exist and leaves
        // the target in `Restarting`. That state is the proof the fallback ran.
        daemon.stop();
        drop(daemon);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if matches!(
                &*supervisor.inner.target.lock(),
                DaemonTarget::Restarting(_)
            ) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the supervisor kept redialing a dead listener instead of restarting"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
