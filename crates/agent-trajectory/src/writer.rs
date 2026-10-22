//! Where the records go: numbered JSON-lines segments in one directory.
//!
//! `<dir>/000001.jsonl`, `000002.jsonl`, … Each segment starts with a
//! [`Record::Segment`] line and rotates past [`SEGMENT_BYTES`]; past
//! [`TOTAL_BYTES`] the oldest segments are deleted and a
//! [`Record::Dropped`] line says so. Files are readable by their owner only:
//! tool payloads can hold anything the agent read.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::record::{Record, SCHEMA};

/// A segment rotates past this size.
pub const SEGMENT_BYTES: u64 = 1024 * 1024;
/// The directory is kept under this size.
pub const TOTAL_BYTES: u64 = 24 * 1024 * 1024;

/// The segment file name for number `n`.
pub fn segment_name(n: u32) -> String {
    format!("{n:06}.jsonl")
}

/// The segment numbers in `dir`, ascending.
pub fn segments(dir: &Path) -> Vec<u32> {
    let mut numbers: Vec<u32> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            name.strip_suffix(".jsonl")?.parse().ok()
        })
        .collect();
    numbers.sort_unstable();
    numbers
}

pub struct Writer {
    dir: PathBuf,
    session: String,
    current: u32,
    file: Option<File>,
    written: u64,
}

impl Writer {
    /// Append to the newest segment of `dir`, creating it as needed.
    pub fn open(dir: &Path, session: &str) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        restrict(dir, 0o700);
        let current = segments(dir).last().copied().unwrap_or(0);
        let mut writer = Self {
            dir: dir.to_path_buf(),
            session: session.to_owned(),
            current,
            file: None,
            written: 0,
        };
        if current == 0 {
            writer.rotate(0)?;
        } else {
            let path = dir.join(segment_name(current));
            writer.written = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            writer.file = Some(OpenOptions::new().append(true).open(&path)?);
        }
        Ok(writer)
    }

    /// The segment being written; a new one starts with nothing known, so
    /// the caller writes full prompts again after it changes.
    pub fn segment(&self) -> u32 {
        self.current
    }

    /// Write one record, rotating first when the segment is full.
    pub fn append(&mut self, record: &Record, at: i64) -> std::io::Result<()> {
        if self.written >= SEGMENT_BYTES {
            self.rotate(at)?;
        }
        let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
        line.push('\n');
        let file = self.file.as_mut().ok_or_else(|| std::io::Error::other("no segment"))?;
        file.write_all(line.as_bytes())?;
        self.written += line.len() as u64;
        Ok(())
    }

    fn rotate(&mut self, at: i64) -> std::io::Result<()> {
        self.current += 1;
        let path = self.dir.join(segment_name(self.current));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        restrict(&path, 0o600);
        self.file = Some(file);
        self.written = 0;
        let dropped = prune(&self.dir, TOTAL_BYTES, self.current);
        self.append(
            &Record::Segment {
                schema: SCHEMA,
                session: self.session.clone(),
                at,
            },
            at,
        )?;
        if dropped > 0 {
            self.append(&Record::Dropped { segments: dropped, at }, at)?;
        }
        Ok(())
    }
}

/// Delete the oldest segments until the directory fits `cap`, never the one
/// being written. Returns how many went.
pub fn prune(dir: &Path, cap: u64, keep: u32) -> u32 {
    let numbers = segments(dir);
    let sizes: Vec<(u32, u64)> = numbers
        .iter()
        .map(|n| {
            let size = fs::metadata(dir.join(segment_name(*n))).map(|meta| meta.len()).unwrap_or(0);
            (*n, size)
        })
        .collect();
    let mut total: u64 = sizes.iter().map(|(_, size)| size).sum();
    let mut dropped = 0;
    for (n, size) in sizes {
        if total <= cap || n == keep {
            break;
        }
        if fs::remove_file(dir.join(segment_name(n))).is_ok() {
            total -= size;
            dropped += 1;
        }
    }
    dropped
}

/// Delete a session's trajectory.
pub fn delete(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_: &Path, _: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::parse_lines;

    #[test]
    fn a_new_directory_starts_a_segment_and_appends_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Writer::open(dir.path(), "s1").unwrap();
        writer.append(&Record::Rewind { keep_turns: 1, at: 3 }, 3).unwrap();
        let text = fs::read_to_string(dir.path().join(segment_name(1))).unwrap();
        let (records, consumed) = parse_lines(&text, 0);
        assert_eq!(consumed, text.len());
        assert!(matches!(records[0], Record::Segment { schema: SCHEMA, .. }));
        assert_eq!(records[1], Record::Rewind { keep_turns: 1, at: 3 });

        // Reopening appends to the same segment.
        let mut again = Writer::open(dir.path(), "s1").unwrap();
        assert_eq!(again.segment(), 1);
        again.append(&Record::Rewind { keep_turns: 0, at: 4 }, 4).unwrap();
        let text = fs::read_to_string(dir.path().join(segment_name(1))).unwrap();
        assert_eq!(parse_lines(&text, 0).0.len(), 3);
    }

    #[test]
    fn oldest_segments_go_past_the_cap_but_never_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=4 {
            fs::write(dir.path().join(segment_name(n)), vec![b'x'; 100]).unwrap();
        }
        assert_eq!(prune(dir.path(), 250, 4), 2);
        assert_eq!(segments(dir.path()), vec![3, 4]);
        assert_eq!(prune(dir.path(), 10, 4), 1);
        assert_eq!(segments(dir.path()), vec![4]);
    }
}
