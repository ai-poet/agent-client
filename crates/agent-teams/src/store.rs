// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Team state on disk, under `<workspace>/<stateDir>/`:
//!
//! - `<teamId>/team.json` — the durable [`TeamState`]
//! - `<teamId>/inbox/<agentKey>.jsonl` — one mailbox per agent (see
//!   [`crate::mailbox`])
//! - `<teamId>/sessions/<memberKey>.json` — a member's conversation, so a
//!   member resumes where it stopped after a restart (Waku addition)
//! - `retired-members.json` — member session ids that must never run again
//! - `archive/<teamId>/` — finished or discarded teams, kept for review
//!
//! Synchronous `std::fs`: every call is a few small files, made by callers
//! that already hold the team's lock (the bridge's keyed async locks).

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, bail};

use crate::key::sanitize_key;
use crate::types::TeamState;
use crate::validate::{parse_team_state, strip_bom};

/// The state directory under the workspace when none is configured.
pub const DEFAULT_STATE_DIR: &str = ".agent-teams";
pub const TEAM_FILE: &str = "team.json";
pub const INBOX_DIR: &str = "inbox";
pub const SESSIONS_DIR: &str = "sessions";
pub const ARCHIVE_DIR: &str = "archive";
/// Durable deny-list for members that must never be resumed.
pub const RETIRED_MEMBERS_FILE: &str = "retired-members.json";

/// Rename attempts before falling back to a direct overwrite.
const ATOMIC_RENAME_RETRIES: u32 = 3;
/// Pause between rename attempts, giving a briefly-locking owner time to
/// finish.
const ATOMIC_RENAME_RETRY_DELAY: Duration = Duration::from_millis(50);

/// The resolved state root of one workspace.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StateRoot {
    root: PathBuf,
}

impl StateRoot {
    /// `<workspace>/<state_dir>`. A relative `state_dir` resolves against the
    /// workspace; an absolute one is taken as-is.
    pub fn new(workspace: &Path, state_dir: &str) -> Self {
        let state_dir = if state_dir.trim().is_empty() {
            DEFAULT_STATE_DIR
        } else {
            state_dir.trim()
        };
        Self {
            root: workspace.join(state_dir),
        }
    }

    pub fn from_path(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The key the bridge's locks and caches use for this root.
    pub fn lock_key(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    pub fn team_dir(&self, team_id: &str) -> PathBuf {
        self.root.join(team_id)
    }

    pub fn team_file(&self, team_id: &str) -> PathBuf {
        self.team_dir(team_id).join(TEAM_FILE)
    }

    pub fn inbox_dir(&self, team_id: &str) -> PathBuf {
        self.team_dir(team_id).join(INBOX_DIR)
    }

    /// Where a member's conversation is kept, by its display name.
    pub fn member_history_file(&self, team_id: &str, member_name: &str) -> PathBuf {
        self.team_dir(team_id)
            .join(SESSIONS_DIR)
            .join(format!("{}.json", sanitize_key(member_name)))
    }

    /// The archive is a state root of its own: same layout, one level down.
    pub fn archive(&self) -> StateRoot {
        StateRoot {
            root: self.root.join(ARCHIVE_DIR),
        }
    }

    /// Create the team directory and write its first record. The state
    /// root keeps itself out of version control: it is working state, not
    /// project content.
    pub fn create_team_dir(&self, state: &TeamState) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.inbox_dir(&state.id))
            .with_context(|| format!("creating team directory for \"{}\"", state.id))?;
        let ignore = self.root.join(".gitignore");
        if !ignore.exists() {
            let _ = std::fs::write(&ignore, "*\n");
        }
        self.write_team(state)
    }

    /// One team record; `None` when absent. A record that fails validation
    /// is an error, never a silent `None`.
    pub fn read_team(&self, team_id: &str) -> anyhow::Result<Option<TeamState>> {
        let path = self.team_file(team_id);
        match std::fs::read_to_string(&path) {
            Ok(text) => parse_team_state(&text, team_id).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Persist one team record (inside the caller's lock).
    pub fn write_team(&self, state: &TeamState) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(state)?;
        atomic_write_text(&self.team_file(&state.id), &text)
            .with_context(|| format!("writing team \"{}\"", state.id))
    }

    /// Every directory that holds a `team.json`, archive excluded.
    pub fn list_team_ids(&self) -> anyhow::Result<Vec<String>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("listing {}", self.root.display()));
            }
        };
        let mut ids = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name == ARCHIVE_DIR || name.starts_with('.') {
                continue;
            }
            if entry.path().join(TEAM_FILE).is_file() {
                ids.push(name);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// The team one captain leads (at most one per captain).
    pub fn find_team_by_captain(
        &self,
        captain_session_id: &str,
    ) -> anyhow::Result<Option<TeamState>> {
        let mut found: Option<TeamState> = None;
        for id in self.list_team_ids()? {
            let Some(team) = self.read_team(&id)? else {
                continue;
            };
            if team.captain_session_id != captain_session_id {
                continue;
            }
            if let Some(previous) = &found
                && previous.id != team.id
            {
                bail!(
                    "captain session leads multiple active teams (\"{}\", \"{}\"); archive one before continuing",
                    previous.id,
                    team.id
                );
            }
            found = Some(team);
        }
        Ok(found)
    }

    /// The team in which one session takes part: as its captain, or as a
    /// member that was not removed.
    pub fn find_team_by_participant(&self, session_id: &str) -> anyhow::Result<Option<TeamState>> {
        let mut found: Option<TeamState> = None;
        for id in self.list_team_ids()? {
            let Some(team) = self.read_team(&id)? else {
                continue;
            };
            let participates =
                team.captain_session_id == session_id || team.member_by_id(session_id).is_some();
            if !participates {
                continue;
            }
            if let Some(previous) = &found
                && previous.id != team.id
            {
                bail!(
                    "agent session belongs to multiple active teams (\"{}\", \"{}\"); the target team is ambiguous",
                    previous.id,
                    team.id
                );
            }
            found = Some(team);
        }
        Ok(found)
    }

    /// The retired member session ids.
    pub fn read_retired_member_ids(&self) -> anyhow::Result<BTreeSet<String>> {
        let path = self.root.join(RETIRED_MEMBERS_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        let ids: Vec<String> = serde_json::from_str(strip_bom(&text))
            .context("invalid AgentTeams retired member index")?;
        if ids.iter().any(String::is_empty) {
            bail!("invalid AgentTeams retired member index");
        }
        Ok(ids.into_iter().collect())
    }

    /// Add session ids to the retired deny-list. The caller serializes this
    /// per state root.
    pub fn record_retired_member_ids<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a str>,
    ) -> anyhow::Result<()> {
        let additions: Vec<&str> = ids.into_iter().filter(|id| !id.is_empty()).collect();
        if additions.is_empty() {
            return Ok(());
        }
        let mut retired = self.read_retired_member_ids()?;
        retired.extend(additions.into_iter().map(str::to_owned));
        std::fs::create_dir_all(&self.root)?;
        let list: Vec<&String> = retired.iter().collect();
        let text = format!("{}\n", serde_json::to_string_pretty(&list)?);
        atomic_write_text(&self.root.join(RETIRED_MEMBERS_FILE), &text)?;
        Ok(())
    }

    /// Remove a team's whole directory.
    pub fn remove_team_dir(&self, team_id: &str) -> anyhow::Result<()> {
        match std::fs::remove_dir_all(self.team_dir(team_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Archive a team instead of deleting it: the whole directory moves under
    /// `archive/<teamId>/`, where later sessions can review how the work was
    /// planned. Only the newest generation of an id is kept.
    pub fn archive_team_dir(&self, team_id: &str) -> anyhow::Result<()> {
        let archive_root = self.root.join(ARCHIVE_DIR);
        std::fs::create_dir_all(&archive_root)?;
        let source = self.team_dir(team_id);
        let target = archive_root.join(team_id);
        let previous = archive_root.join(format!(".{team_id}.previous-{}", uuid::Uuid::new_v4()));
        let displaced = match rename_with_retry(&target, &previous) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if let Err(error) = rename_with_retry(&source, &target) {
            if displaced && let Err(restore) = rename_with_retry(&previous, &target) {
                bail!(
                    "failed to archive team \"{team_id}\" ({error}) and restore its previous archive ({restore})"
                );
            }
            return Err(error.into());
        }
        if displaced {
            let _ = std::fs::remove_dir_all(&previous);
        }
        Ok(())
    }

    pub fn read_archived_team(&self, team_id: &str) -> anyhow::Result<Option<TeamState>> {
        self.archive().read_team(team_id)
    }

    /// Every archived team id, hidden recovery directories excluded.
    pub fn list_archived_team_ids(&self) -> anyhow::Result<Vec<String>> {
        self.archive().list_team_ids()
    }
}

/// Filesystem primitives used by [`replace_file_atomic_or_direct`];
/// injectable for tests.
pub trait ReplacePrimitives {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn write_file(&self, file: &Path, content: &str) -> io::Result<()>;
    fn remove(&self, file: &Path) -> io::Result<()>;
    fn sleep(&self, duration: Duration);
}

struct RealFs;

impl ReplacePrimitives for RealFs {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn write_file(&self, file: &Path, content: &str) -> io::Result<()> {
        std::fs::write(file, content)
    }

    fn remove(&self, file: &Path) -> io::Result<()> {
        std::fs::remove_file(file)
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A rename error worth retrying before the direct-write fallback. On
/// Windows, replacing a target another process holds open without
/// `FILE_SHARE_DELETE` (an editor, an indexer, antivirus) fails with access
/// denied or a sharing violation; those clear quickly.
fn is_retryable_rename_error(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied
            | io::ErrorKind::AlreadyExists
            | io::ErrorKind::DirectoryNotEmpty
            | io::ErrorKind::ResourceBusy
    ) {
        return true;
    }
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

/// Replace `file` with `content`, preferring an atomic rename of the already
/// written `temporary`. When the rename keeps failing, the payload is written
/// in place — content-equivalent, merely not atomic. The temporary file is
/// removed on every path.
pub fn replace_file_atomic_or_direct(
    temporary: &Path,
    file: &Path,
    content: &str,
    primitives: &dyn ReplacePrimitives,
    retries: u32,
    retry_delay: Duration,
) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        let error = match primitives.rename(temporary, file) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if is_retryable_rename_error(&error) && attempt < retries {
            attempt += 1;
            primitives.sleep(retry_delay);
            continue;
        }
        let fallback = primitives.write_file(file, content);
        let _ = primitives.remove(temporary);
        return match fallback {
            Ok(()) => Ok(()),
            Err(write_error) => Err(io::Error::new(
                write_error.kind(),
                format!(
                    "failed to replace \"{}\" atomically ({error}) or by direct write ({write_error})",
                    file.display()
                ),
            )),
        };
    }
}

/// Atomically replace one UTF-8 state file from a same-directory temporary
/// file, degrading to a direct overwrite when the rename cannot proceed.
pub fn atomic_write_text(file: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = PathBuf::from(format!(
        "{}.{}.{}.tmp",
        file.display(),
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let written = (|| {
        use std::io::Write as _;
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
    replace_file_atomic_or_direct(
        &temporary,
        file,
        content,
        &RealFs,
        ATOMIC_RENAME_RETRIES,
        ATOMIC_RENAME_RETRY_DELAY,
    )
}

/// `rename` with the same transient retry policy, for directories, where no
/// direct-write degradation exists.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable_rename_error(&error) && attempt < ATOMIC_RENAME_RETRIES => {
                attempt += 1;
                std::thread::sleep(ATOMIC_RENAME_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::types::{TeamMember, TeamTask};

    fn team(id: &str, captain: &str) -> TeamState {
        let mut team = TeamState::new(id, id, captain, 1);
        let mut member = TeamMember::new("dev", 1);
        member.id = format!("{id}-dev");
        team.members.push(member);
        team.tasks.push(TeamTask::new("t1", "do", 1));
        team.task_seq = 1;
        team
    }

    #[test]
    fn create_read_write_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = StateRoot::new(dir.path(), ".agent-teams");
        assert!(root.read_team("alpha").unwrap().is_none());
        let mut state = team("alpha", "cap");
        root.create_team_dir(&state).unwrap();
        assert!(root.inbox_dir("alpha").is_dir());
        state.description = Some("goal".into());
        root.write_team(&state).unwrap();
        assert_eq!(root.read_team("alpha").unwrap().unwrap(), state);
        let leftovers: Vec<_> = std::fs::read_dir(root.team_dir("alpha"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn find_by_captain_and_participant() {
        let dir = tempfile::tempdir().unwrap();
        let root = StateRoot::new(dir.path(), ".agent-teams");
        root.create_team_dir(&team("alpha", "cap-a")).unwrap();
        root.create_team_dir(&team("beta", "cap-b")).unwrap();
        assert_eq!(
            root.find_team_by_captain("cap-b").unwrap().unwrap().id,
            "beta"
        );
        assert!(root.find_team_by_captain("nobody").unwrap().is_none());
        assert_eq!(
            root.find_team_by_participant("alpha-dev")
                .unwrap()
                .unwrap()
                .id,
            "alpha"
        );
        let mut removed = team("gamma", "cap-c");
        removed.members[0].status = crate::types::MemberStatus::Removed;
        root.create_team_dir(&removed).unwrap();
        assert!(
            root.find_team_by_participant("gamma-dev")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_captain_leading_two_teams_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = StateRoot::new(dir.path(), ".agent-teams");
        root.create_team_dir(&team("alpha", "cap")).unwrap();
        root.create_team_dir(&team("beta", "cap")).unwrap();
        assert!(root.find_team_by_captain("cap").is_err());
    }

    #[test]
    fn archive_keeps_the_newest_generation_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = StateRoot::new(dir.path(), ".agent-teams");
        let mut first = team("alpha", "cap");
        first.description = Some("first".into());
        root.create_team_dir(&first).unwrap();
        root.archive_team_dir("alpha").unwrap();
        assert!(root.read_team("alpha").unwrap().is_none());
        let mut second = team("alpha", "cap");
        second.description = Some("second".into());
        root.create_team_dir(&second).unwrap();
        root.archive_team_dir("alpha").unwrap();
        let archived = root.read_archived_team("alpha").unwrap().unwrap();
        assert_eq!(archived.description.as_deref(), Some("second"));
        assert_eq!(
            root.list_archived_team_ids().unwrap(),
            vec!["alpha".to_owned()]
        );
        assert!(root.list_team_ids().unwrap().is_empty());
    }

    #[test]
    fn retired_members_accumulate_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let root = StateRoot::new(dir.path(), ".agent-teams");
        root.record_retired_member_ids(["b", "", "a"]).unwrap();
        root.record_retired_member_ids(["c", "a"]).unwrap();
        let ids: Vec<String> = root
            .read_retired_member_ids()
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    struct Scripted {
        renames: RefCell<Vec<io::Result<()>>>,
        write: RefCell<Option<io::Result<()>>>,
        removed: RefCell<u32>,
        slept: RefCell<u32>,
    }

    impl ReplacePrimitives for Scripted {
        fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
            self.renames.borrow_mut().remove(0)
        }
        fn write_file(&self, _: &Path, _: &str) -> io::Result<()> {
            self.write.borrow_mut().take().unwrap()
        }
        fn remove(&self, _: &Path) -> io::Result<()> {
            *self.removed.borrow_mut() += 1;
            Ok(())
        }
        fn sleep(&self, _: Duration) {
            *self.slept.borrow_mut() += 1;
        }
    }

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn a_transient_lock_is_retried_then_renamed() {
        let fs = Scripted {
            renames: RefCell::new(vec![Err(denied()), Ok(())]),
            write: RefCell::new(None),
            removed: RefCell::new(0),
            slept: RefCell::new(0),
        };
        replace_file_atomic_or_direct(Path::new("t"), Path::new("f"), "x", &fs, 3, Duration::ZERO)
            .unwrap();
        assert_eq!(*fs.slept.borrow(), 1);
        assert_eq!(*fs.removed.borrow(), 0);
    }

    #[test]
    fn a_persistent_lock_falls_back_to_a_direct_write() {
        let fs = Scripted {
            renames: RefCell::new((0..4).map(|_| Err(denied())).collect()),
            write: RefCell::new(Some(Ok(()))),
            removed: RefCell::new(0),
            slept: RefCell::new(0),
        };
        replace_file_atomic_or_direct(Path::new("t"), Path::new("f"), "x", &fs, 3, Duration::ZERO)
            .unwrap();
        assert_eq!(*fs.slept.borrow(), 3);
        assert_eq!(*fs.removed.borrow(), 1);
    }

    #[test]
    fn both_paths_failing_reports_both() {
        let fs = Scripted {
            renames: RefCell::new(vec![Err(io::Error::other("boom"))]),
            write: RefCell::new(Some(Err(io::Error::other("disk full")))),
            removed: RefCell::new(0),
            slept: RefCell::new(0),
        };
        let error = replace_file_atomic_or_direct(
            Path::new("t"),
            Path::new("f"),
            "x",
            &fs,
            3,
            Duration::ZERO,
        )
        .unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("boom") && text.contains("disk full"),
            "{text}"
        );
        assert_eq!(*fs.removed.borrow(), 1);
    }
}
