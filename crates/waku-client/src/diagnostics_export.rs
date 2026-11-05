//! Support bundles for daemon reconnect investigations.
//!
//! Only Waku's metadata-only reconnect JSONL and daemon records explicitly
//! targeted at `daemon_connection` are included. General daemon logs can quote
//! agent activity and must not be copied wholesale into a support bundle.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const MAX_LOG_TAIL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonDiagnosticsMetadata {
    pub exported_at: String,
    pub app_version: String,
    pub app_pid: u32,
    pub daemon_version: String,
    pub daemon_pid: Option<u32>,
    pub connection_id: String,
    pub remote_daemon: bool,
}

pub fn export_daemon_diagnostics(
    destination: &Path,
    metadata: &DaemonDiagnosticsMetadata,
) -> anyhow::Result<usize> {
    write_export(
        destination,
        crate::diagnostics::recent_log_files(),
        metadata,
    )
}

fn write_export(
    destination: &Path,
    mut sources: Vec<PathBuf>,
    metadata: &DaemonDiagnosticsMetadata,
) -> anyhow::Result<usize> {
    // Rotated records predate the current file.
    sources.sort_by_key(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        (!name.ends_with(".1"), name.to_owned())
    });
    sources.retain(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        !metadata.remote_daemon || !name.starts_with("daemon.log")
    });

    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options
        .open(destination)
        .with_context(|| format!("could not create {}", destination.display()))?;
    let mut archive = zip::ZipWriter::new(file);
    let archive_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o600);

    let mut manifest = serde_json::to_value(metadata)?;
    manifest["purpose"] = "Daemon reconnect diagnostics / 用于排查守护进程重连".into();
    manifest["privacy"] =
        "Connection metadata only; no auth tokens, API keys, or message bodies. / 仅含连接元数据，不含认证令牌、API 密钥或消息正文。".into();
    manifest["client_log"] = serde_json::to_value(crate::diagnostics::client_diagnostics_status())?;
    archive.start_file("manifest.json", archive_options)?;
    archive.write_all(&serde_json::to_vec_pretty(&manifest)?)?;

    let mut included = 0;
    for source in sources {
        if !source.is_file() {
            continue;
        }
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .context("diagnostics log has a non-UTF-8 name")?;
        let daemon_log = name.starts_with("daemon.log");
        let bytes = read_log_tail(&source, MAX_LOG_TAIL_BYTES)?;
        let bytes = if daemon_log {
            filter_daemon_connection_records(&bytes)
        } else {
            filter_client_connection_records(&bytes)
        };
        if bytes.is_empty() {
            continue;
        }
        let archive_name = if daemon_log {
            name.replace("daemon.log", "daemon-connection.log")
        } else {
            name.to_owned()
        };
        archive.start_file(archive_name, archive_options)?;
        archive.write_all(&bytes)?;
        included += 1;
    }
    archive.finish()?;
    Ok(included)
}

fn read_log_tail(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    let mut file =
        File::open(path).with_context(|| format!("could not read {}", path.display()))?;
    let len = file.metadata()?.len();
    let truncated = len > limit;
    if truncated {
        file.seek(SeekFrom::Start(len - limit))?;
    }
    let mut bytes = Vec::with_capacity(len.min(limit) as usize);
    file.read_to_end(&mut bytes)?;
    if truncated {
        // Do not begin a text export in the middle of a UTF-8 character or a
        // JSON/tracing record. If no newline remains, the bounded tail is one
        // oversized partial record and is not useful.
        let start = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + 1)
            .unwrap_or(bytes.len());
        bytes.drain(..start);
    }
    Ok(bytes)
}

fn filter_daemon_connection_records(bytes: &[u8]) -> Vec<u8> {
    let mut filtered = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        if let Some(record) = daemon_connection_record(line) {
            push_json_record(&mut filtered, record);
        }
    }
    filtered
}

/// Parse the non-JSON tracing formatter structurally. In particular, matching
/// the target in an arbitrary error message is not enough: the header must be
/// exactly `<rfc3339 timestamp> <level> daemon_connection:`.
fn daemon_connection_record(line: &str) -> Option<Map<String, Value>> {
    let (header, record) = line.split_once("daemon_connection:")?;
    let mut header = header.split_whitespace();
    let timestamp = header.next()?;
    let level = header.next()?;
    if header.next().is_some()
        || chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
        || !matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR")
    {
        return None;
    }
    let record = record
        .strip_prefix(' ')?
        .strip_prefix("daemon connection terminated")?;
    if !record.is_empty() && !record.starts_with(char::is_whitespace) {
        return None;
    }

    let mut output = Map::new();
    output.insert("timestamp".into(), timestamp.into());
    output.insert("level".into(), level.into());
    output.insert("target".into(), "daemon_connection".into());
    output.insert("event".into(), "connection_drop".into());
    for token in record.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "connection_id" if valid_connection_id(value) => {
                output.insert(key.into(), value.into());
            }
            "phase" if matches!(value, "handshake" | "read" | "write") => {
                output.insert(key.into(), value.into());
            }
            "tungstenite_variant" if valid_websocket_variant(value) => {
                output.insert(key.into(), value.into());
            }
            "io_kind" if valid_io_kind(value) => {
                output.insert(key.into(), value.into());
            }
            "raw_os_error" => {
                insert_optional_number(&mut output, key, value, |value| {
                    value.parse::<i32>().ok().map(Value::from)
                });
            }
            "message_size" | "max_message_size" => {
                insert_optional_number(&mut output, key, value, |value| {
                    value.parse::<u64>().ok().map(Value::from)
                });
            }
            "daemon_pid" => {
                insert_optional_number(&mut output, key, value, |value| {
                    value.parse::<u32>().ok().map(Value::from)
                });
            }
            "daemon_version" if valid_version(value) => {
                output.insert(key.into(), value.into());
            }
            _ => {}
        }
    }
    Some(output)
}

fn insert_optional_number(
    output: &mut Map<String, Value>,
    key: &str,
    value: &str,
    parse: impl FnOnce(&str) -> Option<Value>,
) {
    if value == "None" {
        output.insert(key.into(), Value::Null);
    } else if let Some(value) = parse(value) {
        output.insert(key.into(), value);
    }
}

fn filter_client_connection_records(bytes: &[u8]) -> Vec<u8> {
    let mut filtered = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(event) = record.get("event").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(
            event,
            "dial_started"
                | "dial_failed"
                | "dial_succeeded"
                | "connection_drop"
                | "connection_publish"
                | "redial_succeeded"
                | "redial_failed"
                | "restart_started"
                | "restart_succeeded"
                | "restart_failed"
        ) {
            continue;
        }
        let Some(timestamp) = record.get("timestamp").and_then(Value::as_str) else {
            continue;
        };
        if chrono::DateTime::parse_from_rfc3339(timestamp).is_err() {
            continue;
        }
        let Some(connection_id) = record.get("connection_id").and_then(Value::as_str) else {
            continue;
        };
        if !valid_connection_id(connection_id)
            || !record
                .get("app_pid")
                .and_then(Value::as_u64)
                .is_some_and(|pid| u32::try_from(pid).is_ok())
        {
            continue;
        }

        let mut output = Map::new();
        output.insert("event".into(), event.into());
        output.insert("timestamp".into(), timestamp.into());
        output.insert("connection_id".into(), connection_id.into());
        output.insert(
            "app_pid".into(),
            Value::from(record["app_pid"].as_u64().unwrap_or_default() as u32),
        );
        for (key, value) in record {
            if let Some(value) = sanitize_client_field(&key, &value) {
                output.insert(key, value);
            }
        }
        push_json_record(&mut filtered, output);
    }
    filtered
}

fn sanitize_client_field(key: &str, value: &Value) -> Option<Value> {
    match key {
        "resume_cursor_count"
        | "attempt"
        | "refused_redials"
        | "message_size"
        | "max_message_size" => value.as_u64().map(Value::from),
        "raw_os_error" => value
            .as_i64()
            .and_then(|code| i32::try_from(code).ok())
            .map(Value::from),
        "daemon_pid" if value.is_null() => Some(Value::Null),
        "daemon_pid" => value
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .map(Value::from),
        "process_exited" | "executable_changed" => value.as_bool().map(Value::from),
        "previous_connection_id" => value
            .as_str()
            .filter(|value| valid_connection_id(value))
            .map(Value::from),
        "daemon_version" => value
            .as_str()
            .filter(|value| valid_version(value))
            .map(Value::from),
        "reason" => value
            .as_str()
            .filter(|value| matches!(*value, "read" | "write" | "close" | "shutting_down"))
            .map(Value::from),
        "restart_reason" => value
            .as_str()
            .filter(|value| {
                matches!(
                    *value,
                    "process_exited" | "executable_changed" | "redial_attempts_exhausted"
                )
            })
            .map(Value::from),
        "tungstenite_variant" => value
            .as_str()
            .filter(|value| valid_websocket_variant(value))
            .map(Value::from),
        "io_kind" => value
            .as_str()
            .filter(|value| valid_io_kind(value))
            .map(Value::from),
        _ => None,
    }
}

fn valid_connection_id(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

fn valid_io_kind(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn valid_websocket_variant(value: &str) -> bool {
    matches!(
        value,
        "connection_closed"
            | "already_closed"
            | "io"
            | "tls"
            | "capacity.message_too_long"
            | "capacity"
            | "protocol"
            | "write_buffer_full"
            | "utf8"
            | "attack_attempt"
            | "url"
            | "http"
            | "http_format"
            | "other"
    )
}

fn push_json_record(output: &mut Vec<u8>, record: Map<String, Value>) {
    if serde_json::to_writer(&mut *output, &Value::Object(record)).is_ok() {
        output.push(b'\n');
    }
}

pub fn validate_destination(path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        bail!("an export path is required")
    }
    if path.is_dir() {
        bail!("the export destination is a directory")
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        bail!("the export directory does not exist")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn metadata(remote_daemon: bool) -> DaemonDiagnosticsMetadata {
        DaemonDiagnosticsMetadata {
            exported_at: "2026-11-05T00:00:00Z".into(),
            app_version: "0.3.2".into(),
            app_pid: 42,
            daemon_version: "0.1.0".into(),
            daemon_pid: (!remote_daemon).then_some(43),
            connection_id: "a1b2c3d4".into(),
            remote_daemon,
        }
    }

    #[test]
    fn daemon_export_excludes_unrelated_sensitive_records() {
        let input = b"2026-11-05T00:00:00Z WARN agent: prompt body and api-key\n\
                      2026-11-05T00:00:00Z ERROR agent: daemon_connection: daemon connection terminated prompt=secret-token\n\
                      2026-11-05T00:00:00Z WARN daemon_connection: daemon connection terminated connection_id=a1b2c3d4 raw_os_error=10054 message=secret-body\n";
        let output = filter_daemon_connection_records(input);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("daemon_connection"));
        assert!(output.contains("10054"));
        assert!(!output.contains("prompt body"));
        assert!(!output.contains("api-key"));
        assert!(!output.contains("secret-token"));
        assert!(!output.contains("secret-body"));
    }

    #[test]
    fn client_export_discards_unknown_fields_and_malformed_records() {
        let input = br#"{"event":"connection_drop","timestamp":"2026-11-05T00:00:00Z","connection_id":"a1b2c3d4","app_pid":42,"raw_os_error":10054,"message":"secret body","api_key":"secret-token"}
{"event":"connection_drop","timestamp":"2026-11-05T00:00:00Z","connection_id":"a1b2c3d4","app_pid":42,"error":"daemon_connection daemon connection terminated secret"}
not json daemon_connection daemon connection terminated secret
"#;
        let output = filter_client_connection_records(input);
        let output = String::from_utf8(output).unwrap();
        let records: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["raw_os_error"], 10054);
        for record in &records {
            let fields = record.as_object().unwrap();
            assert!(!fields.contains_key("message"));
            assert!(!fields.contains_key("api_key"));
            assert!(!fields.contains_key("error"));
        }
        assert!(!output.contains("secret"));
    }

    #[test]
    fn export_contains_metadata_and_only_filtered_logs() {
        let root = std::env::temp_dir().join(format!("waku-export-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let reconnect = root.join("reconnect.log");
        let daemon = root.join("daemon.log");
        fs::write(
            &reconnect,
            b"{\"event\":\"connection_drop\",\"timestamp\":\"2026-11-05T00:00:00Z\",\"connection_id\":\"a1b2c3d4\",\"app_pid\":42,\"message\":\"secret body\"}\n",
        )
        .unwrap();
        fs::write(
            &daemon,
            b"2026-11-05T00:00:00Z WARN agent: secret message\n2026-11-05T00:00:00Z WARN daemon_connection: daemon connection terminated raw_os_error=10054\n",
        )
        .unwrap();
        let destination = root.join("export.zip");
        let included =
            write_export(&destination, vec![daemon, reconnect], &metadata(false)).unwrap();
        assert_eq!(included, 2);

        let mut archive = zip::ZipArchive::new(File::open(&destination).unwrap()).unwrap();
        assert!(archive.by_name("manifest.json").is_ok());
        assert!(archive.by_name("reconnect.log").is_ok());
        assert!(archive.by_name("daemon.log").is_err());
        let mut daemon_export = archive.by_name("daemon-connection.log").unwrap();
        let mut text = String::new();
        daemon_export.read_to_string(&mut text).unwrap();
        assert!(text.contains("10054"));
        assert!(!text.contains("secret message"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_export_omits_local_daemon_logs() {
        let root = std::env::temp_dir().join(format!("waku-export-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let daemon = root.join("daemon.log");
        fs::write(
            &daemon,
            b"2026-11-05T00:00:00Z WARN daemon_connection: daemon connection terminated\n",
        )
        .unwrap();
        let destination = root.join("export.zip");
        let included = write_export(&destination, vec![daemon], &metadata(true)).unwrap();
        assert_eq!(included, 0);
        let mut archive = zip::ZipArchive::new(File::open(&destination).unwrap()).unwrap();
        assert!(archive.by_name("daemon-connection.log").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
