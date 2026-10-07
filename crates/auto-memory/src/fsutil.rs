//! Atomic file replacement (the AgentTeams store's policy, copied so this
//! crate stands alone) and the symlink guard.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

const RENAME_RETRIES: u32 = 5;
const RENAME_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Whether `path` is a symbolic link. A memory directory never follows one:
/// a link could pull text from anywhere on disk into the system prompt.
pub fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// A rename error worth retrying: on Windows another process holding the
/// target open (an editor, an indexer, antivirus) fails the rename with
/// access denied or a sharing violation, which clears quickly.
fn is_retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::AlreadyExists | io::ErrorKind::ResourceBusy
    ) || (cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33)))
}

/// Replace `file` with `content` through a same-directory temporary file;
/// when the rename keeps failing, write in place (equivalent content, merely
/// not atomic). The temporary file never survives.
pub fn atomic_write_text(file: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = PathBuf::from(format!(
        "{}.{}.{}.tmp",
        file.display(),
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let written = (|| {
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        handle.write_all(content.as_bytes())?;
        handle.sync_all()
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    let mut attempt = 0;
    loop {
        match std::fs::rename(&temporary, file) {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable(&error) && attempt < RENAME_RETRIES => {
                attempt += 1;
                std::thread::sleep(RENAME_RETRY_DELAY);
            }
            Err(error) => {
                let fallback = std::fs::write(file, content);
                let _ = std::fs::remove_file(&temporary);
                return fallback.map_err(|write_error| {
                    io::Error::new(
                        write_error.kind(),
                        format!(
                            "failed to replace \"{}\" atomically ({error}) or by direct write ({write_error})",
                            file.display()
                        ),
                    )
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_replaces_without_leaving_temporaries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("a.md");
        atomic_write_text(&file, "one").unwrap();
        atomic_write_text(&file, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
        let leftovers = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
