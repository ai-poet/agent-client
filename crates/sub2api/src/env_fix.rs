//! Removing the environment variables that override written routing.
//!
//! [`crate::env_conflicts`] finds variables that outrank the configuration
//! files this app writes; this module takes them away, which is the other
//! half of the same problem — a warning the user cannot act on from here
//! just moves the work to a terminal.
//!
//! Two rules make that safe enough to offer:
//!
//! * **Nothing is removed before it is written down.** Every run saves the
//!   raw values to a timestamped file under `~/.cheaprouter/env-backups`
//!   first, and [`restore`] puts them back.
//! * **Nothing is guessed.** The value and, for a shell file, the exact
//!   line are re-read from the source at the moment of the change; a file
//!   that no longer holds what the scan saw is left alone and reported.
//!
//! Machine-wide Windows variables need elevation, so they are never touched
//! — the caller is handed the command to run instead. Variables that exist
//! only in this process cannot be unset for an already-running CLI either;
//! they come back from their real source until the app is restarted.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;
use crate::env_conflicts::{ConflictSource, EnvConflict};
use crate::global_config::atomic_write_private;

/// Which Windows environment block a variable lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvScope {
    User,
    Machine,
}

impl EnvScope {
    fn as_powershell(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Machine => "Machine",
        }
    }
}

/// One removed variable, with everything needed to put it back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub name: String,
    /// The raw value as it was at the moment of removal — not masked; this
    /// file is the only way back.
    pub value: String,
    pub source: ConflictSource,
    /// The shell file that carried it, when it came from one.
    #[serde(default)]
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub line: Option<usize>,
    /// The line exactly as it read before being commented out.
    #[serde(default)]
    pub original_line: Option<String>,
}

/// One removal run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    /// Unix seconds.
    #[serde(default)]
    pub created_at: i64,
    /// The product that wrote it, for anyone reading the file by hand.
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub entries: Vec<BackupEntry>,
}

/// What can be done about one conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixPlan {
    /// Deletable from here: the Windows per-user environment block.
    RemoveUserVar,
    /// Machine-wide; needs an elevated shell, so the command is offered
    /// rather than run.
    RemoveMachineVar { elevated_command: String },
    /// A line in a shell startup file, commented out in place.
    CommentOutLine { path: PathBuf, line: usize },
    /// Set only for this process. Changing it would not affect the CLI the
    /// app spawns next, and it will reappear from wherever it really comes
    /// from; the app has to be restarted after fixing that source.
    SkipProcess,
}

/// How a conflict can be dealt with.
pub fn plan(conflict: &EnvConflict) -> FixPlan {
    match &conflict.source {
        ConflictSource::WindowsUser => FixPlan::RemoveUserVar,
        ConflictSource::WindowsMachine => FixPlan::RemoveMachineVar {
            elevated_command: elevated_command(&conflict.name),
        },
        ConflictSource::ShellFile { path, line } => FixPlan::CommentOutLine {
            path: path.clone(),
            line: *line,
        },
        ConflictSource::Process => FixPlan::SkipProcess,
    }
}

/// The command an administrator runs to clear a machine-wide variable.
pub fn elevated_command(name: &str) -> String {
    format!(
        "[Environment]::SetEnvironmentVariable('{}', $null, 'Machine')",
        escape_single_quotes(name)
    )
}

/// What one run did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixReport {
    /// Names actually removed.
    pub removed: Vec<String>,
    /// Names deliberately left alone, and why.
    pub skipped: Vec<(String, String)>,
    /// Where the values were saved, when anything was written.
    pub backup_path: Option<PathBuf>,
    /// Failures; the run continues past them so one bad entry does not
    /// strand the rest.
    pub errors: Vec<String>,
}

impl FixReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && self.skipped.is_empty()
    }
}

/// Reads and writes persisted environment variables.
///
/// A trait so the removal logic — which decides what to save, what to skip,
/// and in which order — is testable without touching a real environment.
pub trait EnvStore {
    fn get(&self, name: &str, scope: EnvScope) -> Result<Option<String>>;
    fn unset(&mut self, name: &str, scope: EnvScope) -> Result<()>;
    fn set(&mut self, name: &str, value: &str, scope: EnvScope) -> Result<()>;
}

/// The real Windows environment, through PowerShell — the same route
/// `env_conflicts` reads it by, so no registry dependency is added.
pub struct PowerShellStore;

impl PowerShellStore {
    fn run(script: &str) -> Result<String> {
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        let run = crate::cli_detect::run_with_timeout(command, Duration::from_secs(5));
        if !run.success {
            return Err(anyhow!(
                "PowerShell refused the change: {}",
                run.output.trim()
            ));
        }
        Ok(run.output)
    }
}

impl EnvStore for PowerShellStore {
    fn get(&self, name: &str, scope: EnvScope) -> Result<Option<String>> {
        let name = checked_name(name)?;
        let value = Self::run(&format!(
            "[Environment]::GetEnvironmentVariable('{name}','{}')",
            scope.as_powershell()
        ))?;
        let value = value.trim_end_matches(['\r', '\n']).to_owned();
        Ok((!value.trim().is_empty()).then_some(value))
    }

    fn unset(&mut self, name: &str, scope: EnvScope) -> Result<()> {
        let name = checked_name(name)?;
        Self::run(&format!(
            "[Environment]::SetEnvironmentVariable('{name}', $null, '{}')",
            scope.as_powershell()
        ))
        .map(|_| ())
    }

    fn set(&mut self, name: &str, value: &str, scope: EnvScope) -> Result<()> {
        let name = checked_name(name)?;
        Self::run(&format!(
            "[Environment]::SetEnvironmentVariable('{name}', '{}', '{}')",
            escape_single_quotes(value),
            scope.as_powershell()
        ))
        .map(|_| ())
    }
}

/// Variable names go into a PowerShell string literal, so they are checked
/// rather than escaped: anything but a plain name is refused outright.
fn checked_name(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if ok {
        Ok(name)
    } else {
        Err(anyhow!("{name} is not a plain environment variable name"))
    }
}

fn escape_single_quotes(value: &str) -> String {
    value.replace('\'', "''")
}

/// Where removal backups are kept.
pub fn backups_dir() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("env-backups"))
}

/// Remove every conflict that can be removed, backing the values up first.
pub fn remove(conflicts: &[EnvConflict]) -> Result<FixReport> {
    let dir = backups_dir().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut store = PowerShellStore;
    remove_with(conflicts, &dir, now_seconds(), &mut store)
}

/// [`remove`] against an explicit backup directory, clock and environment.
pub fn remove_with(
    conflicts: &[EnvConflict],
    backup_dir: &Path,
    now: i64,
    env: &mut dyn EnvStore,
) -> Result<FixReport> {
    let mut report = FixReport::default();
    let mut entries: Vec<BackupEntry> = Vec::new();
    // Two passes: collect first so nothing is changed until every value the
    // run could need is recorded.
    let mut pending: Vec<(usize, FixPlan)> = Vec::new();
    for (index, conflict) in conflicts.iter().enumerate() {
        let plan = plan(conflict);
        match &plan {
            FixPlan::SkipProcess => {
                report.skipped.push((
                    conflict.name.clone(),
                    "set for this process; remove it at its source and restart".to_owned(),
                ));
                continue;
            }
            FixPlan::RemoveMachineVar { .. } => {
                report.skipped.push((
                    conflict.name.clone(),
                    "machine-wide; needs an elevated shell".to_owned(),
                ));
                continue;
            }
            FixPlan::RemoveUserVar => match env.get(&conflict.name, EnvScope::User) {
                Ok(Some(value)) => entries.push(BackupEntry {
                    name: conflict.name.clone(),
                    value,
                    source: conflict.source.clone(),
                    file: None,
                    line: None,
                    original_line: None,
                }),
                Ok(None) => {
                    report
                        .skipped
                        .push((conflict.name.clone(), "already gone".to_owned()));
                    continue;
                }
                Err(error) => {
                    report
                        .errors
                        .push(format!("{}: {error:#}", conflict.name));
                    continue;
                }
            },
            FixPlan::CommentOutLine { path, line } => {
                match read_shell_line(path, *line, &conflict.name) {
                    Ok((original_line, value)) => entries.push(BackupEntry {
                        name: conflict.name.clone(),
                        value,
                        source: conflict.source.clone(),
                        file: Some(path.clone()),
                        line: Some(*line),
                        original_line: Some(original_line),
                    }),
                    Err(error) => {
                        report
                            .errors
                            .push(format!("{}: {error:#}", conflict.name));
                        continue;
                    }
                }
            }
        }
        pending.push((index, plan));
    }

    if entries.is_empty() {
        return Ok(report);
    }

    let backup = Backup {
        created_at: now,
        app: brand::DISPLAY_NAME.to_owned(),
        entries: entries.clone(),
    };
    let path = backup_dir.join(format!("{now}.json"));
    write_backup(&path, &backup)?;
    report.backup_path = Some(path);

    for (index, plan) in pending {
        let conflict = &conflicts[index];
        let entry = entries
            .iter()
            .find(|entry| entry.name == conflict.name && entry.source == conflict.source);
        let outcome = match plan {
            FixPlan::RemoveUserVar => env.unset(&conflict.name, EnvScope::User),
            FixPlan::CommentOutLine { path, line } => {
                let original = entry.and_then(|entry| entry.original_line.clone());
                match original {
                    Some(original) => comment_out_in_file(&path, line, &original, now),
                    None => Err(anyhow!("nothing was recorded for this line")),
                }
            }
            FixPlan::RemoveMachineVar { .. } | FixPlan::SkipProcess => continue,
        };
        match outcome {
            Ok(()) => report.removed.push(conflict.name.clone()),
            Err(error) => report
                .errors
                .push(format!("{}: {error:#}", conflict.name)),
        }
    }
    Ok(report)
}

/// The newest backup in `dir`, if any.
pub fn latest_backup(dir: &Path) -> Option<PathBuf> {
    let mut newest: Option<(i64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        // The name is the timestamp, so ordering needs no file metadata.
        let Some(stamp) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<i64>().ok())
        else {
            continue;
        };
        if newest.as_ref().is_none_or(|(known, _)| stamp > *known) {
            newest = Some((stamp, path));
        }
    }
    newest.map(|(_, path)| path)
}

/// Put back everything one backup recorded.
pub fn restore(path: &Path) -> Result<FixReport> {
    let mut store = PowerShellStore;
    restore_with(path, &mut store)
}

/// [`restore`] against an explicit environment.
pub fn restore_with(path: &Path, env: &mut dyn EnvStore) -> Result<FixReport> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let backup: Backup =
        serde_json::from_str(&raw).with_context(|| format!("could not parse {}", path.display()))?;
    let mut report = FixReport {
        backup_path: Some(path.to_owned()),
        ..FixReport::default()
    };
    for entry in &backup.entries {
        let outcome = match &entry.source {
            ConflictSource::WindowsUser => env.set(&entry.name, &entry.value, EnvScope::User),
            ConflictSource::WindowsMachine => {
                // Never written from here; the command carries the value so
                // an administrator can finish the job.
                report.skipped.push((
                    entry.name.clone(),
                    format!(
                        "[Environment]::SetEnvironmentVariable('{}', '{}', 'Machine')",
                        escape_single_quotes(&entry.name),
                        escape_single_quotes(&entry.value)
                    ),
                ));
                continue;
            }
            ConflictSource::Process => {
                report
                    .skipped
                    .push((entry.name.clone(), "was never removed".to_owned()));
                continue;
            }
            ConflictSource::ShellFile { .. } => match (&entry.file, &entry.original_line) {
                (Some(file), Some(original)) => restore_in_file(file, original),
                _ => Err(anyhow!("the backup recorded no line to restore")),
            },
        };
        match outcome {
            Ok(()) => report.removed.push(entry.name.clone()),
            Err(error) => {
                let hint = entry
                    .original_line
                    .clone()
                    .unwrap_or_else(|| entry.value.clone());
                report
                    .errors
                    .push(format!("{}: {error:#} — {hint}", entry.name));
            }
        }
    }
    Ok(report)
}

// ── shell files ────────────────────────────────────────────────────────────

/// The comment that replaces a removed export, and the marker restore looks
/// for. Ends with the original line so the file itself explains the change.
pub fn marker_for(original_line: &str, now: i64) -> String {
    format!(
        "# removed by {} ({now}): {original_line}",
        brand::DISPLAY_NAME
    )
}

/// Byte range of one 1-based line's content, excluding its terminator.
fn line_span(contents: &str, line: usize) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    for (index, raw) in contents.split_inclusive('\n').enumerate() {
        if index + 1 == line {
            let content = raw.trim_end_matches('\n').trim_end_matches('\r');
            return Some((offset, offset + content.len()));
        }
        offset += raw.len();
    }
    None
}

/// Replace line `line` with a comment carrying the original. Returns the new
/// contents and the line that was replaced. Refuses when that line does not
/// actually assign `name` — the file changed since it was scanned.
pub fn comment_out_line(
    contents: &str,
    line: usize,
    name: &str,
    now: i64,
) -> Result<(String, String)> {
    let (start, end) = line_span(contents, line)
        .ok_or_else(|| anyhow!("the file has no line {line} any more"))?;
    let original = &contents[start..end];
    let assigns = crate::env_conflicts::parse_shell_exports(original)
        .into_iter()
        .any(|(found, ..)| found.eq_ignore_ascii_case(name));
    if !assigns {
        return Err(anyhow!(
            "line {line} no longer sets {name}; rescan and try again"
        ));
    }
    let original = original.to_owned();
    let mut updated = String::with_capacity(contents.len() + 48);
    updated.push_str(&contents[..start]);
    updated.push_str(&marker_for(&original, now));
    updated.push_str(&contents[end..]);
    Ok((updated, original))
}

/// Put a commented-out line back. The marker is found by content, so a file
/// edited elsewhere in the meantime still restores correctly.
pub fn restore_line(contents: &str, original_line: &str) -> Result<String> {
    let mut offset = 0usize;
    for raw in contents.split_inclusive('\n') {
        let content = raw.trim_end_matches('\n').trim_end_matches('\r');
        if content.trim_end().ends_with(original_line.trim_end())
            && content.trim_start().starts_with('#')
            && content.contains(brand::DISPLAY_NAME)
        {
            let mut updated = String::with_capacity(contents.len());
            updated.push_str(&contents[..offset]);
            updated.push_str(original_line);
            updated.push_str(&contents[offset + content.len()..]);
            return Ok(updated);
        }
        offset += raw.len();
    }
    Err(anyhow!("the commented-out line is no longer in the file"))
}

/// Read one line of a shell file and the value it assigns.
fn read_shell_line(path: &Path, line: usize, name: &str) -> Result<(String, String)> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let (start, end) =
        line_span(&contents, line).ok_or_else(|| anyhow!("the file has no line {line}"))?;
    let original = contents[start..end].to_owned();
    let value = crate::env_conflicts::parse_shell_exports(&original)
        .into_iter()
        .find(|(found, ..)| found.eq_ignore_ascii_case(name))
        .map(|(_, value, _)| value)
        .ok_or_else(|| anyhow!("line {line} no longer sets {name}; rescan and try again"))?;
    Ok((original, value))
}

fn comment_out_in_file(path: &Path, line: usize, expected: &str, now: i64) -> Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let (start, end) =
        line_span(&contents, line).ok_or_else(|| anyhow!("the file has no line {line}"))?;
    if contents[start..end] != *expected {
        return Err(anyhow!("{} changed while reading it", path.display()));
    }
    let mut updated = String::with_capacity(contents.len() + 48);
    updated.push_str(&contents[..start]);
    updated.push_str(&marker_for(expected, now));
    updated.push_str(&contents[end..]);
    write_preserving_mode(path, &updated)
}

fn restore_in_file(path: &Path, original_line: &str) -> Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let updated = restore_line(&contents, original_line)?;
    write_preserving_mode(path, &updated)
}

/// Rewrite a file the user owns, keeping its permissions. Deliberately not
/// [`atomic_write_private`]: a shell profile must not become 0600.
fn write_preserving_mode(path: &Path, contents: &str) -> Result<()> {
    #[cfg(unix)]
    let mode = std::fs::metadata(path)
        .ok()
        .map(|metadata| std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()));
    std::fs::write(path, contents)
        .with_context(|| format!("could not write {}", path.display()))?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        let _ = std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode));
    }
    Ok(())
}

fn write_backup(path: &Path, backup: &Backup) -> Result<()> {
    let mut encoded =
        serde_json::to_string_pretty(backup).context("could not encode the backup")?;
    encoded.push('\n');
    atomic_write_private(path, encoded.as_bytes())
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        values: BTreeMap<(String, &'static str), String>,
        fail_unset: bool,
    }

    impl MemoryStore {
        fn with(name: &str, value: &str) -> Self {
            let mut store = Self::default();
            store
                .values
                .insert((name.to_owned(), "User"), value.to_owned());
            store
        }
    }

    impl EnvStore for MemoryStore {
        fn get(&self, name: &str, scope: EnvScope) -> Result<Option<String>> {
            Ok(self
                .values
                .get(&(name.to_owned(), scope.as_powershell()))
                .cloned())
        }

        fn unset(&mut self, name: &str, scope: EnvScope) -> Result<()> {
            if self.fail_unset {
                return Err(anyhow!("access denied"));
            }
            self.values.remove(&(name.to_owned(), scope.as_powershell()));
            Ok(())
        }

        fn set(&mut self, name: &str, value: &str, scope: EnvScope) -> Result<()> {
            self.values.insert(
                (name.to_owned(), scope.as_powershell()),
                value.to_owned(),
            );
            Ok(())
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cheaprouter-env-fix-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn user_conflict(name: &str) -> EnvConflict {
        EnvConflict {
            provider_id: "claude",
            name: name.to_owned(),
            value_masked: "…".to_owned(),
            source: ConflictSource::WindowsUser,
        }
    }

    fn file_conflict(name: &str, path: &Path, line: usize) -> EnvConflict {
        EnvConflict {
            provider_id: "claude",
            name: name.to_owned(),
            value_masked: "…".to_owned(),
            source: ConflictSource::ShellFile {
                path: path.to_owned(),
                line,
            },
        }
    }

    #[test]
    fn plans_match_the_source() {
        assert_eq!(plan(&user_conflict("ANTHROPIC_BASE_URL")), FixPlan::RemoveUserVar);
        assert!(matches!(
            plan(&EnvConflict {
                source: ConflictSource::WindowsMachine,
                ..user_conflict("ANTHROPIC_BASE_URL")
            }),
            FixPlan::RemoveMachineVar { .. }
        ));
        assert_eq!(
            plan(&EnvConflict {
                source: ConflictSource::Process,
                ..user_conflict("ANTHROPIC_BASE_URL")
            }),
            FixPlan::SkipProcess
        );
    }

    #[test]
    fn elevated_command_quotes_name() {
        assert_eq!(
            elevated_command("ANTHROPIC_BASE_URL"),
            "[Environment]::SetEnvironmentVariable('ANTHROPIC_BASE_URL', $null, 'Machine')"
        );
        // A pasted apostrophe cannot end the literal early.
        assert!(elevated_command("A'B").contains("'A''B'"));
        assert!(checked_name("ANTHROPIC_BASE_URL").is_ok());
        assert!(checked_name("A'B; rm -rf /").is_err());
        assert!(checked_name("").is_err());
    }

    #[test]
    fn comment_out_then_restore_round_trips_lf_and_crlf() {
        for (label, newline) in [("lf", "\n"), ("crlf", "\r\n")] {
            let original = format!("export ANTHROPIC_BASE_URL=https://a.example.org");
            let contents = format!(
                "# profile{newline}export PATH=$PATH:/x{newline}{original}{newline}alias k=kubectl{newline}"
            );
            let (updated, seen) =
                comment_out_line(&contents, 3, "ANTHROPIC_BASE_URL", 1700).expect(label);
            assert_eq!(seen, original, "{label}");
            assert!(updated.contains(&marker_for(&original, 1700)), "{label}");
            assert!(!updated.contains(&format!("{newline}{original}{newline}")), "{label}");
            // Every other byte, the line endings and the trailing newline
            // included, is untouched.
            assert!(updated.contains("alias k=kubectl"), "{label}");
            assert_eq!(
                updated.matches(newline).count(),
                contents.matches(newline).count(),
                "{label}"
            );
            assert!(updated.ends_with(newline), "{label}");

            let restored = restore_line(&updated, &original).expect(label);
            assert_eq!(restored, contents, "{label}");
        }
    }

    #[test]
    fn comment_out_refuses_a_line_that_changed() {
        let contents = "export OTHER=1\n";
        let error = comment_out_line(contents, 1, "ANTHROPIC_BASE_URL", 1).unwrap_err();
        assert!(format!("{error}").contains("no longer sets"), "{error}");
        assert!(comment_out_line(contents, 9, "OTHER", 1).is_err());
        // Restore needs the marker, not just the text.
        assert!(restore_line("export OTHER=1\n", "export OTHER=1").is_err());
    }

    #[test]
    fn removes_a_user_variable_after_writing_the_backup() {
        let dir = temp_dir("user");
        let mut store = MemoryStore::with("ANTHROPIC_BASE_URL", "https://secret.example.org");
        let report = remove_with(
            &[user_conflict("ANTHROPIC_BASE_URL")],
            &dir,
            1700,
            &mut store,
        )
        .expect("remove");
        assert_eq!(report.removed, vec!["ANTHROPIC_BASE_URL".to_owned()]);
        assert!(report.errors.is_empty());
        assert!(store.values.is_empty());

        let path = report.backup_path.clone().expect("backup path");
        let backup: Backup =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        // The raw value, not the masked one, or the restore would be useless.
        assert_eq!(backup.entries[0].value, "https://secret.example.org");
        assert_eq!(backup.created_at, 1700);
        assert_eq!(latest_backup(&dir).as_deref(), Some(path.as_path()));

        let report = restore_with(&path, &mut store).expect("restore");
        assert_eq!(report.removed, vec!["ANTHROPIC_BASE_URL".to_owned()]);
        assert_eq!(
            store
                .get("ANTHROPIC_BASE_URL", EnvScope::User)
                .unwrap()
                .as_deref(),
            Some("https://secret.example.org")
        );
    }

    #[test]
    fn backup_is_written_before_any_change() {
        let dir = temp_dir("fail");
        let mut store = MemoryStore::with("ANTHROPIC_API_KEY", "sk-secret-value");
        store.fail_unset = true;
        let report = remove_with(
            &[user_conflict("ANTHROPIC_API_KEY")],
            &dir,
            1800,
            &mut store,
        )
        .expect("remove");
        assert!(report.removed.is_empty());
        assert_eq!(report.errors.len(), 1);
        // The value survived in the backup even though the removal failed.
        let path = report.backup_path.expect("backup path");
        assert!(std::fs::read_to_string(path).unwrap().contains("sk-secret-value"));
    }

    #[test]
    fn machine_and_process_scopes_are_skipped() {
        let dir = temp_dir("skip");
        let mut store = MemoryStore::default();
        let conflicts = vec![
            EnvConflict {
                source: ConflictSource::WindowsMachine,
                ..user_conflict("ANTHROPIC_BASE_URL")
            },
            EnvConflict {
                source: ConflictSource::Process,
                ..user_conflict("OPENAI_API_KEY")
            },
        ];
        let report = remove_with(&conflicts, &dir, 1900, &mut store).expect("remove");
        assert!(report.removed.is_empty());
        assert!(report.errors.is_empty());
        assert_eq!(report.skipped.len(), 2);
        // Nothing to save, so nothing was written.
        assert!(report.backup_path.is_none());
        assert!(latest_backup(&dir).is_none());
    }

    #[test]
    fn comments_out_a_shell_line_and_restores_it() {
        let dir = temp_dir("shell");
        let profile = dir.join(".zshrc");
        let contents = "# setup\nexport ANTHROPIC_BASE_URL=\"https://a.example.org\"\nexport PATH=$PATH:/x\n";
        std::fs::write(&profile, contents).expect("write profile");
        let mut store = MemoryStore::default();
        let report = remove_with(
            &[file_conflict("ANTHROPIC_BASE_URL", &profile, 2)],
            &dir,
            2000,
            &mut store,
        )
        .expect("remove");
        assert_eq!(report.removed, vec!["ANTHROPIC_BASE_URL".to_owned()]);
        let after = std::fs::read_to_string(&profile).expect("read");
        assert!(after.contains("# removed by"), "{after}");
        assert!(after.contains("export PATH=$PATH:/x"), "{after}");
        assert!(!after.contains("\nexport ANTHROPIC_BASE_URL="), "{after}");

        // The quoted value, not the quotes, is what the backup records.
        let path = report.backup_path.expect("backup path");
        let backup: Backup =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(backup.entries[0].value, "https://a.example.org");

        let report = restore_with(&path, &mut store).expect("restore");
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(std::fs::read_to_string(&profile).expect("read"), contents);
    }

    #[test]
    fn a_shell_line_that_moved_is_reported_not_guessed() {
        let dir = temp_dir("moved");
        let profile = dir.join(".bashrc");
        std::fs::write(&profile, "export OTHER=1\nexport MORE=2\n").expect("write");
        let mut store = MemoryStore::default();
        let report = remove_with(
            &[file_conflict("ANTHROPIC_BASE_URL", &profile, 1)],
            &dir,
            2100,
            &mut store,
        )
        .expect("remove");
        assert!(report.removed.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("no longer sets"), "{report:?}");
        // The file is exactly as it was.
        assert_eq!(
            std::fs::read_to_string(&profile).expect("read"),
            "export OTHER=1\nexport MORE=2\n"
        );
    }

    #[test]
    fn latest_backup_picks_the_newest() {
        let dir = temp_dir("newest");
        for stamp in ["100", "2000", "300"] {
            std::fs::write(dir.join(format!("{stamp}.json")), "{}").expect("write");
        }
        std::fs::write(dir.join("notes.txt"), "x").expect("write");
        std::fs::write(dir.join("bad.json"), "{}").expect("write");
        assert_eq!(
            latest_backup(&dir).and_then(|path| path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())),
            Some("2000.json".to_owned())
        );
    }
}
