// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The event-driven scheduler: kicks, dispatch, member and captain edges,
//! and the final failure of a member turn.

use crate::key::CAPTAIN_KEY;
use crate::mailbox;
use crate::scheduler::{DispatchDecision, assignment_prompt, plan_dispatch, rollback_dispatch};
use crate::transitions::invalidate_task_attempt;
use crate::types::{MemberStatus, TaskStatus, TeamMessage, TeamState, now_ms};

use super::{DeliveryMode, MemberActivity, OpResult, TeamRuntime, global_locks, text};

/// The member's failed turn, as the host observed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedAttempt {
    pub task_id: String,
    pub attempt: Option<u64>,
    pub attempt_id: Option<String>,
}

impl TeamRuntime {
    fn member_available(&self, member: &crate::types::TeamMember) -> bool {
        if member.is_stopping() {
            return false;
        }
        !member.is_spawned() || self.host.member_activity(&member.id) != MemberActivity::Running
    }

    /// Try to give every genuinely idle member one unit of ready work.
    pub fn kick_team(&self, team_id: &str) {
        let Ok(Some(team)) = self.root.read_team(team_id) else {
            return;
        };
        if team.is_halted() || team.is_staged() || team.captain_session_id != self.captain_id() {
            return;
        }
        for member in team.members.iter().filter(|member| !member.is_removed()) {
            self.kick_member(team_id, &member.name);
        }
    }

    /// Flush pending mail, or give one member one ready task. Serialized per
    /// member.
    pub fn kick_member(&self, team_id: &str, member_name: &str) {
        let key = self.member_lock_key(team_id, member_name);
        global_locks().with(&key, || {
            if let Err(error) = self.kick_member_locked(team_id, member_name) {
                tracing_warn(&format!(
                    "agent-teams: kick of {member_name} failed: {error}"
                ));
            }
        });
    }

    fn kick_member_locked(&self, team_id: &str, member_name: &str) -> OpResult<()> {
        let Some(team) = self.root.read_team(team_id).map_err(text)? else {
            return Ok(());
        };
        if team.is_halted() || team.is_staged() || team.captain_session_id != self.captain_id() {
            return Ok(());
        }
        let Some(member) = team
            .members
            .iter()
            .find(|member| member.name == member_name && !member.is_removed())
        else {
            return Ok(());
        };
        if !self.member_available(member) {
            return Ok(());
        }

        // Pending mail is real work: deliver it before a fresh task, and mark
        // it delivered only once the member took it.
        let unread = self.with_team_lock(team_id, || -> OpResult<Vec<TeamMessage>> {
            let Some(fresh) = self.root.read_team(team_id).map_err(text)? else {
                return Ok(Vec::new());
            };
            let current = mailbox::read_current_pending(&self.root, &fresh, member_name, now_ms())
                .map_err(text)?;
            mailbox::claim_delivery(&self.root, team_id, member_name, &mailbox::ids_of(&current))
                .map_err(text)?;
            Ok(current)
        })?;
        if !unread.is_empty() {
            let prompt = mailbox::mailbox_prompt(member_name, &unread);
            let accepted =
                self.dispatch_member(team_id, member_name, &prompt, DeliveryMode::Steer, None);
            let ids = mailbox::ids_of(&unread);
            self.with_team_lock(team_id, || {
                let _ = if accepted {
                    mailbox::mark_delivered(&self.root, team_id, member_name, &ids)
                } else {
                    mailbox::release_delivery(&self.root, team_id, member_name, &ids)
                };
            });
            return Ok(());
        }

        let ticket = self.with_team_lock(team_id, || -> OpResult<Option<_>> {
            let Some(mut fresh) = self.root.read_team(team_id).map_err(text)? else {
                return Ok(None);
            };
            if fresh.is_halted() || fresh.is_staged() {
                return Ok(None);
            }
            let Some(current) = fresh
                .members
                .iter()
                .find(|member| member.name == member_name && !member.is_removed())
            else {
                return Ok(None);
            };
            if !self.member_available(current) {
                return Ok(None);
            }
            let parked = self.parked_attempt(team_id, member_name);
            match plan_dispatch(
                &mut fresh,
                member_name,
                parked.as_deref(),
                self.config.execution_prompt.as_deref(),
            ) {
                DispatchDecision::Nothing { changed } => {
                    if changed {
                        self.write(&fresh)?;
                    }
                    Ok(None)
                }
                DispatchDecision::Dispatch(ticket) => {
                    // A recovered generation is parked before delivery, so each
                    // (member, attempt) recovery happens once even if every
                    // later kick sees a stopped member.
                    if ticket.recovered_owned {
                        self.set_parked(team_id, member_name, Some(ticket.attempt_id.clone()));
                    } else {
                        self.set_parked(team_id, member_name, None);
                    }
                    self.write(&fresh)?;
                    Ok(Some(ticket))
                }
            }
        })?;
        let Some(ticket) = ticket else {
            return Ok(());
        };

        let prompt = assignment_prompt(&ticket, &self.config.state_dir, team_id);
        if self.dispatch_member(
            team_id,
            member_name,
            &prompt,
            DeliveryMode::Queue,
            Some(&ticket.attempt_id),
        ) {
            return Ok(());
        }
        // Roll back only this exact failed dispatch.
        self.with_team_lock(team_id, || -> OpResult<()> {
            let Some(mut fresh) = self.root.read_team(team_id).map_err(text)? else {
                return Ok(());
            };
            if let Some(parked) = rollback_dispatch(&mut fresh, &ticket) {
                self.set_parked(team_id, member_name, parked);
                self.write(&fresh)?;
            }
            Ok(())
        })
    }

    /// Hand a member a text, starting it with this text when it has no
    /// session yet. Records why a member could not start, so the captain does
    /// not only see an unexplained unstarted member.
    pub(crate) fn dispatch_member(
        &self,
        team_id: &str,
        member_name: &str,
        text_to_send: &str,
        mode: DeliveryMode,
        attempt_id: Option<&str>,
    ) -> bool {
        let result = self.with_team_lock(team_id, || -> OpResult<bool> {
            let Some(mut team) = self.root.read_team(team_id).map_err(text)? else {
                return Ok(false);
            };
            if team.captain_session_id != self.captain_id() || team.is_halted() || team.is_staged()
            {
                return Ok(false);
            }
            let Some(index) = team
                .members
                .iter()
                .position(|member| member.name == member_name && !member.is_removed())
            else {
                return Ok(false);
            };
            let member = &team.members[index];
            if member.is_stopping()
                || team.tasks.iter().any(|task| {
                    task.is_reassigning() && task.assignee.as_deref() == Some(member_name)
                })
            {
                return Ok(false);
            }
            if let Some(attempt) = attempt_id
                && !team.tasks.iter().any(|task| {
                    task.attempt_id.as_deref() == Some(attempt)
                        && task.assignee.as_deref() == Some(member_name)
                        && task.status.is_open_attempt()
                })
            {
                return Ok(false);
            }
            if member.is_spawned() {
                return Ok(self.host.deliver(&team, member, text_to_send, mode));
            }
            let id = self.host.spawn_member(&team, member, text_to_send)?;
            let member = &mut team.members[index];
            member.id = id;
            member.spawn_error = None;
            self.write(&team)?;
            Ok(true)
        });
        match result {
            Ok(accepted) => accepted,
            Err(reason) => {
                tracing_warn(&format!(
                    "agent-teams: dispatch to {member_name} failed: {reason}"
                ));
                let _ = self.with_team_lock(team_id, || -> OpResult<()> {
                    let Some(mut team) = self.root.read_team(team_id).map_err(text)? else {
                        return Ok(());
                    };
                    if let Some(member) = team
                        .members
                        .iter_mut()
                        .find(|member| member.name == member_name && !member.is_removed())
                        && !member.is_spawned()
                    {
                        member.spawn_error = Some(reason);
                        self.write(&team)?;
                    }
                    Ok(())
                });
                false
            }
        }
    }

    /// A member's turn started (`running`) or ended (`!running`). On the idle
    /// edge an open attempt is parked and the member is kicked, which is what
    /// continues it to its next ready task.
    pub fn member_status_edge(&self, team_id: &str, member_id: &str, running: bool) {
        let name = self.with_team_lock(team_id, || -> OpResult<Option<String>> {
            let Some(mut team) = self.root.read_team(team_id).map_err(text)? else {
                return Ok(None);
            };
            let Some(index) = team
                .members
                .iter()
                .position(|member| member.id == member_id && !member.is_removed())
            else {
                return Ok(None);
            };
            let name = team.members[index].name.clone();
            let next = if running {
                self.set_parked(team_id, &name, None);
                MemberStatus::Working
            } else {
                let owned = crate::scheduler::owned_open_task(&team.tasks, &name)
                    .and_then(|task| task.attempt_id.clone());
                self.set_parked(team_id, &name, owned);
                MemberStatus::Idle
            };
            if team.members[index].status != next {
                team.members[index].status = next;
                self.write(&team)?;
            }
            Ok(Some(name))
        });
        if !running && let Ok(Some(name)) = name {
            self.kick_member(team_id, &name);
        }
    }

    /// The captain's turn ended. A captain takeover lasts one turn: whatever
    /// it still holds returns to the shared pool.
    pub fn captain_idle_edge(&self) {
        let Ok(Some(team)) = self.current_team() else {
            return;
        };
        let requeued = self.with_team_lock(&team.id, || -> OpResult<bool> {
            let Some(mut fresh) = self.root.read_team(&team.id).map_err(text)? else {
                return Ok(false);
            };
            if fresh.captain_session_id != self.captain_id() {
                return Ok(false);
            }
            let mut requeued = false;
            for task in fresh.tasks.iter_mut() {
                if task.assignee.as_deref() != Some(CAPTAIN_KEY) || task.status.is_terminal() {
                    continue;
                }
                invalidate_task_attempt(task, None, false);
                requeued = true;
            }
            if requeued {
                self.write(&fresh)?;
            }
            Ok(requeued)
        });
        if requeued == Ok(true) {
            self.kick_team(&team.id);
        }
    }

    /// Whether a member may take another model step: the team runs, the
    /// member is current, and none of its tasks is mid-handoff.
    pub fn admit_member_step(&self, team_id: &str, member_id: &str) -> bool {
        self.with_team_lock(team_id, || {
            let Ok(Some(team)) = self.root.read_team(team_id) else {
                return false;
            };
            let Some(member) = team.member_by_id(member_id) else {
                return false;
            };
            team.captain_session_id == self.captain_id()
                && !team.is_staged()
                && !team.is_halted()
                && !member.is_stopping()
                && !team.tasks.iter().any(|task| {
                    task.is_reassigning() && task.assignee.as_deref() == Some(member.name.as_str())
                })
        })
    }

    /// The member's open attempt, captured when its turn fails.
    pub fn observe_member_attempt(
        &self,
        team_id: &str,
        member_id: &str,
    ) -> Option<ObservedAttempt> {
        let team = self.root.read_team(team_id).ok()??;
        let member = team.member_by_id(member_id)?;
        let task = crate::scheduler::owned_open_task(&team.tasks, &member.name)?;
        Some(ObservedAttempt {
            task_id: task.id.clone(),
            attempt: task.attempt,
            attempt_id: task.attempt_id.clone(),
        })
    }

    /// Record a member's final turn failure: the observed attempt fails, the
    /// captain is told and woken, and the member is kicked once quiet.
    /// `observed` is what was open when the failure happened; a reassignment
    /// or completion that won the lock since then leaves the task alone.
    pub fn fail_member_open_attempt(
        &self,
        team_id: &str,
        member_id: &str,
        observed: Option<&ObservedAttempt>,
        failure: &str,
    ) -> bool {
        let prepared = self.with_team_lock(team_id, || -> OpResult<Option<TeamMessage>> {
            let Some(mut team) = self.root.read_team(team_id).map_err(text)? else {
                return Ok(None);
            };
            if team.is_halted() || team.captain_session_id != self.captain_id() {
                return Ok(None);
            }
            let Some(member_index) = team
                .members
                .iter()
                .position(|member| member.id == member_id && !member.is_removed())
            else {
                return Ok(None);
            };
            let member_name = team.members[member_index].name.clone();
            let task_index = team.tasks.iter().position(|task| {
                task.assignee.as_deref() == Some(member_name.as_str()) && task.status.is_open_attempt()
            });
            let current = task_index.map(|index| &team.tasks[index]);
            let matches = match (current, observed) {
                (None, None) => true,
                (Some(task), Some(seen)) => {
                    task.id == seen.task_id
                        && task.attempt == seen.attempt
                        && task.attempt_id == seen.attempt_id
                }
                _ => false,
            };
            if !matches {
                return Ok(None);
            }
            if task_index.is_none() && team.members[member_index].status != MemberStatus::Working {
                return Ok(None);
            }
            let content = match task_index {
                Some(index) => {
                    let task = &mut team.tasks[index];
                    task.status = TaskStatus::Failed;
                    task.output = Some(failure.to_owned());
                    task.updated_at = now_ms();
                    format!(
                        "Member \"{member_name}\" hit an unrecoverable turn failure: {failure}. Task {} (\"{}\") was marked failed; reassign it or retry when ready.",
                        task.id, task.subject
                    )
                }
                None => format!(
                    "Member \"{member_name}\" hit an unrecoverable turn failure: {failure}. No open attempt was owned."
                ),
            };
            if self.host.member_activity(member_id) != MemberActivity::Running {
                team.members[member_index].status = MemberStatus::Idle;
            }
            let mut message = TeamMessage::new(member_name, CAPTAIN_KEY, content);
            message.delivery_claimed_at = Some(now_ms());
            self.write(&team)?;
            mailbox::append(&self.root, team_id, CAPTAIN_KEY, &message).map_err(text)?;
            Ok(Some(message))
        });
        let Ok(Some(message)) = prepared else {
            return false;
        };
        self.deliver_captain_mail(team_id, &[message]);
        true
    }

    /// Steer mail into the captain and settle its delivery marks.
    pub(crate) fn deliver_captain_mail(&self, team_id: &str, messages: &[TeamMessage]) -> bool {
        if messages.is_empty() {
            return true;
        }
        let delivered = self
            .host
            .steer_captain(&mailbox::mailbox_prompt(CAPTAIN_KEY, messages));
        let ids = mailbox::ids_of(messages);
        self.with_team_lock(team_id, || {
            let _ = if delivered {
                mailbox::acknowledge(&self.root, team_id, CAPTAIN_KEY, &ids)
            } else {
                mailbox::release_delivery(&self.root, team_id, CAPTAIN_KEY, &ids)
            };
        });
        delivered
    }

    /// Deliver captain mail no live path took — after a restart, or a wake
    /// that failed. Called when the captain goes idle.
    pub fn flush_captain_mail(&self) {
        let Ok(Some(team)) = self.current_team() else {
            return;
        };
        let pending = self.with_team_lock(&team.id, || -> OpResult<Vec<TeamMessage>> {
            let Some(fresh) = self.root.read_team(&team.id).map_err(text)? else {
                return Ok(Vec::new());
            };
            let current = mailbox::read_current_pending(&self.root, &fresh, CAPTAIN_KEY, now_ms())
                .map_err(text)?;
            mailbox::claim_delivery(
                &self.root,
                &team.id,
                CAPTAIN_KEY,
                &mailbox::ids_of(&current),
            )
            .map_err(text)?;
            Ok(current)
        });
        if let Ok(pending) = pending {
            self.deliver_captain_mail(&team.id, &pending);
        }
    }

    /// Kick the captain's team at session start (cold recovery).
    pub fn resume_after_start(&self) -> Option<TeamState> {
        let team = self.current_team().ok()??;
        self.kick_team(&team.id);
        Some(team)
    }
}

fn tracing_warn(message: &str) {
    tracing::warn!("{message}");
}
