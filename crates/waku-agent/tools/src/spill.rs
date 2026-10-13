//! Fork addition (Waku): oversized tool output goes to a file the model can
//! read back, and the conversation keeps its two ends.
//!
//! Each tool capped its own output, or did not: Bash and WebFetch kept
//! 100,000 characters and dropped the middle for good, most tools kept
//! everything. What stayed in could be some 25k tokens in the conversation at
//! once, sent again with every request until a compaction. Past
//! [`SPILL_ABOVE_CHARS`] the whole result is now written to a private file
//! and the conversation carries its head, its tail and where the rest is,
//! for the model to read or grep when it needs more — what DeepSeek Harness
//! and ZCode do.
//!
//! `Read` is never spilled: it already pages through a file, and a spill of
//! a spill file would keep the model from ever seeing it. The spill
//! directory counts as an allowed root (`ToolContext::permission_allowed_roots`),
//! so reading back output the agent produced never asks.

use std::path::{Path, PathBuf};

/// Results longer than this are spilled.
pub const SPILL_ABOVE_CHARS: usize = 40_000;
/// How much of the start and the end stays in the conversation.
const HEAD_CHARS: usize = 16_000;
const TAIL_CHARS: usize = 8_000;
/// Spill files older than this are removed, once per process.
const KEEP_FOR: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Where spilled output is kept.
pub fn root() -> PathBuf {
    claurst_core::config::Settings::config_dir().join("tool-output")
}

/// `content` as the conversation should carry it: unchanged when it is short
/// or comes from `Read`, otherwise its two ends and the path of the whole.
pub fn spill_if_large(session_id: &str, tool_name: &str, content: String) -> String {
    spill_into(&root(), session_id, tool_name, content)
}

fn spill_into(root: &Path, session_id: &str, tool_name: &str, content: String) -> String {
    if tool_name == claurst_core::constants::TOOL_NAME_FILE_READ
        || content.len() <= SPILL_ABOVE_CHARS
    {
        return content;
    }
    let total_chars = content.chars().count();
    if total_chars <= SPILL_ABOVE_CHARS {
        return content;
    }
    let head_end = byte_index_of_char(&content, HEAD_CHARS);
    let tail_start = byte_index_of_char(&content, total_chars - TAIL_CHARS);
    let omitted = total_chars - HEAD_CHARS - TAIL_CHARS;
    let note = match save(root, session_id, tool_name, &content) {
        Ok(path) => format!(
            "[… {omitted} characters omitted. The full output ({total_chars} characters, \
             {lines} lines) is saved at {path} — read parts of it with Read (offset and \
             limit) or search it with Grep.]",
            lines = content.lines().count(),
            path = path.display(),
        ),
        Err(error) => {
            tracing::warn!(%error, tool = tool_name, "agent: could not save a long tool result");
            format!(
                "[… {omitted} characters omitted; the full output could not be saved \
                 ({error}). Narrow the command or query to see the rest.]"
            )
        }
    };
    format!(
        "{}\n\n{note}\n\n{}",
        &content[..head_end],
        &content[tail_start..]
    )
}

/// The byte offset of the `n`th character, or the end.
fn byte_index_of_char(text: &str, n: usize) -> usize {
    text.char_indices()
        .nth(n)
        .map_or(text.len(), |(index, _)| index)
}

fn save(root: &Path, session_id: &str, tool_name: &str, content: &str) -> std::io::Result<PathBuf> {
    clean_old_once(root);
    let directory = root.join(session_directory(session_id));
    create_private_dir(&directory)?;
    let name: String = tool_name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .take(40)
        .collect();
    let path = directory.join(format!("{name}-{}.txt", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    std::io::Write::write_all(&mut file, content.as_bytes())?;
    Ok(path)
}

/// One directory per session, named by a hash of its id: ids carry
/// characters (`parent::call`) that are not safe in a path.
fn session_directory(session_id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    format!("session-{:016x}", hasher.finish())
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Remove spill files past [`KEEP_FOR`], once per process, best effort.
fn clean_old_once(root: &Path) {
    static CLEANED: std::sync::Once = std::sync::Once::new();
    CLEANED.call_once(|| {
        let Ok(sessions) = std::fs::read_dir(root) else {
            return;
        };
        let now = std::time::SystemTime::now();
        for session in sessions.flatten() {
            let Ok(files) = std::fs::read_dir(session.path()) else {
                continue;
            };
            let mut kept = false;
            for file in files.flatten() {
                let old = file
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age > KEEP_FOR);
                if old {
                    let _ = std::fs::remove_file(file.path());
                } else {
                    kept = true;
                }
            }
            if !kept {
                let _ = std::fs::remove_dir(session.path());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_and_reads_stay_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let short = "x".repeat(SPILL_ABOVE_CHARS);
        assert_eq!(spill_into(dir.path(), "s", "Bash", short.clone()), short);
        let long = "y".repeat(SPILL_ABOVE_CHARS * 2);
        assert_eq!(spill_into(dir.path(), "s", "Read", long.clone()), long);
    }

    #[test]
    fn long_output_keeps_its_ends_and_saves_the_whole() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..20_000).map(|n| format!("{n:05}\n")).collect();
        assert!(body.chars().count() > SPILL_ABOVE_CHARS);

        let kept = spill_into(dir.path(), "spill-test", "Bash", body.clone());
        assert!(kept.starts_with("00000\n"), "the head stays");
        assert!(kept.ends_with("19999\n"), "the tail stays");
        assert!(kept.chars().count() < body.chars().count());
        let path = kept
            .lines()
            .find_map(|line| {
                let start = line.find(" is saved at ")? + " is saved at ".len();
                let end = line.find(" — read parts")?;
                Some(line[start..end].to_owned())
            })
            .expect("the note names the file");
        assert_eq!(std::fs::read_to_string(path).unwrap(), body);
    }

    #[test]
    fn the_cut_never_splits_a_character() {
        let text = "汉".repeat(SPILL_ABOVE_CHARS + 10);
        assert_eq!(byte_index_of_char(&text, 3), 9);
        assert_eq!(byte_index_of_char(&text, usize::MAX), text.len());
    }
}
