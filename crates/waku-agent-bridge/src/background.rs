//! Work that outlives the turn that started it.
//!
//! The engine keeps one process-global registry of background work: shell
//! commands run with `run_in_background`, and sub-agents spawned with
//! `run_in_background`. Waku's background-work panel wants a level signal —
//! "here is everything still live" — rather than edge events, so this module
//! snapshots that registry on demand and after every turn.
//!
//! Stopping is the one place the registry is not enough. `TaskRegistry::cancel`
//! signals a sub-agent's token, which does stop it, but a background shell has
//! no token — only a pid, held by a detached task that will not drop the child
//! until it exits on its own. So stopping a shell means signalling the pid
//! ourselves, and reporting honestly when that could not be done.

use std::process::Command;

use claurst_core::tasks::{BackgroundTask, TaskStatus, global_registry};

/// What kind of work an entry is. Decided from the name the engine gives it,
/// which is the only classification the registry carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundKind {
    Process,
    Subagent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

/// One entry, in this crate's vocabulary. `waku-core` maps it onto its own
/// `BackgroundWorkItem`.
#[derive(Clone, Debug)]
pub struct BackgroundEntry {
    pub id: String,
    pub kind: BackgroundKind,
    pub title: String,
    pub status: BackgroundStatus,
    pub detail: Option<String>,
    pub pid: Option<u32>,
    /// The tail of what the work has printed so far.
    pub output: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

/// How much output a snapshot carries per entry. The panel shows a tail; the
/// full log is one `TaskOutput` tool call away for the model and not
/// something to ship through the event pump on every refresh.
const OUTPUT_TAIL_LINES: usize = 200;

pub fn snapshot() -> Vec<BackgroundEntry> {
    global_registry().list().iter().map(entry).collect()
}

fn entry(task: &BackgroundTask) -> BackgroundEntry {
    let (kind, title) = classify(&task.name);
    let (status, detail) = match &task.status {
        TaskStatus::Running => (BackgroundStatus::Running, None),
        TaskStatus::Completed => (BackgroundStatus::Completed, None),
        TaskStatus::Failed(reason) => (BackgroundStatus::Failed, Some(reason.clone())),
        TaskStatus::Cancelled => (BackgroundStatus::Stopped, None),
    };
    let output = (!task.output.is_empty()).then(|| {
        let start = task.output.len().saturating_sub(OUTPUT_TAIL_LINES);
        task.output[start..].join("\n")
    });
    BackgroundEntry {
        id: task.id.clone(),
        kind,
        title,
        status,
        detail,
        pid: task.pid,
        output,
        started_at_ms: task.started_at.timestamp_millis().max(0) as u64,
        finished_at_ms: task
            .completed_at
            .map(|at| at.timestamp_millis().max(0) as u64),
    }
}

/// The engine names background shells `bg: <command>` and background
/// sub-agents `subagent: <description>`. Strip the tag for the title and use
/// it for the kind.
fn classify(name: &str) -> (BackgroundKind, String) {
    if let Some(rest) = name.strip_prefix("subagent:") {
        return (BackgroundKind::Subagent, rest.trim().to_owned());
    }
    if let Some(rest) = name.strip_prefix("bg:") {
        return (BackgroundKind::Process, rest.trim().to_owned());
    }
    (BackgroundKind::Process, name.trim().to_owned())
}

/// Stop one entry. `Err` carries a reason the user can read.
pub fn stop(id: &str) -> Result<(), String> {
    let Some(task) = global_registry().get(id) else {
        return Err("this work is no longer tracked".to_owned());
    };
    if !task.is_running() {
        return Err(format!("already {}", task.status));
    }

    // Sub-agents (and anything else with a token) stop through the registry,
    // which both signals the loop and relabels the entry.
    if task.cancel_token.is_some() {
        global_registry().cancel(id);
        return Ok(());
    }

    // A background shell: only the pid can reach it.
    let Some(pid) = task.pid else {
        return Err("the process has not reported a pid yet".to_owned());
    };
    signal(pid)?;
    global_registry().cancel(id);
    Ok(())
}

#[cfg(unix)]
fn signal(pid: u32) -> Result<(), String> {
    // TERM first; the shell's own children get it through the process group
    // when `bash -c` ran them in one. Escalating to KILL is left to the user
    // pressing stop again after the panel shows it still running.
    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|error| format!("could not run kill: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kill exited with {status}"))
    }
}

#[cfg(windows)]
fn signal(pid: u32) -> Result<(), String> {
    // `cmd /C` started the command, so the work is the tree under that pid,
    // not the pid itself — `/T` is what actually stops it.
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map_err(|error| format!("could not run taskkill: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("taskkill exited with {status}"))
    }
}

#[cfg(not(any(unix, windows)))]
fn signal(_pid: u32) -> Result<(), String> {
    Err("stopping a background process is not supported on this platform".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engines_name_tags_decide_the_kind() {
        assert_eq!(
            classify("subagent: review the diff"),
            (BackgroundKind::Subagent, "review the diff".to_owned())
        );
        assert_eq!(
            classify("bg: cargo test"),
            (BackgroundKind::Process, "cargo test".to_owned())
        );
        assert_eq!(
            classify("something else"),
            (BackgroundKind::Process, "something else".to_owned())
        );
    }

    #[test]
    fn a_failed_task_keeps_its_reason_as_the_detail() {
        let mut task = BackgroundTask::new("bg: make");
        task.status = TaskStatus::Failed("exit code 2".into());
        let entry = entry(&task);
        assert_eq!(entry.status, BackgroundStatus::Failed);
        assert_eq!(entry.detail.as_deref(), Some("exit code 2"));
    }

    #[test]
    fn only_the_tail_of_a_long_log_travels_with_the_snapshot() {
        let mut task = BackgroundTask::new("bg: noisy");
        task.output = (0..500).map(|line| line.to_string()).collect();
        let entry = entry(&task);
        let output = entry.output.unwrap();
        assert!(output.starts_with("300\n"), "{output}");
        assert!(output.ends_with("499"));
    }

    #[test]
    fn stopping_untracked_work_says_so() {
        assert!(stop("no-such-task").is_err());
    }
}
