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
            bytes
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
    let text = String::from_utf8_lossy(bytes);
    let mut filtered = String::new();
    for line in text.lines() {
        if line.contains("daemon_connection") && line.contains("daemon connection terminated") {
            filtered.push_str(line);
            filtered.push('\n');
        }
    }
    filtered.into_bytes()
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
        let input = b"2026-11-05 WARN agent: prompt body and api-key\n\
                      2026-11-05 WARN daemon_connection: daemon connection terminated connection_id=a1b2c3d4 raw_os_error=10054\n";
        let output = filter_daemon_connection_records(input);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("daemon_connection"));
        assert!(output.contains("10054"));
        assert!(!output.contains("prompt body"));
        assert!(!output.contains("api-key"));
    }

    #[test]
    fn export_contains_metadata_and_only_filtered_logs() {
        let root = std::env::temp_dir().join(format!("waku-export-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let reconnect = root.join("reconnect.log");
        let daemon = root.join("daemon.log");
        fs::write(
            &reconnect,
            b"{\"event\":\"connection_drop\",\"connection_id\":\"a1b2c3d4\"}\n",
        )
        .unwrap();
        fs::write(
            &daemon,
            b"WARN agent: secret message\nWARN daemon_connection: daemon connection terminated raw_os_error=10054\n",
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
            b"WARN daemon_connection: daemon connection terminated\n",
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
