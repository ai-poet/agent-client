//! Undoing the file changes of one turn, from its changed-files card.
//!
//! Fork addition. The only way back used to be "rewind to here", which
//! restores the whole worktree to a checkpoint (`git clean` included) and
//! sends the prompt again. Undo is narrower: it puts back just the files the
//! turn changed, skips any changed again since, and leaves the conversation
//! alone. The daemon does the Git work (`checkpoint::plan_turn_undo` and
//! `apply_turn_undo`); this side asks, shows the plan, and records the result.

use super::confirm_dialog::ConfirmSection;
use super::*;

/// What an undo needs to reach the daemon, taken while the session is at hand.
struct TurnUndoTarget {
    session_id: Uuid,
    turn_count: usize,
    cwd: PathBuf,
    edited_paths: Vec<String>,
}

/// The files the agent edited through its tools in `turn_id`, as it named
/// them. The daemon resolves them against the repository.
pub(super) fn turn_edited_paths(session: &AgentSession, turn_id: Uuid) -> Vec<String> {
    let mut paths = session
        .transcript_blocks
        .iter()
        .filter(|block| block.turn_id == Some(turn_id))
        .flat_map(|block| &block.activities)
        .filter(|activity| activity.kind == ActivityKind::FileChange && !activity.failed)
        .flat_map(|activity| &activity.file_changes)
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

impl Waku {
    fn turn_undo_target(
        &mut self,
        turn_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Option<TurnUndoTarget> {
        let session = self.selected_session()?;
        if session.is_busy() || self.turn_undo_pending.contains(&turn_id) {
            self.show_toast(tr!("changes.undo_busy"));
            cx.notify();
            return None;
        }
        let turn = session.turns.iter().find(|turn| turn.id == turn_id)?;
        let Some(cwd) = self
            .workspace_path_for_session(session)
            .map(Path::to_path_buf)
        else {
            self.show_toast(tr!("errors.task_project_not_found"));
            cx.notify();
            return None;
        };
        Some(TurnUndoTarget {
            session_id: session.id,
            turn_count: turn.turn_count,
            cwd,
            edited_paths: turn_edited_paths(session, turn_id),
        })
    }

    /// Work out what undoing the turn would do, then ask.
    pub(super) fn start_turn_undo(&mut self, turn_id: Uuid, cx: &mut Context<Self>) {
        let Some(target) = self.turn_undo_target(turn_id, cx) else {
            return;
        };
        self.turn_undo_pending.insert(turn_id);
        cx.notify();
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        cx.spawn(async move |waku, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match workspace.request(waku_client::WorkspaceOperation::PlanTurnUndo {
                        cwd: target.cwd,
                        session_id: target.session_id,
                        turn_count: target.turn_count,
                        edited_paths: target.edited_paths,
                    })? {
                        waku_client::WorkspaceResult::TurnUndoPlan { plan } => Ok(plan),
                        _ => anyhow::bail!("the daemon returned an invalid undo response"),
                    }
                })
                .await;
            let _ = waku.update(cx, |waku, cx| {
                waku.turn_undo_pending.remove(&turn_id);
                match result {
                    Ok(plan) => waku.confirm_turn_undo(turn_id, plan, cx),
                    Err(error) => {
                        waku.show_toast(tr!("changes.undo_failed", error = error.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_turn_undo(
        &mut self,
        turn_id: Uuid,
        plan: waku_client::TurnUndoPlan,
        cx: &mut Context<Self>,
    ) {
        if plan.safe.is_empty() {
            self.show_toast(tr!("changes.undo_nothing"));
            return;
        }
        let reason = |reason: waku_client::UndoReason| match reason {
            waku_client::UndoReason::ChangedSinceTurn => tr!("changes.reason_changed"),
            waku_client::UndoReason::ShellWritten => tr!("changes.reason_shell"),
            waku_client::UndoReason::Submodule => tr!("changes.reason_submodule"),
        };
        let mut sections = vec![ConfirmSection {
            title: tr!("changes.undo_safe", count = plan.safe.len()),
            items: plan.safe.clone(),
            muted: false,
        }];
        for (key, files) in [
            ("changes.undo_unsafe", &plan.blocked),
            ("changes.undo_ignored", &plan.ignored),
        ] {
            if !files.is_empty() {
                sections.push(ConfirmSection {
                    title: tr!(key, count = files.len()),
                    items: files
                        .iter()
                        .map(|file| format!("{} · {}", file.path, reason(file.reason)))
                        .collect(),
                    muted: true,
                });
            }
        }
        let count = plan.safe.len();
        let confirm_label = if count == 1 {
            tr!("changes.undo_confirm_one", count = count)
        } else {
            tr!("changes.undo_confirm_other", count = count)
        };
        let expected_safe = plan.safe;
        self.request_confirm_with_sections(
            tr!("changes.undo_title"),
            Some(tr!("changes.undo_detail")),
            sections,
            confirm_label,
            true,
            cx,
            move |this, _, cx| this.apply_turn_undo(turn_id, expected_safe, cx),
        );
    }

    fn apply_turn_undo(
        &mut self,
        turn_id: Uuid,
        expected_safe: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.turn_undo_target(turn_id, cx) else {
            return;
        };
        let session_id = target.session_id;
        self.turn_undo_pending.insert(turn_id);
        cx.notify();
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        cx.spawn(async move |waku, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match workspace.request(waku_client::WorkspaceOperation::ApplyTurnUndo {
                        cwd: target.cwd,
                        session_id: target.session_id,
                        turn_count: target.turn_count,
                        edited_paths: target.edited_paths,
                        expected_safe,
                    })? {
                        waku_client::WorkspaceResult::TurnUndone { restored } => Ok(restored),
                        _ => anyhow::bail!("the daemon returned an invalid undo response"),
                    }
                })
                .await;
            let _ = waku.update(cx, |waku, cx| {
                waku.turn_undo_pending.remove(&turn_id);
                match result {
                    Ok(restored) => waku.finish_turn_undo(session_id, turn_id, restored.len()),
                    Err(error) => {
                        let error = error.to_string();
                        waku.show_toast(if error.contains(waku_client::TURN_UNDO_STALE) {
                            tr!("changes.undo_stale")
                        } else {
                            tr!("changes.undo_failed", error = error)
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_turn_undo(&mut self, session_id: Uuid, turn_id: Uuid, restored: usize) {
        let recorded = self
            .state
            .session_mut(session_id)
            .and_then(|session| session.turns.iter_mut().find(|turn| turn.id == turn_id))
            .map(|turn| turn.undone_at = Some(unix_time()))
            .is_some();
        if recorded {
            self.state.mark_session_dirty(session_id);
        }
        // The files moved under the checkpoints and the workspace views.
        self.invalidate_checkpoint_refs();
        if self.state.selected_session == Some(session_id) {
            self.workspace_queries_stale = true;
            self.remeasure_changed_files(turn_id);
        }
        self.show_toast(tr!("changes.undone_count", count = restored));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActivityFileChange, TranscriptBlock};

    fn edit(path: &str, failed: bool) -> ActivityItem {
        let mut activity = ActivityItem::new(None, ActivityKind::FileChange, "Edit", None, true);
        activity.failed = failed;
        activity.file_changes = vec![ActivityFileChange {
            path: path.into(),
            additions: Some(1),
            deletions: Some(0),
            status: None,
            diff: None,
        }];
        activity
    }

    #[test]
    fn the_edited_paths_are_the_turns_successful_tool_edits() {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Native);
        let turn_id = session.begin_turn("Change it");
        let other_turn = Uuid::new_v4();
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 1,
            turn_id: Some(turn_id),
            activities: vec![
                edit("src/b.rs", false),
                edit("src/a.rs", false),
                edit("src/c.rs", true),
            ],
        });
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 1,
            turn_id: Some(other_turn),
            activities: vec![edit("src/d.rs", false)],
        });
        session.transcript_blocks.push(TranscriptBlock {
            after_message: 1,
            turn_id: Some(turn_id),
            activities: vec![edit("src/a.rs", false)],
        });

        assert_eq!(
            turn_edited_paths(&session, turn_id),
            ["src/a.rs", "src/b.rs"]
        );
    }
}
