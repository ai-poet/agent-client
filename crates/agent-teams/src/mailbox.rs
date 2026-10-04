// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Per-agent JSONL mailboxes (`<team>/inbox/<agentKey>.jsonl`).
//!
//! A message moves through three marks: a short delivery lease while one
//! path tries to hand it over, `deliveredAt` once the recipient's live inbox
//! took it, and `readAt` once the recipient consumed or was shown it.
//! Messages scoped to an execution generation that is no longer current are
//! discarded rather than delivered — a member that was reassigned must not
//! act on guidance meant for its old attempt.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use anyhow::Context as _;

use crate::key::{CAPTAIN_KEY, sanitize_key};
use crate::store::{StateRoot, atomic_write_text};
use crate::types::{TeamMessage, TeamState, now_ms};
use crate::validate::{parse_message_line, strip_bom};

/// A crashed live-delivery attempt becomes retryable after this interval.
pub const DELIVERY_LEASE_MS: u64 = 60_000;

/// A line that could not be read as a message, kept on disk for diagnosis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalformedLine {
    /// 1-based.
    pub line: usize,
    pub reason: &'static str,
}

pub fn mailbox_file(root: &StateRoot, team_id: &str, agent: &str) -> PathBuf {
    root.inbox_dir(team_id)
        .join(format!("{}.jsonl", sanitize_key(agent)))
}

fn read_text(root: &StateRoot, team_id: &str, agent: &str) -> anyhow::Result<Option<String>> {
    let path = mailbox_file(root, team_id, agent);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Append one message to an agent's mailbox.
pub fn append(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    message: &TeamMessage,
) -> anyhow::Result<()> {
    let existing = read_text(root, team_id, agent)?.unwrap_or_default();
    let separator = if !existing.is_empty() && !existing.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let line = serde_json::to_string(message)?;
    atomic_write_text(
        &mailbox_file(root, team_id, agent),
        &format!("{existing}{separator}{line}\n"),
    )?;
    Ok(())
}

/// One agent's whole mailbox, oldest first, plus the lines that were
/// skipped. A missing mailbox is empty.
pub fn read_with_warnings(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
) -> anyhow::Result<(Vec<TeamMessage>, Vec<MalformedLine>)> {
    let Some(text) = read_text(root, team_id, agent)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut messages = Vec::new();
    let mut malformed = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let line = strip_bom(raw).trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if serde_json::from_str::<serde_json::Value>(line).is_err() {
            malformed.push(MalformedLine {
                line: index + 1,
                reason: "invalid JSON",
            });
            continue;
        }
        match parse_message_line(line) {
            Some(message) => messages.push(message),
            None => malformed.push(MalformedLine {
                line: index + 1,
                reason: "invalid message shape",
            }),
        }
    }
    Ok((messages, malformed))
}

pub fn read(root: &StateRoot, team_id: &str, agent: &str) -> anyhow::Result<Vec<TeamMessage>> {
    Ok(read_with_warnings(root, team_id, agent)?.0)
}

/// Messages the recipient has not acknowledged.
pub fn read_unread(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
) -> anyhow::Result<Vec<TeamMessage>> {
    Ok(read(root, team_id, agent)?
        .into_iter()
        .filter(TeamMessage::is_unread)
        .collect())
}

/// Unread, undelivered, and not leased by a delivery still in its window.
pub fn read_pending(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    now: u64,
) -> anyhow::Result<Vec<TeamMessage>> {
    Ok(read_unread(root, team_id, agent)?
        .into_iter()
        .filter(|message| {
            message.delivered_at.is_none()
                && message
                    .delivery_claimed_at
                    .is_none_or(|claimed| now.saturating_sub(claimed) >= DELIVERY_LEASE_MS)
        })
        .collect())
}

/// Rewrite the selected records in place, leaving malformed lines untouched
/// for diagnosis.
fn mutate(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    ids: &[String],
    change: impl Fn(&mut TeamMessage),
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let Some(text) = read_text(root, team_id, agent)? else {
        return Ok(());
    };
    let selected: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let lines: Vec<String> = text
        .split('\n')
        .map(|raw| {
            let line = strip_bom(raw).trim_end_matches('\r');
            if line.trim().is_empty() {
                return raw.to_owned();
            }
            match parse_message_line(line) {
                Some(mut message) if selected.contains(message.id.as_str()) => {
                    change(&mut message);
                    serde_json::to_string(&message).unwrap_or_else(|_| raw.to_owned())
                }
                _ => raw.to_owned(),
            }
        })
        .collect();
    atomic_write_text(&mailbox_file(root, team_id, agent), &lines.join("\n"))?;
    Ok(())
}

/// Lease the selected messages to one delivery path.
pub fn claim_delivery(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    ids: &[String],
) -> anyhow::Result<()> {
    let now = now_ms();
    mutate(root, team_id, agent, ids, |message| {
        message.delivery_claimed_at = Some(now);
    })
}

/// Release a failed delivery lease so the scheduler can retry later.
pub fn release_delivery(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    ids: &[String],
) -> anyhow::Result<()> {
    mutate(root, team_id, agent, ids, |message| {
        message.delivery_claimed_at = None;
    })
}

/// Mark the selected messages delivered and read.
pub fn acknowledge(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    ids: &[String],
) -> anyhow::Result<()> {
    let now = now_ms();
    mutate(root, team_id, agent, ids, |message| {
        message.delivery_claimed_at = None;
        message.delivered_at.get_or_insert(now);
        message.read_at.get_or_insert(now);
    })
}

/// Accepted by the recipient's live inbox — not yet evidence it was read.
pub fn mark_delivered(
    root: &StateRoot,
    team_id: &str,
    agent: &str,
    ids: &[String],
) -> anyhow::Result<()> {
    let now = now_ms();
    mutate(root, team_id, agent, ids, |message| {
        message.delivery_claimed_at = None;
        message.delivered_at.get_or_insert(now);
    })
}

/// Keep the records for audit, but never deliver them.
pub fn discard(root: &StateRoot, team_id: &str, agent: &str, ids: &[String]) -> anyhow::Result<()> {
    let now = now_ms();
    mutate(root, team_id, agent, ids, |message| {
        message.discarded_at.get_or_insert(now);
    })
}

/// Whether a message still belongs to a live execution generation: its
/// source attempt (if any) is still the sender's, and its recipient attempt
/// (if any) is still claimed or in progress.
pub fn is_current_mail(team: &TeamState, message: &TeamMessage) -> bool {
    if message.discarded_at.is_some() {
        return false;
    }
    if let Some(source_attempt) = &message.source_attempt_id {
        let source_live = team.tasks.iter().any(|task| {
            Some(&task.id) == message.source_task_id.as_ref()
                && task.attempt_id.as_ref() == Some(source_attempt)
                && task.assignee.as_deref() == Some(message.from.as_str())
                && (message.source_task_status.is_none()
                    || !task.status.is_terminal()
                    || message.source_task_status == Some(task.status))
        });
        if !source_live {
            return false;
        }
    }
    let Some(attempt) = &message.attempt_id else {
        return true;
    };
    team.tasks.iter().any(|task| {
        Some(&task.id) == message.task_id.as_ref()
            && task.attempt_id.as_ref() == Some(attempt)
            && task.assignee.as_deref() == Some(message.to.as_str())
            && task.status.is_open_attempt()
    })
}

/// The message text with its structured provenance.
pub fn mailbox_content(message: &TeamMessage) -> String {
    let provenance = match &message.source_task_id {
        None => String::new(),
        Some(task) => format!(
            "[Source task {task}, attempt_id {}{}]\n",
            message.source_attempt_id.as_deref().unwrap_or("undefined"),
            message
                .source_task_status
                .map(|status| format!(", status={status}"))
                .unwrap_or_default()
        ),
    };
    format!("{provenance}{}", message.content)
}

/// What a recipient reads for a batch of messages.
pub fn mailbox_prompt(recipient: &str, messages: &[TeamMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            if recipient == CAPTAIN_KEY {
                format!(
                    "AgentTeams message from member {}:\n\n{}",
                    message.from,
                    mailbox_content(message)
                )
            } else {
                let scope = match &message.attempt_id {
                    None => String::new(),
                    Some(attempt) => format!(
                        " for task {}, attempt_id {attempt}",
                        message.task_id.as_deref().unwrap_or("undefined")
                    ),
                };
                format!(
                    "AgentTeams message from {}{scope}:\n\n{}",
                    message.from,
                    mailbox_content(message)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Unread mail that is still current; obsolete mail is discarded on the way.
/// The caller holds the team lock.
pub fn read_current(
    root: &StateRoot,
    team: &TeamState,
    recipient: &str,
) -> anyhow::Result<Vec<TeamMessage>> {
    let unread = read_unread(root, &team.id, recipient)?;
    let (current, stale): (Vec<_>, Vec<_>) = unread
        .into_iter()
        .partition(|message| is_current_mail(team, message));
    let stale_ids: Vec<String> = stale.into_iter().map(|message| message.id).collect();
    discard(root, &team.id, recipient, &stale_ids)?;
    Ok(current)
}

/// Pending (deliverable) mail that is still current; obsolete pending mail
/// is discarded. The caller holds the team lock.
pub fn read_current_pending(
    root: &StateRoot,
    team: &TeamState,
    recipient: &str,
    now: u64,
) -> anyhow::Result<Vec<TeamMessage>> {
    let pending = read_pending(root, &team.id, recipient, now)?;
    let (current, stale): (Vec<_>, Vec<_>) = pending
        .into_iter()
        .partition(|message| is_current_mail(team, message));
    let stale_ids: Vec<String> = stale.into_iter().map(|message| message.id).collect();
    discard(root, &team.id, recipient, &stale_ids)?;
    Ok(current)
}

pub fn ids_of(messages: &[TeamMessage]) -> Vec<String> {
    messages.iter().map(|message| message.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TaskStatus, TeamTask};

    fn root() -> (tempfile::TempDir, StateRoot) {
        let dir = tempfile::tempdir().unwrap();
        let root = StateRoot::new(dir.path(), ".agent-teams");
        std::fs::create_dir_all(root.inbox_dir("alpha")).unwrap();
        (dir, root)
    }

    #[test]
    fn append_read_and_marks() {
        let (_dir, root) = root();
        let first = TeamMessage::new("dev", CAPTAIN_KEY, "one");
        let second = TeamMessage::new("dev", CAPTAIN_KEY, "two");
        append(&root, "alpha", CAPTAIN_KEY, &first).unwrap();
        append(&root, "alpha", CAPTAIN_KEY, &second).unwrap();
        assert_eq!(read(&root, "alpha", CAPTAIN_KEY).unwrap().len(), 2);

        claim_delivery(&root, "alpha", CAPTAIN_KEY, std::slice::from_ref(&first.id)).unwrap();
        let pending = read_pending(&root, "alpha", CAPTAIN_KEY, now_ms()).unwrap();
        assert_eq!(ids_of(&pending), vec![second.id.clone()]);
        // The lease lapses.
        let later = now_ms() + DELIVERY_LEASE_MS;
        assert_eq!(
            read_pending(&root, "alpha", CAPTAIN_KEY, later)
                .unwrap()
                .len(),
            2
        );

        release_delivery(&root, "alpha", CAPTAIN_KEY, std::slice::from_ref(&first.id)).unwrap();
        mark_delivered(&root, "alpha", CAPTAIN_KEY, std::slice::from_ref(&first.id)).unwrap();
        assert_eq!(
            read_pending(&root, "alpha", CAPTAIN_KEY, now_ms())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(read_unread(&root, "alpha", CAPTAIN_KEY).unwrap().len(), 2);

        acknowledge(&root, "alpha", CAPTAIN_KEY, std::slice::from_ref(&first.id)).unwrap();
        discard(
            &root,
            "alpha",
            CAPTAIN_KEY,
            std::slice::from_ref(&second.id),
        )
        .unwrap();
        assert!(read_unread(&root, "alpha", CAPTAIN_KEY).unwrap().is_empty());
    }

    #[test]
    fn malformed_lines_are_skipped_and_preserved() {
        let (_dir, root) = root();
        let good = TeamMessage::new("dev", CAPTAIN_KEY, "ok");
        let path = mailbox_file(&root, "alpha", CAPTAIN_KEY);
        std::fs::write(
            &path,
            format!(
                "{{broken\n{{}}\n{}\n",
                serde_json::to_string(&good).unwrap()
            ),
        )
        .unwrap();
        let (messages, warnings) = read_with_warnings(&root, "alpha", CAPTAIN_KEY).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(warnings.len(), 2);
        acknowledge(&root, "alpha", CAPTAIN_KEY, &[good.id]).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("{broken\n{}\n")
        );
    }

    fn team_with_attempt() -> TeamState {
        let mut team = TeamState::new("alpha", "alpha", "cap", 1);
        let mut task = TeamTask::new("t1", "do", 1);
        task.status = TaskStatus::InProgress;
        task.assignee = Some("dev".into());
        task.attempt_id = Some("a1".into());
        team.tasks.push(task);
        team
    }

    #[test]
    fn mail_for_a_superseded_attempt_is_not_current() {
        let mut team = team_with_attempt();
        let mut message = TeamMessage::new(CAPTAIN_KEY, "dev", "keep going");
        message.task_id = Some("t1".into());
        message.attempt_id = Some("a1".into());
        assert!(is_current_mail(&team, &message));
        team.tasks[0].attempt_id = Some("a2".into());
        assert!(!is_current_mail(&team, &message));
    }

    #[test]
    fn a_report_from_a_revoked_source_attempt_is_not_current() {
        let mut team = team_with_attempt();
        let mut report = TeamMessage::new("dev", CAPTAIN_KEY, "done");
        report.source_task_id = Some("t1".into());
        report.source_attempt_id = Some("a1".into());
        report.source_task_status = Some(TaskStatus::Completed);
        assert!(is_current_mail(&team, &report));
        team.tasks[0].status = TaskStatus::Completed;
        assert!(is_current_mail(&team, &report));
        team.tasks[0].status = TaskStatus::Failed;
        assert!(!is_current_mail(&team, &report));
        team.tasks[0].status = TaskStatus::InProgress;
        team.tasks[0].attempt_id = None;
        assert!(!is_current_mail(&team, &report));
    }

    #[test]
    fn prompts_name_the_sender_and_scope() {
        let mut message = TeamMessage::new("dev", CAPTAIN_KEY, "done");
        message.source_task_id = Some("t1".into());
        message.source_attempt_id = Some("a1".into());
        assert_eq!(
            mailbox_prompt(CAPTAIN_KEY, std::slice::from_ref(&message)),
            "AgentTeams message from member dev:\n\n[Source task t1, attempt_id a1]\ndone"
        );
        let mut guidance = TeamMessage::new(CAPTAIN_KEY, "dev", "use X");
        guidance.task_id = Some("t1".into());
        guidance.attempt_id = Some("a1".into());
        assert_eq!(
            mailbox_prompt("dev", &[guidance]),
            "AgentTeams message from captain for task t1, attempt_id a1:\n\nuse X"
        );
    }
}
