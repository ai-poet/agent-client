//! Metadata-only diagnostics for daemon transport failures.
//!
//! These records deliberately contain no daemon address, bearer token, RPC
//! payload, or message body. They are safe to export when investigating a
//! reconnect without including a conversation in the support bundle.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value, json};
use tungstenite::Error as WebSocketError;
use tungstenite::error::CapacityError;

use crate::persistence::StateStore;

const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;
const CLIENT_LOG_FILE: &str = "reconnect.log";
const DAEMON_LOG_FILE: &str = "daemon.log";
static CLIENT_LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();

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
        "reconnect.log.1",
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

    let log = CLIENT_LOG.get_or_init(|| Mutex::new(open_client_log()));
    let Ok(mut log) = log.lock() else {
        return;
    };
    if let Some(file) = log.as_mut() {
        let _ = file.write_all(&line);
    }
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

fn open_client_log() -> Option<File> {
    let directory = log_directory();
    fs::create_dir_all(&directory).ok()?;
    let path = directory.join(CLIENT_LOG_FILE);
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > ROTATE_AT_BYTES) {
        let _ = fs::rename(&path, directory.join("reconnect.log.1"));
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).ok()
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
}
