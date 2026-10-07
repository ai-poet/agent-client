//! The scope lock: every write, delete and index rebuild in one memory
//! directory runs inside it, across threads and across processes (the
//! daemon's agent and the desktop's settings page both write).
//!
//! In-process, a per-directory mutex queues callers. Across processes, a
//! `MEMORY.md.lock` file is created exclusively and holds
//! `<pid> <millis> <token>`. A lock older than [`STALE_AFTER`], or carrying
//! this process's pid (the mutex proves no thread here holds it), is an
//! orphan from a crash or Ctrl+C and is taken over — the reference's
//! orphan recovery, without a pid-liveness probe.

use std::collections::HashMap;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, bail};
use parking_lot::Mutex;

/// The lock file beside the index.
pub const LOCK_FILE: &str = "MEMORY.md.lock";
/// How long a contender waits.
const WAIT: Duration = Duration::from_secs(12);
const RETRY: Duration = Duration::from_millis(20);
/// A lock this old is an orphan: no operation holds it this long.
const STALE_AFTER: Duration = Duration::from_secs(10);

static LOCAL: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Run `operation` holding `dir`'s lock (the directory is created first).
pub fn with_scope_lock<T>(
    dir: &Path,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let local = LOCAL
        .lock()
        .entry(dir.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _local = local.lock();
    let _file = FileLock::acquire(&dir.join(LOCK_FILE))?;
    operation()
}

struct FileLock {
    path: PathBuf,
    token: String,
}

impl FileLock {
    fn acquire(path: &Path) -> anyhow::Result<Self> {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let deadline = Instant::now() + WAIT;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    let _ = write!(file, "{} {} {}", std::process::id(), crate::now_ms(), token);
                    let _ = file.sync_all();
                    return Ok(Self {
                        path: path.to_path_buf(),
                        token,
                    });
                }
                Err(error)
                    if error.kind() == ErrorKind::AlreadyExists
                        || (cfg!(windows) && error.kind() == ErrorKind::PermissionDenied) =>
                {
                    if is_orphan(path) {
                        let _ = std::fs::remove_file(path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        bail!(
                            "memory store is busy (another write holds {}); try again in a moment",
                            path.display()
                        );
                    }
                    std::thread::sleep(RETRY);
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("failed to lock {}", path.display()));
                }
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Only our own lock: an orphan taken over by someone else is theirs.
        if read_lock(&self.path).is_some_and(|content| content.ends_with(&self.token)) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn read_lock(path: &Path) -> Option<String> {
    let mut content = String::new();
    std::fs::File::open(path)
        .ok()?
        .read_to_string(&mut content)
        .ok()?;
    Some(content.trim().to_owned())
}

fn is_orphan(path: &Path) -> bool {
    if let Some(content) = read_lock(path) {
        let mut parts = content.split_whitespace();
        let pid = parts.next().and_then(|pid| pid.parse::<u32>().ok());
        let stamp = parts.next().and_then(|stamp| stamp.parse::<u64>().ok());
        if pid == Some(std::process::id()) {
            return true;
        }
        if let Some(stamp) = stamp {
            return crate::now_ms().saturating_sub(stamp) > STALE_AFTER.as_millis() as u64;
        }
    }
    // Unreadable (being written, or garbage): judge by its age on disk.
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_queue_on_the_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Arc::new(Mutex::new(Vec::new()));
        let handles = (0..8)
            .map(|index| {
                let dir = dir.path().to_path_buf();
                let counter = counter.clone();
                std::thread::spawn(move || {
                    with_scope_lock(&dir, || {
                        counter.lock().push(index);
                        Ok(())
                    })
                    .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(counter.lock().len(), 8);
        assert!(!dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn an_orphan_lock_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        // Our own pid with an old stamp: left behind by a crash.
        std::fs::write(
            dir.path().join(LOCK_FILE),
            format!("{} 1 dead", std::process::id()),
        )
        .unwrap();
        assert_eq!(with_scope_lock(dir.path(), || Ok(7)).unwrap(), 7);
        // Another process's lock from long ago.
        std::fs::write(dir.path().join(LOCK_FILE), "999999 1 old").unwrap();
        assert_eq!(with_scope_lock(dir.path(), || Ok(8)).unwrap(), 8);
    }

    #[test]
    fn a_live_foreign_lock_is_respected_until_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(LOCK_FILE),
            format!("999999 {} live", crate::now_ms()),
        )
        .unwrap();
        // Released after a moment by its "owner".
        let path = dir.path().join(LOCK_FILE);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            std::fs::remove_file(path).unwrap();
        });
        let started = Instant::now();
        with_scope_lock(dir.path(), || Ok(())).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150));
        releaser.join().unwrap();
    }
}
