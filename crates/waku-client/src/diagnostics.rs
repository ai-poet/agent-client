//! Metadata-only diagnostics for daemon transport failures.
//!
//! These records deliberately contain no daemon address, bearer token, RPC
//! payload, or message body. They are safe to export when investigating a
//! reconnect without including a conversation in the support bundle.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value, json};
use tungstenite::Error as WebSocketError;
use tungstenite::error::CapacityError;

use crate::persistence::StateStore;

const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;
const OPEN_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_LOG_FILE: &str = "reconnect.log";
const CLIENT_ROTATED_LOG_FILE: &str = "reconnect.log.1";
const DAEMON_LOG_FILE: &str = "daemon.log";
static CLIENT_LOG: OnceLock<Mutex<ClientLog>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientLogAvailability {
    Uninitialized,
    Writable,
    Unavailable,
}

/// Observable health of the client reconnect log. No paths or OS error
/// messages are exposed because those can contain user or directory names.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClientDiagnosticsStatus {
    pub availability: ClientLogAvailability,
    pub dropped_records: u64,
    pub rotations: u64,
    pub last_error_kind: Option<String>,
    pub last_raw_os_error: Option<i32>,
}

struct ClientLog {
    directory: PathBuf,
    rotate_at_bytes: u64,
    retry_interval: Duration,
    file: Option<File>,
    bytes_written: u64,
    next_open_attempt: Option<Instant>,
    status: ClientDiagnosticsStatus,
}

impl ClientLog {
    fn new(directory: PathBuf, rotate_at_bytes: u64, retry_interval: Duration) -> Self {
        Self {
            directory,
            rotate_at_bytes,
            retry_interval,
            file: None,
            bytes_written: 0,
            next_open_attempt: None,
            status: ClientDiagnosticsStatus {
                availability: ClientLogAvailability::Uninitialized,
                dropped_records: 0,
                rotations: 0,
                last_error_kind: None,
                last_raw_os_error: None,
            },
        }
    }

    fn write_line(&mut self, line: &[u8]) {
        self.write_line_at(line, Instant::now());
    }

    fn write_line_at(&mut self, line: &[u8], now: Instant) {
        let line_len = line.len() as u64;
        if line_len > self.rotate_at_bytes {
            self.status.dropped_records = self.status.dropped_records.saturating_add(1);
            return;
        }
        if self.file.is_none()
            && self
                .next_open_attempt
                .is_some_and(|next_attempt| now < next_attempt)
        {
            self.status.dropped_records = self.status.dropped_records.saturating_add(1);
            return;
        }
        if self.file.is_none()
            && let Err(error) = self.open()
        {
            self.record_failure(&error, now);
            return;
        }

        if self.bytes_written > 0
            && self.bytes_written.saturating_add(line_len) > self.rotate_at_bytes
        {
            // Do not keep appending if rotation fails: the size limit is more
            // important than retaining one more diagnostic record.
            self.file = None;
            self.bytes_written = 0;
            if let Err(error) = self.rotate() {
                self.record_failure(&error, now);
                return;
            }
            self.status.rotations = self.status.rotations.saturating_add(1);
            if let Err(error) = self.open() {
                self.record_failure(&error, now);
                return;
            }
        }

        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(error) = file.write_all(line) {
            self.record_failure(&error, now);
            return;
        }
        self.bytes_written = self.bytes_written.saturating_add(line_len);
    }

    fn open(&mut self) -> io::Result<()> {
        fs::create_dir_all(&self.directory)?;
        let path = self.directory.join(CLIENT_LOG_FILE);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > 0 && metadata.len() >= self.rotate_at_bytes => {
                self.rotate()?;
                self.status.rotations = self.status.rotations.saturating_add(1);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        self.bytes_written = file.metadata()?.len();
        self.file = Some(file);
        self.next_open_attempt = None;
        self.status.availability = ClientLogAvailability::Writable;
        self.status.last_error_kind = None;
        self.status.last_raw_os_error = None;
        Ok(())
    }

    fn rotate(&self) -> io::Result<()> {
        let path = self.directory.join(CLIENT_LOG_FILE);
        let rotated = self.directory.join(CLIENT_ROTATED_LOG_FILE);
        match fs::remove_file(&rotated) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(path, rotated)
    }

    fn record_failure(&mut self, error: &io::Error, now: Instant) {
        self.file = None;
        self.bytes_written = 0;
        self.next_open_attempt = Some(now + self.retry_interval);
        self.status.availability = ClientLogAvailability::Unavailable;
        self.status.dropped_records = self.status.dropped_records.saturating_add(1);
        self.status.last_error_kind = Some(format!("{:?}", error.kind()));
        self.status.last_raw_os_error = error.raw_os_error();
    }
}

fn client_log() -> &'static Mutex<ClientLog> {
    CLIENT_LOG.get_or_init(|| {
        Mutex::new(ClientLog::new(
            log_directory(),
            ROTATE_AT_BYTES,
            OPEN_RETRY_INTERVAL,
        ))
    })
}

/// Snapshot included in support bundles so missing client records are visible.
pub fn client_diagnostics_status() -> ClientDiagnosticsStatus {
    client_log()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .status
        .clone()
}

/// The directory shared by the desktop and daemon logs.
pub fn log_directory() -> PathBuf {
    StateStore::default_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("logs")
}

/// Current and previous transport logs eligible for a diagnostics export.
/// The exporter filters daemon records to the `daemon_connection` target.
pub fn recent_log_files() -> Vec<PathBuf> {
    let directory = log_directory();
    [
        CLIENT_LOG_FILE,
        CLIENT_ROTATED_LOG_FILE,
        DAEMON_LOG_FILE,
        "daemon.log.1",
    ]
    .into_iter()
    .map(|name| directory.join(name))
    .collect()
}

pub(crate) fn connection_event(event: &str, connection_id: &str, mut fields: Map<String, Value>) {
    fields.insert("event".into(), event.into());
    fields.insert("connection_id".into(), connection_id.into());
    fields.insert("app_pid".into(), std::process::id().into());
    fields.insert(
        "timestamp".into(),
        chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .into(),
    );
    let Ok(mut line) = serde_json::to_vec(&Value::Object(fields)) else {
        return;
    };
    line.push(b'\n');

    let mut log = client_log()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    log.write_line(&line);
}

pub(crate) fn error_fields(error: &(dyn std::error::Error + 'static)) -> Map<String, Value> {
    let mut fields = Map::new();
    collect_error_fields(error, &mut fields);
    fields
}

fn collect_error_fields(
    error: &(dyn std::error::Error + 'static),
    fields: &mut Map<String, Value>,
) {
    if let Some(error) = error.downcast_ref::<WebSocketError>() {
        fields.insert(
            "tungstenite_variant".into(),
            websocket_variant(error).into(),
        );
        match error {
            WebSocketError::Io(error) => insert_io_fields(error, fields),
            WebSocketError::Capacity(CapacityError::MessageTooLong { size, max_size }) => {
                fields.insert("message_size".into(), (*size as u64).into());
                fields.insert("max_message_size".into(), (*max_size as u64).into());
            }
            _ => {}
        }
    }
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        insert_io_fields(error, fields);
    }
    if let Some(source) = error.source() {
        collect_error_fields(source, fields);
    }
}

fn insert_io_fields(error: &std::io::Error, fields: &mut Map<String, Value>) {
    fields.insert("io_kind".into(), format!("{:?}", error.kind()).into());
    if let Some(os_error) = error.raw_os_error() {
        fields.insert("raw_os_error".into(), os_error.into());
    }
}

fn websocket_variant(error: &WebSocketError) -> &'static str {
    match error {
        WebSocketError::ConnectionClosed => "connection_closed",
        WebSocketError::AlreadyClosed => "already_closed",
        WebSocketError::Io(_) => "io",
        WebSocketError::Tls(_) => "tls",
        WebSocketError::Capacity(CapacityError::MessageTooLong { .. }) => {
            "capacity.message_too_long"
        }
        WebSocketError::Capacity(_) => "capacity",
        WebSocketError::Protocol(_) => "protocol",
        WebSocketError::WriteBufferFull(_) => "write_buffer_full",
        WebSocketError::Utf8(_) => "utf8",
        WebSocketError::AttackAttempt => "attack_attempt",
        WebSocketError::Url(_) => "url",
        WebSocketError::Http(_) => "http",
        WebSocketError::HttpFormat(_) => "http_format",
    }
}

pub(crate) fn log_error_event(
    event: &str,
    connection_id: &str,
    error: &(dyn std::error::Error + 'static),
    mut fields: Map<String, Value>,
) {
    fields.extend(error_fields(error));
    connection_event(event, connection_id, fields);
}

pub(crate) fn fields(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

pub(crate) fn empty_fields() -> Map<String, Value> {
    fields(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_io_errors_keep_only_structured_metadata() {
        let error = WebSocketError::Io(std::io::Error::from_raw_os_error(10054));
        let fields = error_fields(&error);
        assert_eq!(fields["tungstenite_variant"], "io");
        assert_eq!(fields["raw_os_error"], 10054);
        assert!(fields.contains_key("io_kind"));
        assert!(!fields.contains_key("error"));
        assert!(!fields.contains_key("message"));
    }

    #[test]
    fn oversized_messages_include_sizes_but_not_payloads() {
        let error = WebSocketError::Capacity(CapacityError::MessageTooLong {
            size: 50,
            max_size: 48,
        });
        let fields = error_fields(&error);
        assert_eq!(fields["tungstenite_variant"], "capacity.message_too_long");
        assert_eq!(fields["message_size"], 50);
        assert_eq!(fields["max_message_size"], 48);
        assert!(!fields.contains_key("message"));
    }

    #[test]
    fn live_log_rotates_before_crossing_the_size_limit() {
        let root = std::env::temp_dir().join(format!("waku-log-test-{}", uuid::Uuid::new_v4()));
        let mut log = ClientLog::new(root.clone(), 48, Duration::from_secs(30));
        let now = Instant::now();
        log.write_line_at(b"0123456789abcdef\n", now);
        log.write_line_at(b"0123456789abcdef\n", now);
        log.write_line_at(b"fedcba9876543210\n", now);

        assert_eq!(log.status.rotations, 1);
        assert_eq!(log.status.dropped_records, 0);
        assert_eq!(
            fs::read(root.join(CLIENT_ROTATED_LOG_FILE)).unwrap(),
            b"0123456789abcdef\n0123456789abcdef\n"
        );
        assert_eq!(
            fs::read(root.join(CLIENT_LOG_FILE)).unwrap(),
            b"fedcba9876543210\n"
        );
        assert!(fs::metadata(root.join(CLIENT_LOG_FILE)).unwrap().len() <= 32);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_open_is_observable_throttled_and_recoverable() {
        let root = std::env::temp_dir().join(format!("waku-log-test-{}", uuid::Uuid::new_v4()));
        // A regular file where the log directory belongs makes the first open
        // fail without relying on platform-specific permission behavior.
        fs::write(&root, b"not a directory").unwrap();
        let mut log = ClientLog::new(root.clone(), 1024, Duration::from_secs(30));
        let now = Instant::now();
        log.write_line_at(b"first\n", now);
        assert_eq!(log.status.availability, ClientLogAvailability::Unavailable);
        assert!(log.status.last_error_kind.is_some());
        assert_eq!(log.status.dropped_records, 1);

        fs::remove_file(&root).unwrap();
        log.write_line_at(b"too soon\n", now + Duration::from_secs(1));
        assert!(!root.exists());
        assert_eq!(log.status.dropped_records, 2);

        log.write_line_at(b"recovered\n", now + Duration::from_secs(30));
        assert_eq!(log.status.availability, ClientLogAvailability::Writable);
        assert_eq!(log.status.last_error_kind, None);
        assert_eq!(
            fs::read(root.join(CLIENT_LOG_FILE)).unwrap(),
            b"recovered\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
