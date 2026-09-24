//! Daemon-owned Git checkpoint capture and restoration.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use anyhow::{Context as _, anyhow, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Checkpoint, CheckpointFile, CheckpointStatus, unix_time};
use waku_protocol::{TURN_UNDO_STALE, TurnUndoPlan, UndoFile, UndoReason};

const TURN_START_METADATA_PREFIX: &str = "Waku-Turn-Start: ";
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Deserialize, Serialize)]
struct TurnStartMetadata {
    head: Option<String>,
    branch: Option<String>,
    refs: BTreeMap<String, String>,
}

pub fn checkpoint_ref(session_id: Uuid, turn_count: usize) -> String {
    format!("refs/waku/session-{session_id}-turn-{turn_count}")
}

pub fn turn_start_ref(session_id: Uuid, turn_count: usize) -> String {
    format!("refs/waku/session-{session_id}-turn-start-{turn_count}")
}

pub fn turn_diff_base_ref(session_id: Uuid, turn_count: usize) -> String {
    format!("refs/waku/session-{session_id}-turn-diff-{turn_count}")
}

/// The worktree as it stood just before a turn's changes were undone, kept
/// with the session's other refs so a rewind past the turn clears it too.
pub fn turn_undo_backup_ref(session_id: Uuid, turn_count: usize) -> String {
    format!("refs/waku/session-{session_id}-turn-undo-{turn_count}")
}

/// Capture the exact workspace state accepted for a turn before its provider
/// starts. This is intentionally distinct from the preceding turn's ending
/// checkpoint: a branch switch or terminal edit between turns must not be
/// attributed to either response.
pub fn capture_turn_start(cwd: &Path, session_id: Uuid, turn_count: usize) -> anyhow::Result<()> {
    if !is_git_repository(cwd) {
        return Ok(());
    }

    let head = resolve_ref(cwd, "HEAD");
    let branch = symbolic_head(cwd);
    let refs = repository_refs(cwd)?;
    let metadata = TurnStartMetadata {
        head: head.clone(),
        branch,
        refs,
    };
    let message = format!(
        "Waku turn start snapshot\n\n{TURN_START_METADATA_PREFIX}{}",
        serde_json::to_string(&metadata)?
    );
    let mut parents = Vec::new();
    let mut seen = HashSet::new();
    if let Some(head) = head.as_ref()
        && seen.insert(head.clone())
    {
        parents.push(head.clone());
    }
    for commit in metadata.refs.values() {
        if seen.insert(commit.clone()) {
            parents.push(commit.clone());
        }
    }
    let commit = capture_worktree_commit_from(cwd, head.as_deref(), &message, &parents)?;
    let start_ref = turn_start_ref(session_id, turn_count);
    let baseline_ref = checkpoint_ref(session_id, turn_count.saturating_sub(1));
    let mut commands = format!("update {start_ref} {commit}\n");
    if !has_ref(cwd, &baseline_ref) {
        commands.push_str(&format!("update {baseline_ref} {commit}\n"));
    }
    update_refs(cwd, commands)
}

pub fn capture_turn(cwd: &Path, session_id: Uuid, turn_count: usize) -> anyhow::Result<Checkpoint> {
    let git_ref = checkpoint_ref(session_id, turn_count);
    if !is_git_repository(cwd) {
        return Ok(Checkpoint {
            turn_count,
            git_ref,
            status: CheckpointStatus::Unavailable,
            files: Vec::new(),
            additions: 0,
            deletions: 0,
            created_at: unix_time(),
        });
    }

    let end_branch = symbolic_head(cwd);
    let end_head = resolve_ref(cwd, "HEAD");
    let end_commit =
        capture_worktree_commit_from(cwd, end_head.as_deref(), "Waku worktree snapshot", &[])?;
    git_output(cwd, ["update-ref", &git_ref, &end_commit])?;
    let files = if turn_count == 0 {
        Vec::new()
    } else {
        let start_ref = turn_start_ref(session_id, turn_count);
        let legacy_ref = checkpoint_ref(session_id, turn_count - 1);
        // Resolved once, then carried as a raw commit id: the provider runs
        // arbitrary commands in this repository during the turn, so the ref
        // can be deleted or repacked between any two git calls (a rewind's
        // cleanup can remove it too). A commit id stays valid regardless, and
        // a start ref that is already gone degrades to the legacy base
        // instead of failing the whole capture.
        let diff_base = if let Some(start_commit) = resolve_ref(cwd, &start_ref) {
            prepare_turn_diff_base(
                cwd,
                session_id,
                turn_count,
                &start_commit,
                end_head.as_deref(),
                end_branch.as_deref(),
            )?
        } else {
            legacy_ref
        };
        if has_ref(cwd, &diff_base) {
            diff_files(cwd, &diff_base, &git_ref)?
        } else {
            Vec::new()
        }
    };
    let additions = files.iter().map(|file| file.additions).sum();
    let deletions = files.iter().map(|file| file.deletions).sum();
    Ok(Checkpoint {
        turn_count,
        git_ref,
        status: CheckpointStatus::Ready,
        files,
        additions,
        deletions,
        created_at: unix_time(),
    })
}

pub fn capture_ref(cwd: &Path, git_ref: &str) -> anyhow::Result<()> {
    if !is_git_repository(cwd) {
        bail!("checkpoints require a Git repository");
    }

    let commit = capture_worktree_commit(cwd)?;
    git_output(cwd, ["update-ref", git_ref, &commit])?;
    Ok(())
}

/// Capture the current worktree and untracked files as a dangling commit.
///
/// This shares the checkpoint path's isolated temporary index, so it never
/// stages or unstages the user's files. Review uses the returned treeish to
/// compare a stable worktree snapshot while edits continue on disk.
pub fn capture_worktree_commit(cwd: &Path) -> anyhow::Result<String> {
    if !is_git_repository(cwd) {
        bail!("worktree snapshots require a Git repository");
    }

    let head = resolve_ref(cwd, "HEAD");
    capture_worktree_commit_from(cwd, head.as_deref(), "Waku worktree snapshot", &[])
}

fn capture_worktree_commit_from(
    cwd: &Path,
    head: Option<&str>,
    message: &str,
    parents: &[String],
) -> anyhow::Result<String> {
    if !is_git_repository(cwd) {
        bail!("worktree snapshots require a Git repository");
    }

    let common_dir = git_output(cwd, ["rev-parse", "--git-common-dir"])?
        .trim()
        .to_owned();
    if common_dir.is_empty() {
        bail!("git did not return its common directory");
    }
    let common_dir = PathBuf::from(common_dir);
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        cwd.join(common_dir)
    };
    let temporary_index = common_dir.join(format!("waku-checkpoint-index-{}", Uuid::new_v4()));

    let result = (|| {
        // Seed the temporary index from the repository's own index when it
        // has one. That copy carries each file's cached stat data, so
        // `add -A` re-hashes only what changed since the last git operation.
        // An index rebuilt from HEAD has none and must read and hash every
        // file in the worktree — on a few thousand files that is seconds per
        // turn, paid before the provider even starts. The worktree, not the
        // copied index, is still what gets recorded: `add -A` reconciles the
        // two. Should the seeded index refuse to become a tree (unmerged
        // entries, say), fall back to the HEAD-based rebuild.
        let tree = match snapshot_tree(cwd, &temporary_index, head, true) {
            Ok(tree) => tree,
            Err(_) => {
                let _ = fs::remove_file(&temporary_index);
                snapshot_tree(cwd, &temporary_index, head, false)?
            }
        };
        let mut arguments = vec![
            "commit-tree".to_owned(),
            tree,
            "-m".to_owned(),
            message.to_owned(),
        ];
        for parent in parents {
            arguments.push("-p".to_owned());
            arguments.push(parent.clone());
        }
        let commit = git_with_identity_and_index(cwd, &temporary_index, &arguments)?
            .trim()
            .to_owned();
        if commit.is_empty() {
            bail!("git commit-tree returned no object id");
        }
        Ok(commit)
    })();

    let _ = fs::remove_file(&temporary_index);
    let _ = fs::remove_file(temporary_index.with_extension("lock"));
    result
}

/// Stage the whole worktree into `temporary_index` and write it as a tree.
/// `seed_from_index` starts from a copy of the repository's index (fast,
/// stat-cached); otherwise from HEAD's tree (exact, but every file is read).
fn snapshot_tree(
    cwd: &Path,
    temporary_index: &Path,
    head: Option<&str>,
    seed_from_index: bool,
) -> anyhow::Result<String> {
    let seeded = seed_from_index
        && repository_index_path(cwd)
            .filter(|index| index.is_file())
            .is_some_and(|index| fs::copy(&index, temporary_index).is_ok());
    if !seeded && let Some(head) = head {
        git_with_index(cwd, temporary_index, ["read-tree", head])?;
    }
    git_with_index(cwd, temporary_index, ["add", "-A", "--", "."])?;
    let tree = git_with_index(cwd, temporary_index, ["write-tree"])?
        .trim()
        .to_owned();
    if tree.is_empty() {
        bail!("git write-tree returned no object id");
    }
    Ok(tree)
}

/// The repository's index file — per worktree, which `--git-path` resolves
/// and `--git-common-dir` would not.
fn repository_index_path(cwd: &Path) -> Option<PathBuf> {
    let path = git_output(cwd, ["rev-parse", "--git-path", "index"])
        .ok()?
        .trim()
        .to_owned();
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    Some(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn prepare_turn_diff_base(
    cwd: &Path,
    session_id: Uuid,
    turn_count: usize,
    start_commit: &str,
    end_head: Option<&str>,
    end_branch: Option<&str>,
) -> anyhow::Result<String> {
    let metadata = turn_start_metadata(cwd, start_commit)?;
    let same_line = match (metadata.branch.as_deref(), end_branch) {
        (Some(start), Some(end)) => start == end,
        (None, None) => metadata.head.as_deref() == end_head,
        _ => false,
    };

    let commit = if same_line {
        start_commit.to_owned()
    } else {
        let target_base = target_branch_start(cwd, start_commit, &metadata, end_head, end_branch)?;
        virtual_branch_start(cwd, start_commit, metadata.head.as_deref(), &target_base)?
    };
    let diff_ref = turn_diff_base_ref(session_id, turn_count);
    git_output(cwd, ["update-ref", &diff_ref, &commit])?;
    Ok(diff_ref)
}

fn turn_start_metadata(cwd: &Path, start_commit: &str) -> anyhow::Result<TurnStartMetadata> {
    let message = git_output(cwd, ["show", "-s", "--format=%B", start_commit])?;
    let encoded = message
        .lines()
        .find_map(|line| line.strip_prefix(TURN_START_METADATA_PREFIX))
        .ok_or_else(|| anyhow!("turn starting checkpoint metadata is unavailable"))?;
    serde_json::from_str(encoded).context("invalid turn starting checkpoint metadata")
}

fn target_branch_start(
    cwd: &Path,
    start_commit: &str,
    metadata: &TurnStartMetadata,
    end_head: Option<&str>,
    end_branch: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(branch) = end_branch
        && let Some(commit) = metadata.refs.get(branch)
    {
        return Ok(commit.clone());
    }
    let Some(end_head) = end_head else {
        return empty_tree_commit(cwd);
    };
    let new_commits = git_output(
        cwd,
        [
            "rev-list",
            "--first-parent",
            "--reverse",
            end_head,
            "--not",
            start_commit,
        ],
    )?;
    let Some(first_new_commit) = new_commits.lines().find(|line| !line.trim().is_empty()) else {
        return Ok(end_head.to_owned());
    };
    resolve_ref(cwd, &format!("{first_new_commit}^1")).map_or_else(|| empty_tree_commit(cwd), Ok)
}

fn virtual_branch_start(
    cwd: &Path,
    start_commit: &str,
    start_head: Option<&str>,
    target_base: &str,
) -> anyhow::Result<String> {
    let Some(start_head) = start_head else {
        return Ok(target_base.to_owned());
    };
    if start_head == target_base {
        return Ok(start_commit.to_owned());
    }
    if git_output(cwd, ["diff", "--name-only", start_head, start_commit])?
        .trim()
        .is_empty()
    {
        return Ok(target_base.to_owned());
    }

    // Recreate the state Git would have after carrying the user's pre-turn
    // dirty files onto the target branch. Comparing against the raw target tip
    // would otherwise attribute those already-present edits to the response.
    let output = git_output(
        cwd,
        [
            "merge-tree",
            "--write-tree",
            "--merge-base",
            start_head,
            target_base,
            start_commit,
        ],
    )?;
    let tree = output
        .lines()
        .next()
        .map(str::trim)
        .filter(|tree| !tree.is_empty())
        .ok_or_else(|| anyhow!("git merge-tree returned no tree"))?;
    commit_tree(cwd, tree, "Waku turn diff base", &[])
}

fn empty_tree_commit(cwd: &Path) -> anyhow::Result<String> {
    commit_tree(cwd, EMPTY_TREE, "Waku empty turn diff base", &[])
}

fn commit_tree(
    cwd: &Path,
    tree: &str,
    message: &str,
    parents: &[String],
) -> anyhow::Result<String> {
    let common_dir = git_output(cwd, ["rev-parse", "--git-common-dir"])?
        .trim()
        .to_owned();
    let common_dir = PathBuf::from(common_dir);
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        cwd.join(common_dir)
    };
    let temporary_index = common_dir.join(format!("waku-checkpoint-index-{}", Uuid::new_v4()));
    let mut arguments = vec![
        "commit-tree".to_owned(),
        tree.to_owned(),
        "-m".to_owned(),
        message.to_owned(),
    ];
    for parent in parents {
        arguments.push("-p".to_owned());
        arguments.push(parent.clone());
    }
    git_with_identity_and_index(cwd, &temporary_index, &arguments)
        .map(|commit| commit.trim().to_owned())
}

pub fn restore_ref(cwd: &Path, git_ref: &str) -> anyhow::Result<()> {
    let commit = resolve_ref(cwd, git_ref)
        .ok_or_else(|| anyhow!("checkpoint `{git_ref}` is unavailable"))?;
    git_output(
        cwd,
        [
            "restore",
            "--source",
            &commit,
            "--worktree",
            "--staged",
            "--",
            ".",
        ],
    )?;
    git_output(cwd, ["clean", "-fd", "--", "."])?;
    if has_head(cwd) {
        git_output(cwd, ["reset", "--quiet", "--", "."])?;
    }
    Ok(())
}

pub fn has_ref(cwd: &Path, git_ref: &str) -> bool {
    resolve_ref(cwd, git_ref).is_some()
}

/// Every turn count that has a checkpoint ref for `session_id`, resolved with
/// a single `git for-each-ref` instead of one `git rev-parse` per turn.
pub fn session_turn_refs(cwd: &Path, session_id: Uuid) -> HashSet<usize> {
    session_checkpoint_ref_commits(cwd, session_id)
        .turns
        .into_keys()
        .collect()
}

pub fn delete_ref(cwd: &Path, git_ref: &str) -> anyhow::Result<()> {
    let output = crate::command_env::plain_command("git")
        .args(["update-ref", "-d", git_ref])
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to delete checkpoint `{git_ref}`"))?;
    if output.status.success() {
        Ok(())
    } else {
        bail!("{}", command_error(&output))
    }
}

pub fn delete_turn_refs_after(
    cwd: &Path,
    session_id: Uuid,
    retained_turn_count: usize,
    previous_turn_count: usize,
) -> anyhow::Result<()> {
    let mut commands = String::new();
    for turn_count in retained_turn_count + 1..=previous_turn_count {
        commands.push_str(&format!(
            "delete {}\ndelete {}\ndelete {}\ndelete {}\n",
            checkpoint_ref(session_id, turn_count),
            turn_start_ref(session_id, turn_count),
            turn_diff_base_ref(session_id, turn_count),
            turn_undo_backup_ref(session_id, turn_count)
        ));
    }
    update_refs(cwd, commands)
}

pub fn delete_session_refs(
    cwd: &Path,
    session_id: Uuid,
    last_turn_count: usize,
) -> anyhow::Result<()> {
    let mut commands = String::new();
    for turn_count in 0..=last_turn_count {
        commands.push_str(&format!(
            "delete {}\n",
            checkpoint_ref(session_id, turn_count)
        ));
        if turn_count > 0 {
            commands.push_str(&format!(
                "delete {}\ndelete {}\ndelete {}\n",
                turn_start_ref(session_id, turn_count),
                turn_diff_base_ref(session_id, turn_count),
                turn_undo_backup_ref(session_id, turn_count)
            ));
        }
    }
    update_refs(cwd, commands)
}

/// Delete every checkpoint ref owned by a session without requiring a client
/// to know how many turns are stored. This is the remote-safe deletion path:
/// the daemon enumerates its own repository refs as the authority.
pub fn delete_all_session_refs(cwd: &Path, session_id: Uuid) -> anyhow::Result<()> {
    let refs = session_checkpoint_ref_commits(cwd, session_id);
    let mut commands = String::new();
    for turn_count in refs.turns.keys() {
        commands.push_str(&format!(
            "delete {}\n",
            checkpoint_ref(session_id, *turn_count)
        ));
    }
    for turn_count in refs.starts.keys() {
        commands.push_str(&format!(
            "delete {}\n",
            turn_start_ref(session_id, *turn_count)
        ));
    }
    for turn_count in refs.diff_bases.keys() {
        commands.push_str(&format!(
            "delete {}\n",
            turn_diff_base_ref(session_id, *turn_count)
        ));
    }
    for turn_count in refs.undo_backups.keys() {
        commands.push_str(&format!(
            "delete {}\n",
            turn_undo_backup_ref(session_id, *turn_count)
        ));
    }
    update_refs(cwd, commands)
}

pub fn copy_session_refs(
    cwd: &Path,
    source_session_id: Uuid,
    target_session_id: Uuid,
    through_turn_count: usize,
) -> anyhow::Result<()> {
    if !is_git_repository(cwd) {
        return Ok(());
    }

    let source = session_checkpoint_ref_commits(cwd, source_session_id);
    let mut commands = String::new();
    for turn_count in 0..=through_turn_count {
        if let Some(commit) = source.turns.get(&turn_count) {
            commands.push_str(&format!(
                "update {} {commit}\n",
                checkpoint_ref(target_session_id, turn_count)
            ));
        }
        if turn_count == 0 {
            continue;
        }
        if let Some(commit) = source.starts.get(&turn_count) {
            commands.push_str(&format!(
                "update {} {commit}\n",
                turn_start_ref(target_session_id, turn_count)
            ));
        }
        if let Some(commit) = source.diff_bases.get(&turn_count) {
            commands.push_str(&format!(
                "update {} {commit}\n",
                turn_diff_base_ref(target_session_id, turn_count)
            ));
        }
    }
    update_refs(cwd, commands)
}

#[derive(Default)]
struct SessionCheckpointRefs {
    turns: HashMap<usize, String>,
    starts: HashMap<usize, String>,
    diff_bases: HashMap<usize, String>,
    undo_backups: HashMap<usize, String>,
}

/// Every checkpoint ref for `session_id`, resolved in one `git for-each-ref`.
fn session_checkpoint_ref_commits(cwd: &Path, session_id: Uuid) -> SessionCheckpointRefs {
    let prefix = format!("refs/waku/session-{session_id}-");
    git_output(
        cwd,
        [
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            &format!("{prefix}*"),
        ],
    )
    .map(|output| {
        let mut refs = SessionCheckpointRefs::default();
        for line in output.lines() {
            let Some((refname, commit)) = line.trim().split_once(' ') else {
                continue;
            };
            let Some(suffix) = refname.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let target = if let Some(turn_count) = suffix.strip_prefix("turn-start-") {
                turn_count
                    .parse()
                    .ok()
                    .map(|turn_count| (&mut refs.starts, turn_count))
            } else if let Some(turn_count) = suffix.strip_prefix("turn-diff-") {
                turn_count
                    .parse()
                    .ok()
                    .map(|turn_count| (&mut refs.diff_bases, turn_count))
            } else if let Some(turn_count) = suffix.strip_prefix("turn-undo-") {
                turn_count
                    .parse()
                    .ok()
                    .map(|turn_count| (&mut refs.undo_backups, turn_count))
            } else if let Some(turn_count) = suffix.strip_prefix("turn-") {
                turn_count
                    .parse()
                    .ok()
                    .map(|turn_count| (&mut refs.turns, turn_count))
            } else {
                None
            };
            if let Some((target, turn_count)) = target {
                target.insert(turn_count, commit.to_owned());
            }
        }
        refs
    })
    .unwrap_or_default()
}

/// Applies a batch of ref updates through one `git update-ref --stdin`.
///
/// Deleting a fifty-turn session's checkpoints was fifty `git` invocations run
/// serially on the thread that had just been asked to remove the session. The
/// batch form is one process, and it is atomic: either the whole set applies or
/// none of it does. Deleting a ref that is already gone is not an error.
///
/// Checkpoint ref names are generated, never user text, so they cannot contain
/// the space or newline this line-oriented format delimits on.
fn update_refs(cwd: &Path, commands: String) -> anyhow::Result<()> {
    if commands.is_empty() {
        return Ok(());
    }
    let mut child = crate::command_env::plain_command("git")
        .args(["update-ref", "--stdin"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to execute git")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("git update-ref stdin is unavailable"))?
        .write_all(commands.as_bytes())
        .context("failed to send ref updates to git")?;
    let output = child.wait_with_output().context("failed to execute git")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!("{}", command_error(&output))
    }
}

/// Which of a turn's changed files can go back to how they were before it.
///
/// The turn's changes run from its base — the state it started from — to its
/// end checkpoint. A file that differs between that checkpoint and the
/// worktree now was changed again afterwards, by the person or a later turn;
/// putting it back would lose that, so it is blocked. With `edited_paths`
/// given (absolute, or relative to `cwd`), a changed file the agent did not
/// edit through a tool was written by a command — a build, a formatter — and
/// is left alone.
pub fn plan_turn_undo(
    cwd: &Path,
    session_id: Uuid,
    turn_count: usize,
    edited_paths: &[String],
) -> anyhow::Result<TurnUndoPlan> {
    let (base, end) = turn_undo_range(cwd, session_id, turn_count)?;
    let current = capture_worktree_commit(cwd)?;
    let location = repository_location(cwd)?;
    plan_undo_between(cwd, &location, &base, &end, &current, edited_paths)
}

/// Put back the files [`plan_turn_undo`] finds safe, all or none. The plan is
/// worked out again first: if its safe files are not `expected_safe`, the
/// worktree moved since the person looked, and nothing is written.
///
/// Only the worktree changes: the index stays as it is and nothing is
/// cleaned. The worktree as it stood is kept under [`turn_undo_backup_ref`]
/// before anything is written.
pub fn apply_turn_undo(
    cwd: &Path,
    session_id: Uuid,
    turn_count: usize,
    edited_paths: &[String],
    expected_safe: &[String],
) -> anyhow::Result<Vec<String>> {
    let (base, end) = turn_undo_range(cwd, session_id, turn_count)?;
    let current = capture_worktree_commit(cwd)?;
    let location = repository_location(cwd)?;
    let plan = plan_undo_between(cwd, &location, &base, &end, &current, edited_paths)?;
    let mut planned = plan.safe.clone();
    planned.sort();
    let mut expected = expected_safe.to_vec();
    expected.sort();
    expected.dedup();
    if planned != expected {
        bail!("{TURN_UNDO_STALE}");
    }
    if plan.safe.is_empty() {
        return Ok(Vec::new());
    }
    git_output(
        cwd,
        [
            "update-ref",
            &turn_undo_backup_ref(session_id, turn_count),
            &current,
        ],
    )?;
    let toplevel = PathBuf::from(&location.toplevel);
    let mut written: Vec<&str> = Vec::with_capacity(plan.safe.len());
    for path in &plan.safe {
        if let Err(error) = write_path_from(cwd, &toplevel, &base, path) {
            // All or none: every file touched so far, this one included, goes
            // back to how it was a moment ago.
            for done in written.iter().copied().chain([path.as_str()]) {
                let _ = write_path_from(cwd, &toplevel, &current, done);
            }
            return Err(error.context(format!("could not put `{path}` back")));
        }
        written.push(path);
    }
    Ok(plan.safe)
}

/// The commits a turn's changes run between: where it started, and its end
/// checkpoint.
fn turn_undo_range(
    cwd: &Path,
    session_id: Uuid,
    turn_count: usize,
) -> anyhow::Result<(String, String)> {
    if turn_count == 0 || !is_git_repository(cwd) {
        bail!("undo needs the turn's checkpoints in a Git repository");
    }
    let end = resolve_ref(cwd, &checkpoint_ref(session_id, turn_count))
        .ok_or_else(|| anyhow!("the turn's checkpoint is unavailable"))?;
    let base = [
        turn_diff_base_ref(session_id, turn_count),
        turn_start_ref(session_id, turn_count),
        checkpoint_ref(session_id, turn_count - 1),
    ]
    .iter()
    .find_map(|git_ref| resolve_ref(cwd, git_ref))
    .ok_or_else(|| anyhow!("the turn's starting checkpoint is unavailable"))?;
    Ok((base, end))
}

/// Where the repository is: its top level, and `cwd` below it (`--show-prefix`,
/// empty or ending in `/`). Git names every path from the top level.
struct RepositoryLocation {
    toplevel: String,
    prefix: String,
}

fn repository_location(cwd: &Path) -> anyhow::Result<RepositoryLocation> {
    Ok(RepositoryLocation {
        toplevel: git_output(cwd, ["rev-parse", "--show-toplevel"])?
            .trim()
            .to_owned(),
        prefix: git_output(cwd, ["rev-parse", "--show-prefix"])?
            .trim()
            .to_owned(),
    })
}

fn plan_undo_between(
    cwd: &Path,
    location: &RepositoryLocation,
    base: &str,
    end: &str,
    current: &str,
    edited_paths: &[String],
) -> anyhow::Result<TurnUndoPlan> {
    let changes = changed_paths(cwd, base, end)?;
    let changed_since = changed_paths(cwd, end, current)?
        .into_iter()
        .map(|change| change.path)
        .collect::<HashSet<_>>();
    // The same file can be named two ways — a short name, a link, another
    // case — so absolute paths are compared resolved.
    let toplevel =
        canonical_text(Path::new(&location.toplevel)).unwrap_or_else(|| location.toplevel.clone());
    let edited = edited_paths
        .iter()
        .filter_map(|path| {
            let path = if Path::new(path).is_absolute() {
                canonical_text(Path::new(path)).unwrap_or_else(|| path.clone())
            } else {
                path.clone()
            };
            repo_relative_path(&path, &toplevel, &location.prefix)
        })
        .collect::<HashSet<_>>();
    // Reported paths that match nothing the turn changed were named some
    // other way than git names them. Judged by them, every change would pass
    // for a command's, so they are not used at all.
    let edited = if changes.iter().any(|change| edited.contains(&change.path)) {
        edited
    } else {
        HashSet::new()
    };
    let mut plan = TurnUndoPlan::default();
    for change in changes {
        let ignored = if change.submodule {
            Some(UndoReason::Submodule)
        } else if !edited.is_empty() && !edited.contains(&change.path) {
            Some(UndoReason::ShellWritten)
        } else {
            None
        };
        match ignored {
            Some(reason) => plan.ignored.push(UndoFile {
                path: change.path,
                reason,
            }),
            None if changed_since.contains(&change.path) => plan.blocked.push(UndoFile {
                path: change.path,
                reason: UndoReason::ChangedSinceTurn,
            }),
            None => plan.safe.push(change.path),
        }
    }
    Ok(plan)
}

struct PathChange {
    path: String,
    submodule: bool,
}

/// Every path that differs between two commits, a rename split into its two
/// sides, submodules marked.
fn changed_paths(cwd: &Path, from: &str, to: &str) -> anyhow::Result<Vec<PathChange>> {
    let output = git_output(
        cwd,
        ["diff", "--raw", "--no-renames", "-z", from, to, "--", "."],
    )?;
    let mut fields = output.split('\0');
    let mut changes = Vec::new();
    while let Some(meta) = fields.next() {
        let meta = meta.trim();
        if meta.is_empty() {
            continue;
        }
        let Some(path) = fields.next() else {
            break;
        };
        // `:<old mode> <new mode> <old object> <new object> <status>`
        let mut columns = meta.trim_start_matches(':').split(' ');
        let old_mode = columns.next().unwrap_or_default();
        let new_mode = columns.next().unwrap_or_default();
        changes.push(PathChange {
            path: path.to_owned(),
            submodule: old_mode == "160000" || new_mode == "160000",
        });
    }
    Ok(changes)
}

/// `path` with links, short names and `..` resolved, as `/`-separated text.
/// A file that no longer exists is resolved through its directory.
fn canonical_text(path: &Path) -> Option<String> {
    let resolved = fs::canonicalize(path).ok().or_else(|| {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    })?;
    let text = resolved.to_string_lossy().replace('\\', "/");
    Some(text.strip_prefix("//?/").map(str::to_owned).unwrap_or(text))
}

/// `path` as git names it: relative to the repository's top level and
/// `/`-separated. An absolute path must lie inside `toplevel`; a relative one
/// is taken from the directory `prefix` names.
fn repo_relative_path(path: &str, toplevel: &str, prefix: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    let absolute = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    let relative = if absolute {
        let toplevel = toplevel.trim_end_matches('/').replace('\\', "/");
        let head = path.get(..toplevel.len())?;
        let same = if cfg!(windows) {
            head.eq_ignore_ascii_case(&toplevel)
        } else {
            head == toplevel
        };
        let rest = path.get(toplevel.len()..)?;
        if !same || !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        rest.trim_start_matches('/').to_owned()
    } else {
        format!("{prefix}{}", path.trim_start_matches("./"))
    };
    (!relative.is_empty()).then_some(relative)
}

/// Make `path` in the worktree what it is in `commit`: its content — with the
/// line endings and filters a checkout would apply — and its mode, or gone
/// when the commit does not have it.
fn write_path_from(cwd: &Path, toplevel: &Path, commit: &str, path: &str) -> anyhow::Result<()> {
    let target = toplevel.join(path);
    let entry = git_output(cwd, ["ls-tree", "--full-tree", "-z", commit, "--", path])?;
    let entry = entry.trim_end_matches('\0');
    if entry.is_empty() {
        match fs::remove_file(&target) {
            Ok(()) => remove_empty_parents(&target, toplevel),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }
    // `<mode> <type> <object>\t<path>`
    let mode = entry.split(' ').next().unwrap_or_default();
    let content = git_bytes(cwd, ["cat-file", "--filters", &format!("{commit}:{path}")])?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    // Writing through a symlink would change what it points at.
    if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        fs::remove_file(&target)?;
    }
    #[cfg(unix)]
    if mode == "120000" {
        use std::os::unix::ffi::OsStrExt as _;
        std::os::unix::fs::symlink(OsStr::from_bytes(&content), &target)?;
        return Ok(());
    }
    fs::write(&target, &content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&target)?.permissions();
        let bits = permissions.mode();
        permissions.set_mode(if mode == "100755" {
            bits | 0o111
        } else {
            bits & !0o111
        });
        fs::set_permissions(&target, permissions)?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

/// After a file the turn created is removed, the directories it created for
/// it go too, up to the first one with anything else in it.
fn remove_empty_parents(file: &Path, toplevel: &Path) {
    let mut directory = file.parent();
    while let Some(current) = directory {
        if current == toplevel || !current.starts_with(toplevel) || fs::remove_dir(current).is_err()
        {
            break;
        }
        directory = current.parent();
    }
}

fn git_bytes<I, S>(cwd: &Path, args: I) -> anyhow::Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = crate::command_env::plain_command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .context("failed to execute git")?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        bail!("{}", command_error(&output))
    }
}

fn diff_files(cwd: &Path, from_ref: &str, to_ref: &str) -> anyhow::Result<Vec<CheckpointFile>> {
    let output = git_output(cwd, ["diff", "--numstat", from_ref, to_ref, "--", "."])?;
    let mut files = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut columns = line.splitn(3, '\t');
        let additions = columns.next().unwrap_or("0").parse().unwrap_or(0);
        let deletions = columns.next().unwrap_or("0").parse().unwrap_or(0);
        let Some(path) = columns.next() else {
            continue;
        };
        files.push(CheckpointFile {
            path: path.to_owned(),
            additions,
            deletions,
        });
    }
    Ok(files)
}

fn is_git_repository(cwd: &Path) -> bool {
    crate::command_env::plain_command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn symbolic_head(cwd: &Path) -> Option<String> {
    let output = crate::command_env::plain_command("git")
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|branch| !branch.is_empty())
}

fn repository_refs(cwd: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    let output = git_output(
        cwd,
        [
            "for-each-ref",
            "--format=%(refname)%09%(objecttype)%09%(objectname)%09%(*objecttype)%09%(*objectname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim().split('\t');
            let refname = fields.next()?;
            let object_type = fields.next()?;
            let object = fields.next()?;
            let peeled_type = fields.next().unwrap_or_default();
            let peeled = fields.next().unwrap_or_default();
            let commit = if object_type == "commit" {
                object
            } else if peeled_type == "commit" {
                peeled
            } else {
                return None;
            };
            (!refname.is_empty() && !commit.is_empty())
                .then(|| (refname.to_owned(), commit.to_owned()))
        })
        .collect())
}

fn has_head(cwd: &Path) -> bool {
    crate::command_env::plain_command("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(cwd)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn resolve_ref(cwd: &Path, git_ref: &str) -> Option<String> {
    let output = crate::command_env::plain_command("git")
        .args(["rev-parse", "--verify", &format!("{git_ref}^{{commit}}")])
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn git_output<I, S>(cwd: &Path, args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = crate::command_env::plain_command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .context("failed to execute git")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        bail!("{}", command_error(&output))
    }
}

fn git_with_index<I, S>(cwd: &Path, index: &Path, args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    git_with_environment(cwd, index, args, false)
}

fn git_with_identity_and_index<I, S>(cwd: &Path, index: &Path, args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    git_with_environment(cwd, index, args, true)
}

fn git_with_environment<I, S>(
    cwd: &Path,
    index: &Path,
    args: I,
    identity: bool,
) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = crate::command_env::plain_command("git");
    command
        .args(args)
        .current_dir(cwd)
        .env("GIT_INDEX_FILE", index);
    if identity {
        command
            .env("GIT_AUTHOR_NAME", "Waku")
            .env("GIT_AUTHOR_EMAIL", "waku@localhost")
            .env("GIT_COMMITTER_NAME", "Waku")
            .env("GIT_COMMITTER_EMAIL", "waku@localhost");
    }
    let output = command.output().context("failed to execute git")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        bail!("{}", command_error(&output))
    }
}

fn command_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        stderr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_ok(cwd: &Path, args: &[&str]) {
        let status = crate::command_env::plain_command("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn git_text(cwd: &Path, args: &[&str]) -> String {
        let output = crate::command_env::plain_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn diverged_repository() -> PathBuf {
        let directory = std::env::temp_dir().join(format!("waku-checkpoints-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        git_ok(&directory, &["init", "--quiet", "--initial-branch=main"]);
        git_ok(&directory, &["config", "user.name", "Waku Test"]);
        git_ok(&directory, &["config", "user.email", "waku@example.com"]);
        fs::write(directory.join("shared.txt"), "shared\n").unwrap();
        git_ok(&directory, &["add", "shared.txt"]);
        git_ok(&directory, &["commit", "--quiet", "-m", "baseline"]);

        git_ok(&directory, &["switch", "--quiet", "-c", "feature"]);
        fs::write(directory.join("feature-only.txt"), "feature\n").unwrap();
        fs::write(directory.join("target.txt"), "feature baseline\n").unwrap();
        git_ok(&directory, &["add", "feature-only.txt", "target.txt"]);
        git_ok(&directory, &["commit", "--quiet", "-m", "feature"]);

        git_ok(&directory, &["switch", "--quiet", "main"]);
        fs::write(directory.join("main-only.txt"), "main\n").unwrap();
        git_ok(&directory, &["add", "main-only.txt"]);
        git_ok(&directory, &["commit", "--quiet", "-m", "main"]);
        directory
    }

    #[test]
    fn session_turn_refs_lists_the_sessions_checkpoints_in_one_call() {
        let directory = std::env::temp_dir().join(format!("waku-checkpoints-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        git_ok(&directory, &["init", "--quiet"]);
        fs::write(directory.join("tracked.txt"), "baseline\n").unwrap();
        git_ok(&directory, &["add", "tracked.txt"]);
        git_ok(
            &directory,
            &[
                "-c",
                "user.name=Waku Test",
                "-c",
                "user.email=waku@example.com",
                "commit",
                "--quiet",
                "-m",
                "baseline",
            ],
        );

        let session = Uuid::new_v4();
        let other = Uuid::new_v4();
        capture_turn(&directory, session, 0).unwrap();
        capture_turn(&directory, session, 2).unwrap();
        capture_turn(&directory, other, 5).unwrap();

        assert_eq!(
            session_turn_refs(&directory, session),
            HashSet::from([0, 2])
        );
        assert_eq!(session_turn_refs(&directory, other), HashSet::from([5]));
        assert!(session_turn_refs(&directory, Uuid::new_v4()).is_empty());
        fs::remove_dir_all(&directory).ok();
    }

    /// Deletes and copies go through one batched `git update-ref --stdin`
    /// rather than a process per turn, so the semantics the loop used to give
    /// for free — gaps are skipped, an already-missing ref is not an error —
    /// are worth pinning down.
    #[test]
    fn refs_are_deleted_and_copied_in_batches() {
        let directory = std::env::temp_dir().join(format!("waku-checkpoints-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        git_ok(&directory, &["init", "--quiet"]);
        fs::write(directory.join("tracked.txt"), "baseline\n").unwrap();
        git_ok(&directory, &["add", "tracked.txt"]);
        git_ok(
            &directory,
            &[
                "-c",
                "user.name=Waku Test",
                "-c",
                "user.email=waku@example.com",
                "commit",
                "--quiet",
                "-m",
                "baseline",
            ],
        );

        let session = Uuid::new_v4();
        capture_turn(&directory, session, 0).unwrap();
        capture_turn_start(&directory, session, 1).unwrap();
        capture_turn(&directory, session, 1).unwrap();
        capture_turn(&directory, session, 3).unwrap();
        capture_ref(&directory, &turn_start_ref(session, 3)).unwrap();
        capture_ref(&directory, &turn_diff_base_ref(session, 3)).unwrap();

        // Turn 2 was never captured; the batch must tolerate the gap.
        let fork = Uuid::new_v4();
        copy_session_refs(&directory, session, fork, 3).unwrap();
        assert_eq!(
            session_turn_refs(&directory, fork),
            HashSet::from([0, 1, 3]),
            "a copy carries every ref the source actually has"
        );
        assert_eq!(
            resolve_ref(&directory, &checkpoint_ref(fork, 1)),
            resolve_ref(&directory, &checkpoint_ref(session, 1)),
            "and points at the same commit"
        );
        assert_eq!(
            resolve_ref(&directory, &turn_start_ref(fork, 1)),
            resolve_ref(&directory, &turn_start_ref(session, 1)),
            "the turn's distinct starting snapshot is copied too"
        );
        assert_eq!(
            resolve_ref(&directory, &turn_diff_base_ref(fork, 1)),
            resolve_ref(&directory, &turn_diff_base_ref(session, 1)),
            "the branch-aware review base is copied too"
        );

        delete_turn_refs_after(&directory, session, 1, 3).unwrap();
        assert_eq!(
            session_turn_refs(&directory, session),
            HashSet::from([0, 1]),
            "everything after the retained turn goes, missing ones included"
        );
        assert!(!has_ref(&directory, &turn_start_ref(session, 3)));
        assert!(!has_ref(&directory, &turn_diff_base_ref(session, 3)));

        delete_session_refs(&directory, fork, 3).unwrap();
        assert!(
            session_turn_refs(&directory, fork).is_empty(),
            "and a session's whole set goes in one call"
        );
        assert!(!has_ref(&directory, &turn_start_ref(fork, 1)));
        assert!(!has_ref(&directory, &turn_diff_base_ref(fork, 1)));

        // Nothing left to remove is a no-op, not a failure.
        delete_session_refs(&directory, fork, 3).unwrap();
        fs::remove_dir_all(&directory).ok();
    }

    /// The provider runs arbitrary commands in the repository during a turn,
    /// so the starting ref can vanish (gc, pack-refs, a rewind's cleanup)
    /// before the ending capture reads it. The capture must fall back to the
    /// previous turn's checkpoint, never fail the turn.
    #[test]
    fn a_start_ref_deleted_mid_turn_degrades_to_the_legacy_diff_base() {
        let directory = std::env::temp_dir().join(format!("waku-checkpoints-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        git_ok(&directory, &["init", "--quiet"]);
        git_ok(&directory, &["config", "core.autocrlf", "false"]);
        fs::write(directory.join("tracked.txt"), "baseline\n").unwrap();
        git_ok(&directory, &["add", "tracked.txt"]);
        git_ok(
            &directory,
            &[
                "-c",
                "user.name=Waku Test",
                "-c",
                "user.email=waku@example.com",
                "commit",
                "--quiet",
                "-m",
                "baseline",
            ],
        );

        let session = Uuid::new_v4();
        capture_turn(&directory, session, 0).unwrap();
        capture_turn_start(&directory, session, 1).unwrap();
        fs::write(directory.join("tracked.txt"), "edited\n").unwrap();
        delete_ref(&directory, &turn_start_ref(session, 1)).unwrap();

        let checkpoint = capture_turn(&directory, session, 1).unwrap();
        assert_eq!(checkpoint.status, CheckpointStatus::Ready);
        assert_eq!(
            checkpoint
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["tracked.txt"],
            "the turn's edit is still attributed via the previous checkpoint"
        );
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn captures_diffs_and_restores_tracked_and_untracked_files() {
        let directory = std::env::temp_dir().join(format!("waku-checkpoints-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        git_ok(&directory, &["init", "--quiet"]);
        git_ok(&directory, &["config", "core.autocrlf", "false"]);
        fs::write(directory.join("tracked.txt"), "baseline\n").unwrap();
        git_ok(&directory, &["add", "tracked.txt"]);
        git_ok(
            &directory,
            &[
                "-c",
                "user.name=Waku Test",
                "-c",
                "user.email=waku@example.com",
                "commit",
                "--quiet",
                "-m",
                "baseline",
            ],
        );

        let session_id = Uuid::new_v4();
        let baseline = capture_turn(&directory, session_id, 0).unwrap();
        assert_eq!(baseline.status, CheckpointStatus::Ready);

        fs::write(directory.join("tracked.txt"), "changed\n").unwrap();
        fs::write(directory.join("new.txt"), "new\n").unwrap();
        fs::write(directory.join("already-staged.txt"), "staged\n").unwrap();
        git_ok(&directory, &["add", "already-staged.txt"]);
        let turn = capture_turn(&directory, session_id, 1).unwrap();
        assert_eq!(turn.files.len(), 3);
        assert!(turn.totals_are_current());
        assert_eq!(
            turn.additions,
            turn.files.iter().map(|file| file.additions).sum::<u64>()
        );
        assert_eq!(
            turn.deletions,
            turn.files.iter().map(|file| file.deletions).sum::<u64>()
        );
        assert_eq!(
            git_text(&directory, &["diff", "--cached", "--name-only"]),
            "already-staged.txt"
        );

        let fork_session_id = Uuid::new_v4();
        copy_session_refs(&directory, session_id, fork_session_id, 1).unwrap();
        assert_eq!(
            resolve_ref(&directory, &checkpoint_ref(fork_session_id, 0)),
            resolve_ref(&directory, &checkpoint_ref(session_id, 0))
        );
        assert_eq!(
            resolve_ref(&directory, &checkpoint_ref(fork_session_id, 1)),
            resolve_ref(&directory, &checkpoint_ref(session_id, 1))
        );

        fs::write(directory.join("tracked.txt"), "later\n").unwrap();
        fs::remove_file(directory.join("new.txt")).unwrap();
        fs::write(directory.join("discard.txt"), "discard\n").unwrap();
        restore_ref(&directory, &turn.git_ref).unwrap();

        assert_eq!(
            fs::read_to_string(directory.join("tracked.txt")).unwrap(),
            "changed\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("new.txt")).unwrap(),
            "new\n"
        );
        assert!(!directory.join("discard.txt").exists());

        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn switching_to_an_existing_branch_does_not_report_its_history_as_turn_changes() {
        let directory = diverged_repository();
        let session_id = Uuid::new_v4();
        capture_turn_start(&directory, session_id, 1).unwrap();

        git_ok(&directory, &["switch", "--quiet", "feature"]);
        let turn = capture_turn(&directory, session_id, 1).unwrap();

        assert!(
            turn.files.is_empty(),
            "a branch switch alone is not a file edit: {:?}",
            turn.files
        );
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn switching_between_preexisting_detached_commits_does_not_report_their_history() {
        let directory = diverged_repository();
        let older = git_text(&directory, &["rev-parse", "main^1"]);
        let newer = git_text(&directory, &["rev-parse", "main"]);
        git_ok(&directory, &["switch", "--quiet", "--detach", &older]);
        let session_id = Uuid::new_v4();
        capture_turn_start(&directory, session_id, 1).unwrap();

        git_ok(&directory, &["switch", "--quiet", "--detach", &newer]);
        let turn = capture_turn(&directory, session_id, 1).unwrap();

        assert!(
            turn.files.is_empty(),
            "a detached checkout alone is not a file edit: {:?}",
            turn.files
        );
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn branch_switch_reports_only_changes_made_on_the_target_branch() {
        let directory = diverged_repository();
        let session_id = Uuid::new_v4();
        capture_turn_start(&directory, session_id, 1).unwrap();

        git_ok(&directory, &["switch", "--quiet", "feature"]);
        fs::write(directory.join("target.txt"), "changed during turn\n").unwrap();
        git_ok(&directory, &["add", "target.txt"]);
        git_ok(&directory, &["commit", "--quiet", "-m", "turn change"]);
        fs::write(directory.join("untracked.txt"), "new during turn\n").unwrap();
        let turn = capture_turn(&directory, session_id, 1).unwrap();

        let paths = turn
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(paths, HashSet::from(["target.txt", "untracked.txt"]));
        assert!(!paths.contains("feature-only.txt"));
        assert!(!paths.contains("main-only.txt"));
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn a_branch_created_during_the_turn_uses_its_creation_point_as_the_diff_base() {
        let directory = diverged_repository();
        let session_id = Uuid::new_v4();
        capture_turn_start(&directory, session_id, 1).unwrap();

        git_ok(&directory, &["switch", "--quiet", "-c", "new-branch"]);
        fs::write(directory.join("shared.txt"), "changed on new branch\n").unwrap();
        git_ok(&directory, &["add", "shared.txt"]);
        git_ok(
            &directory,
            &["commit", "--quiet", "-m", "new branch change"],
        );
        let turn = capture_turn(&directory, session_id, 1).unwrap();

        assert_eq!(turn.files.len(), 1);
        assert_eq!(turn.files[0].path, "shared.txt");
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn dirty_files_carried_across_a_branch_switch_remain_part_of_the_starting_state() {
        let directory = diverged_repository();
        fs::write(directory.join("shared.txt"), "already dirty\n").unwrap();
        let session_id = Uuid::new_v4();
        capture_turn_start(&directory, session_id, 1).unwrap();

        git_ok(&directory, &["switch", "--quiet", "feature"]);
        let turn = capture_turn(&directory, session_id, 1).unwrap();

        assert!(
            turn.files.is_empty(),
            "pre-turn dirty state carried by Git is not a response edit: {:?}",
            turn.files
        );
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn each_turn_uses_its_own_start_snapshot() {
        let directory = diverged_repository();
        let session_id = Uuid::new_v4();
        capture_turn_start(&directory, session_id, 1).unwrap();
        fs::write(directory.join("first-turn.txt"), "first\n").unwrap();
        let first = capture_turn(&directory, session_id, 1).unwrap();
        assert_eq!(first.files.len(), 1);

        fs::write(directory.join("between-turns.txt"), "external\n").unwrap();
        capture_turn_start(&directory, session_id, 2).unwrap();
        fs::write(directory.join("second-turn.txt"), "second\n").unwrap();
        let second = capture_turn(&directory, session_id, 2).unwrap();

        assert_eq!(second.files.len(), 1);
        assert_eq!(second.files[0].path, "second-turn.txt");
        fs::remove_dir_all(directory).ok();
    }

    /// A repository with one committed file, and turn 1 of a session that
    /// changed it and added another.
    fn repository_with_a_turn() -> (PathBuf, Uuid) {
        let directory = std::env::temp_dir().join(format!("waku-undo-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        git_ok(&directory, &["init", "--quiet"]);
        // Contents without line endings, so no end-of-line conversion the
        // machine is configured for can change them.
        fs::write(directory.join("tracked.txt"), "before").unwrap();
        git_ok(&directory, &["add", "tracked.txt"]);
        git_ok(
            &directory,
            &[
                "-c",
                "user.name=Waku Test",
                "-c",
                "user.email=waku@example.com",
                "commit",
                "--quiet",
                "-m",
                "baseline",
            ],
        );
        let session = Uuid::new_v4();
        capture_turn_start(&directory, session, 1).unwrap();
        fs::write(directory.join("tracked.txt"), "after").unwrap();
        fs::create_dir_all(directory.join("added")).unwrap();
        fs::write(directory.join("added/new.txt"), "new").unwrap();
        capture_turn(&directory, session, 1).unwrap();
        (directory, session)
    }

    fn sorted(mut paths: Vec<String>) -> Vec<String> {
        paths.sort();
        paths
    }

    #[test]
    fn undo_puts_a_turns_files_back_and_leaves_the_index_alone() {
        let (directory, session) = repository_with_a_turn();

        let plan = plan_turn_undo(&directory, session, 1, &[]).unwrap();
        assert_eq!(sorted(plan.safe.clone()), ["added/new.txt", "tracked.txt"]);
        assert!(plan.blocked.is_empty() && plan.ignored.is_empty());

        let restored = apply_turn_undo(&directory, session, 1, &[], &plan.safe).unwrap();
        assert_eq!(sorted(restored), ["added/new.txt", "tracked.txt"]);
        assert_eq!(
            fs::read_to_string(directory.join("tracked.txt")).unwrap(),
            "before"
        );
        assert!(!directory.join("added").exists());
        assert!(git_text(&directory, &["diff", "--cached", "--name-only"]).is_empty());
        assert!(has_ref(&directory, &turn_undo_backup_ref(session, 1)));
    }

    #[test]
    fn a_file_changed_after_the_turn_is_not_put_back() {
        let (directory, session) = repository_with_a_turn();
        fs::write(directory.join("tracked.txt"), "later").unwrap();

        let plan = plan_turn_undo(&directory, session, 1, &[]).unwrap();
        assert_eq!(plan.safe, ["added/new.txt"]);
        assert_eq!(
            plan.blocked,
            [UndoFile {
                path: "tracked.txt".into(),
                reason: UndoReason::ChangedSinceTurn,
            }]
        );

        apply_turn_undo(&directory, session, 1, &[], &plan.safe).unwrap();
        assert_eq!(
            fs::read_to_string(directory.join("tracked.txt")).unwrap(),
            "later"
        );
        assert!(!directory.join("added/new.txt").exists());
    }

    #[test]
    fn files_a_command_wrote_are_left_alone() {
        let (directory, session) = repository_with_a_turn();
        let edited = [directory.join("tracked.txt").to_string_lossy().into_owned()];

        let plan = plan_turn_undo(&directory, session, 1, &edited).unwrap();
        assert_eq!(plan.safe, ["tracked.txt"]);
        assert_eq!(
            plan.ignored,
            [UndoFile {
                path: "added/new.txt".into(),
                reason: UndoReason::ShellWritten,
            }]
        );
    }

    #[test]
    fn a_plan_the_files_moved_away_from_writes_nothing() {
        let (directory, session) = repository_with_a_turn();
        let plan = plan_turn_undo(&directory, session, 1, &[]).unwrap();
        fs::write(directory.join("tracked.txt"), "later").unwrap();

        let error = apply_turn_undo(&directory, session, 1, &[], &plan.safe).unwrap_err();
        assert!(error.to_string().contains(TURN_UNDO_STALE));
        assert!(directory.join("added/new.txt").exists());
        assert!(!has_ref(&directory, &turn_undo_backup_ref(session, 1)));
    }

    #[test]
    fn a_rewind_clears_the_undo_backup_with_the_turns_other_refs() {
        let (directory, session) = repository_with_a_turn();
        let plan = plan_turn_undo(&directory, session, 1, &[]).unwrap();
        apply_turn_undo(&directory, session, 1, &[], &plan.safe).unwrap();

        delete_turn_refs_after(&directory, session, 0, 1).unwrap();
        assert!(!has_ref(&directory, &turn_undo_backup_ref(session, 1)));
    }

    #[test]
    fn reported_paths_are_named_the_way_git_names_them() {
        assert_eq!(
            repo_relative_path("/repo/src/a.rs", "/repo", "").as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(
            repo_relative_path("./src/a.rs", "/repo", "app/").as_deref(),
            Some("app/src/a.rs")
        );
        assert_eq!(repo_relative_path("/other/a.rs", "/repo", ""), None);
        assert_eq!(repo_relative_path("/repository/a.rs", "/repo", ""), None);
        assert_eq!(
            repo_relative_path("C:\\Repo\\src\\a.rs", "C:/Repo", "").as_deref(),
            Some("src/a.rs")
        );
        if cfg!(windows) {
            assert_eq!(
                repo_relative_path("c:\\repo\\a.rs", "C:/Repo", "").as_deref(),
                Some("a.rs")
            );
        }
    }
}
