// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The scheduling decisions, without I/O.
//!
//! Every idle edge and every task-graph mutation attempts one atomic claim
//! for one member and wakes it with an assignment. A resident member that
//! becomes idle while it still owns an open attempt is *parked*: only an
//! explicit reassignment may rotate that capability. Automatic retry is
//! reserved for cold recovery — an open attempt this process never saw its
//! owner settle (the app restarted, the session was reopened).

use std::collections::HashSet;

use crate::transitions::{begin_task_attempt, unsatisfied_dependencies};
use crate::types::{
    AcceptanceResult, CommandResult, MemberStatus, ReviewFinding, ReviewVerdict, TaskStatus,
    TeamState, TeamTask, now_ms,
};

/// Per-dependency output cap in the assignment prompt.
pub const DEPENDENCY_OUTPUT_MAX_CHARS: usize = 2_000;
/// Combined dependency-output budget in the assignment prompt.
pub const DEPENDENCY_OUTPUTS_TOTAL_MAX_CHARS: usize = 12_000;

/// One completed upstream task shown to the assignee.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyOutput {
    pub id: String,
    pub subject: String,
    pub profile_seed_id: Option<String>,
    pub output: Option<String>,
}

fn seed_of(task: &TeamTask) -> Option<String> {
    task.profile_seed_id
        .as_deref()
        .map(str::trim)
        .filter(|seed| !seed.is_empty())
        .map(str::to_owned)
}

/// The completed ancestors of `task_id`, dependencies before dependents. A
/// cycle stops only its own branch.
pub fn collect_completed_dependency_outputs(
    tasks: &[TeamTask],
    task_id: &str,
) -> Vec<DependencyOutput> {
    fn walk<'a>(
        tasks: &'a [TeamTask],
        id: &str,
        root: &str,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<&'a TeamTask>,
    ) {
        if visiting.contains(id) || visited.contains(id) {
            return;
        }
        visiting.insert(id.to_owned());
        if let Some(task) = tasks.iter().find(|task| task.id == id) {
            for dependency in &task.dependencies {
                walk(tasks, dependency, root, visiting, visited, ordered);
            }
            if id != root {
                ordered.push(task);
            }
        }
        visiting.remove(id);
        visited.insert(id.to_owned());
    }
    let mut ordered = Vec::new();
    walk(
        tasks,
        task_id,
        task_id,
        &mut HashSet::new(),
        &mut HashSet::new(),
        &mut ordered,
    );
    ordered
        .into_iter()
        .filter(|task| task.status == TaskStatus::Completed)
        .map(|task| DependencyOutput {
            id: task.id.clone(),
            subject: task.subject.clone(),
            profile_seed_id: seed_of(task),
            output: task.output.clone(),
        })
        .collect()
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn char_prefix(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((cut, _)) => &text[..cut],
        None => text,
    }
}

/// Completed-dependency outputs with per-item and total truncation; the
/// oldest items go first when the total is over budget.
pub fn format_dependency_outputs(items: &[DependencyOutput]) -> String {
    if items.is_empty() {
        return "(none)".to_owned();
    }
    let formatted: Vec<String> = items
        .iter()
        .map(|item| {
            let seed = item
                .profile_seed_id
                .as_ref()
                .map(|seed| format!(" [{seed}]"))
                .unwrap_or_default();
            let raw = match item.output.as_deref() {
                None | Some("") => "(no output recorded)",
                Some(output) => output,
            };
            let body = if char_len(raw) > DEPENDENCY_OUTPUT_MAX_CHARS {
                format!(
                    "{} [truncated]",
                    char_prefix(raw, DEPENDENCY_OUTPUT_MAX_CHARS)
                )
            } else {
                raw.to_owned()
            };
            format!("- {}{seed} {}:\n  {body}", item.id, item.subject)
        })
        .collect();
    let mut selected: &[String] = &formatted;
    while selected.len() > 1 && char_len(&selected.join("\n")) > DEPENDENCY_OUTPUTS_TOTAL_MAX_CHARS
    {
        selected = &selected[1..];
    }
    if selected.len() == 1 && char_len(&selected[0]) > DEPENDENCY_OUTPUTS_TOTAL_MAX_CHARS {
        return format!(
            "{} [truncated]",
            char_prefix(&selected[0], DEPENDENCY_OUTPUTS_TOTAL_MAX_CHARS)
        );
    }
    selected.join("\n")
}

/// The member's claimed or in-progress task.
pub fn owned_open_task<'a>(tasks: &'a [TeamTask], member_name: &str) -> Option<&'a TeamTask> {
    tasks
        .iter()
        .find(|task| task.assignee.as_deref() == Some(member_name) && task.status.is_open_attempt())
}

/// Index of the next ready task for a member: one assigned to it first,
/// otherwise one from the shared pool.
pub fn next_ready_task(tasks: &[TeamTask], member_name: &str) -> Option<usize> {
    let ready = |task: &TeamTask| {
        task.status == TaskStatus::Pending
            && !task.is_reassigning()
            && unsatisfied_dependencies(tasks, &task.dependencies).is_empty()
    };
    tasks
        .iter()
        .position(|task| ready(task) && task.assignee.as_deref() == Some(member_name))
        .or_else(|| {
            tasks
                .iter()
                .position(|task| ready(task) && task.assignee.is_none())
        })
}

/// What a failed automatic recovery must restore.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviousGeneration {
    pub status: TaskStatus,
    pub attempt: Option<u64>,
    pub attempt_id: Option<String>,
    pub output: Option<String>,
    pub verdict: Option<ReviewVerdict>,
    pub findings: Option<Vec<ReviewFinding>>,
    pub changed_paths: Option<Vec<String>>,
    pub acceptance_results: Option<Vec<AcceptanceResult>>,
    pub commands_run: Option<Vec<CommandResult>>,
}

/// One claim, made under the team lock, waiting to be delivered.
#[derive(Clone, Debug, PartialEq)]
pub struct DispatchTicket {
    pub task_id: String,
    pub member_name: String,
    pub attempt: u64,
    pub attempt_id: String,
    pub previous_assignee: Option<String>,
    /// This ticket rotates an open attempt this process never saw settle.
    pub recovered_owned: bool,
    pub previous: Option<PreviousGeneration>,
    pub subject: String,
    pub description: Option<String>,
    pub team_description: Option<String>,
    pub profile_protocol: Option<String>,
    pub profile_seed_id: Option<String>,
    pub dependency_outputs: Vec<DependencyOutput>,
    pub execution_prompt: Option<String>,
    pub kind: String,
    pub round: Option<u32>,
    pub objective: Option<String>,
    pub in_scope: Option<Vec<String>>,
    pub out_of_scope: Option<Vec<String>>,
    pub acceptance: Option<Vec<String>>,
    pub verify: Option<Vec<String>>,
    pub reviewed_task_id: Option<String>,
}

/// The outcome of [`plan_dispatch`].
#[derive(Clone, Debug, PartialEq)]
pub enum DispatchDecision {
    /// No work for this member now. `changed` asks the caller to save the
    /// record (the member was marked idle).
    Nothing { changed: bool },
    /// Claimed: save the record, then deliver the assignment.
    Dispatch(Box<DispatchTicket>),
}

/// Decide one claim for `member_name` and apply it to `team`.
///
/// `parked_attempt` is the capability this process saw the member go idle
/// with; an open attempt that does not match it is recovered once as a new
/// generation. The caller records the ticket's capability as parked when
/// [`DispatchTicket::recovered_owned`] is set and forgets it otherwise.
pub fn plan_dispatch(
    team: &mut TeamState,
    member_name: &str,
    parked_attempt: Option<&str>,
    config_execution_prompt: Option<&str>,
) -> DispatchDecision {
    let Some(member_index) = team
        .members
        .iter()
        .position(|member| member.name == member_name && !member.is_removed())
    else {
        return DispatchDecision::Nothing { changed: false };
    };
    let owned = team.tasks.iter().position(|task| {
        task.assignee.as_deref() == Some(member_name) && task.status.is_open_attempt()
    });
    let recover = owned.is_some_and(|index| {
        let attempt = team.tasks[index].attempt_id.as_deref();
        attempt.is_none() || attempt != parked_attempt
    });
    let chosen = if recover {
        owned
    } else if owned.is_none() {
        next_ready_task(&team.tasks, member_name)
    } else {
        None
    };
    let Some(index) = chosen else {
        let member = &mut team.members[member_index];
        if member.status != MemberStatus::Idle {
            member.status = MemberStatus::Idle;
            return DispatchDecision::Nothing { changed: true };
        }
        return DispatchDecision::Nothing { changed: false };
    };

    let dependency_outputs =
        collect_completed_dependency_outputs(&team.tasks, &team.tasks[index].id);
    let task = &mut team.tasks[index];
    let previous_assignee = task.assignee.clone();
    let previous = recover.then(|| PreviousGeneration {
        status: task.status,
        attempt: task.attempt,
        attempt_id: task.attempt_id.clone(),
        output: task.output.clone(),
        verdict: task.verdict,
        findings: task.findings.clone(),
        changed_paths: task.changed_paths.clone(),
        acceptance_results: task.acceptance_results.clone(),
        commands_run: task.commands_run.clone(),
    });
    let attempt_id = begin_task_attempt(task, member_name);
    let ticket = DispatchTicket {
        task_id: task.id.clone(),
        member_name: member_name.to_owned(),
        attempt: task.attempt.unwrap_or(1),
        attempt_id,
        previous_assignee,
        recovered_owned: recover,
        previous,
        subject: task.subject.clone(),
        description: task.description.clone(),
        team_description: team.description.clone(),
        profile_protocol: team
            .profile
            .as_ref()
            .and_then(|profile| profile.protocol.clone()),
        profile_seed_id: seed_of(task),
        dependency_outputs,
        execution_prompt: team
            .profile
            .as_ref()
            .and_then(|profile| profile.execution_prompt.clone())
            .or_else(|| config_execution_prompt.map(str::to_owned)),
        kind: task.kind_or_work().as_str().to_owned(),
        round: task.round,
        objective: task.objective.clone(),
        in_scope: task.in_scope.clone(),
        out_of_scope: task.out_of_scope.clone(),
        acceptance: task.acceptance.clone(),
        verify: task.verify.clone(),
        reviewed_task_id: task.reviewed_task_id.clone(),
    };
    team.members[member_index].status = MemberStatus::Working;
    DispatchDecision::Dispatch(Box::new(ticket))
}

/// Undo a claim whose delivery failed, but only if the capability is still
/// the ticket's (a concurrent handoff wins). `None` when nothing was rolled
/// back; otherwise the attempt id to keep parked when a recovery was
/// restored, or `Some(None)` to forget the parked entry.
pub fn rollback_dispatch(team: &mut TeamState, ticket: &DispatchTicket) -> Option<Option<String>> {
    let task = team.task_mut(&ticket.task_id)?;
    if task.attempt_id.as_deref() != Some(ticket.attempt_id.as_str()) {
        return None;
    }
    let parked = match (&ticket.previous, ticket.recovered_owned) {
        (Some(previous), true) if previous.attempt_id.is_some() => {
            task.status = previous.status;
            task.assignee = ticket.previous_assignee.clone();
            task.attempt = previous.attempt;
            task.attempt_id = previous.attempt_id.clone();
            task.output = previous.output.clone();
            task.verdict = previous.verdict;
            task.findings = previous.findings.clone();
            task.changed_paths = previous.changed_paths.clone();
            task.acceptance_results = previous.acceptance_results.clone();
            task.commands_run = previous.commands_run.clone();
            previous.attempt_id.clone()
        }
        _ => {
            task.status = TaskStatus::Pending;
            task.assignee = ticket.previous_assignee.clone();
            task.attempt_id = None;
            None
        }
    };
    task.handoff_id = None;
    task.reassigning = Some(false);
    task.updated_at = now_ms();
    if let Some(member) = team
        .members
        .iter_mut()
        .find(|member| member.name == ticket.member_name && !member.is_removed())
    {
        member.status = MemberStatus::Idle;
    }
    Some(parked)
}

/// Compact JSON for one string, as `JSON.stringify` writes it.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

/// The member's turn prompt for one claimed task.
pub fn assignment_prompt(ticket: &DispatchTicket, state_dir: &str, team_id: &str) -> String {
    let description = ticket
        .description
        .as_ref()
        .map(|text| format!("\n\n{text}"))
        .unwrap_or_default();
    let seed = ticket
        .profile_seed_id
        .as_ref()
        .map(|seed| format!(" [{seed}]"))
        .unwrap_or_default();
    let goal = ticket
        .team_description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("(not provided)");
    let protocol = ticket
        .profile_protocol
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("(none)");
    let execution_prompt = ticket
        .execution_prompt
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let kind = if ticket.kind.trim().is_empty() {
        "work"
    } else {
        ticket.kind.trim()
    };
    fn list(items: &Option<Vec<String>>) -> Option<&Vec<String>> {
        items.as_ref().filter(|items| !items.is_empty())
    }
    let mut contract = vec![format!(
        "Kind: {kind}{}",
        ticket
            .round
            .map(|round| format!(" (round {round})"))
            .unwrap_or_default()
    )];
    if let Some(objective) = ticket.objective.as_deref().filter(|text| !text.is_empty()) {
        contract.push(format!("Objective: {objective}"));
    }
    if let Some(items) = list(&ticket.in_scope) {
        contract.push(format!("In scope: {}", items.join(", ")));
    }
    if let Some(items) = list(&ticket.out_of_scope) {
        contract.push(format!("Out of scope: {}", items.join(", ")));
    }
    if let Some(items) = list(&ticket.acceptance) {
        contract.push(format!("Acceptance: {}", items.join("; ")));
    }
    if let Some(items) = list(&ticket.verify) {
        contract.push(format!("Verify: {}", items.join("; ")));
    }
    if let Some(reviewed) = &ticket.reviewed_task_id {
        contract.push(format!("Reviewed task: {reviewed}"));
    }
    let contract = contract.join("\n");
    let structured = if matches!(
        kind,
        "implementation" | "repair" | "verification" | "integration"
    ) {
        let acceptance: Vec<String> = ticket
            .acceptance
            .iter()
            .flatten()
            .map(|criterion| {
                format!(
                    "{{\"criterion\":{},\"status\":\"passed\",\"evidence\":\"<what proved it>\"}}",
                    json_string(criterion)
                )
            })
            .collect();
        let commands: Vec<String> = ticket
            .verify
            .iter()
            .flatten()
            .map(|command| {
                format!(
                    "{{\"command\":{},\"status\":\"passed\",\"exitCode\":0,\"evidence\":\"<observed result>\"}}",
                    json_string(command)
                )
            })
            .collect();
        let changed = if matches!(kind, "implementation" | "repair") {
            "changedPaths: list the actual workspace-relative POSIX paths you changed.\n"
        } else {
            ""
        };
        format!(
            "\nStructured completion payload (keep these arrays in contract order):\nacceptanceResults: [{}]\ncommandsRun: [{}]\n{changed}",
            acceptance.join(","),
            commands.join(",")
        )
    } else {
        String::new()
    };
    let guidance = match execution_prompt {
        Some(text) => format!("\nExecution guidance:\n{text}\n"),
        None => String::new(),
    };
    let contract_block = if contract.is_empty() {
        String::new()
    } else {
        format!("\nContract:\n{contract}\n")
    };
    format!(
        "AgentTeams automatic task assignment from the shared task list.

You are executing as configured member \"{member}\".
Do not start a teammate's assigned task.

Team goal:
{goal}

Profile protocol:
{protocol}
{guidance}
Completed dependency results:
{dependencies}

Task: {task_id}{seed} — {subject}{description}
{contract_block}
{structured}
Attempt: {attempt}
Attempt id: {attempt_id}

Call agent_teams_claim_task for {task_id}; it will return this same attempt_id. Include attempt_id={attempt_id} in every agent_teams_update_task call. If it is rejected as stale, stop work because the task was reassigned. claimed cannot jump to completed. Mark in_progress first, then completed or failed. Include attempt_id on every update. Then send_message to captain with source_task_id and source_attempt_id and become idle.
When finishing: use status=completed only when the task's success criteria are satisfied; use status=failed when blocking findings or validation failures mean downstream work must not proceed; include a concise output in either case. Quality kinds must submit structured fields: review/requirements need verdict=pass to complete (needs_revision/reject must fail with findings); implementation/repair/verification/integration need acceptanceResults and commandsRun, while implementation/repair also need in-scope changedPaths. Use status values \"passed\" or \"failed\" inside those arrays. After the work and verification finish, call agent_teams_update_task immediately; do not wait for captain confirmation and do not continue exploring. Do not approve your own implementation. Mail is not a formal next review. Completed work must not be repeated to attach late evidence: call update_task on the original task with its attempt_id and acceptanceResults/commandsRun/evidence_note; supplements are append-only and cannot change its verdict. Treat the dependency results above as source material. Do not ignore them. Work only this task and only its in-scope paths in this turn.

State policy: {state_dir}/{team_id}/ is read-only diagnostics; mutate team state only through agent_teams_* tools.",
        member = ticket.member_name,
        dependencies = format_dependency_outputs(&ticket.dependency_outputs),
        task_id = ticket.task_id,
        subject = ticket.subject,
        attempt = ticket.attempt,
        attempt_id = ticket.attempt_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TaskKind, TeamMember};

    fn team() -> TeamState {
        let mut team = TeamState::new("Alpha", "alpha", "cap", 1);
        team.members.push(TeamMember::new("dev", 1));
        team.members.push(TeamMember::new("ops", 1));
        team
    }

    fn task(id: &str, deps: &[&str]) -> TeamTask {
        let mut task = TeamTask::new(id, format!("task {id}"), 1);
        task.dependencies = deps.iter().map(|dep| (*dep).to_owned()).collect();
        task
    }

    #[test]
    fn assigned_work_comes_before_the_shared_pool() {
        let mut shared = task("t1", &[]);
        shared.assignee = None;
        let mut mine = task("t2", &[]);
        mine.assignee = Some("dev".into());
        let tasks = vec![shared, mine];
        assert_eq!(next_ready_task(&tasks, "dev"), Some(1));
        assert_eq!(next_ready_task(&tasks, "ops"), Some(0));
    }

    #[test]
    fn dependencies_gate_readiness() {
        let mut first = task("t1", &[]);
        first.assignee = Some("ops".into());
        let second = task("t2", &["t1"]);
        let mut tasks = vec![first, second];
        assert_eq!(next_ready_task(&tasks, "dev"), None);
        tasks[0].status = TaskStatus::Completed;
        assert_eq!(next_ready_task(&tasks, "dev"), Some(1));
        tasks[1].reassigning = Some(true);
        assert_eq!(next_ready_task(&tasks, "dev"), None);
    }

    #[test]
    fn a_claim_starts_an_attempt_and_marks_the_member_working() {
        let mut team = team();
        let mut done = task("t1", &[]);
        done.status = TaskStatus::Completed;
        done.output = Some("schema".into());
        team.tasks = vec![done, task("t2", &["t1"])];
        let DispatchDecision::Dispatch(ticket) =
            plan_dispatch(&mut team, "dev", None, Some("be terse"))
        else {
            panic!("expected a dispatch");
        };
        assert_eq!(ticket.task_id, "t2");
        assert_eq!(ticket.attempt, 1);
        assert!(!ticket.recovered_owned);
        assert_eq!(ticket.dependency_outputs.len(), 1);
        assert_eq!(team.tasks[1].status, TaskStatus::Claimed);
        assert_eq!(team.member("dev").unwrap().status, MemberStatus::Working);
        let prompt = assignment_prompt(&ticket, ".agent-teams", "alpha");
        assert!(prompt.contains("Task: t2 — task t2"));
        assert!(prompt.contains("- t1 task t1:\n  schema"));
        assert!(prompt.contains(&format!("attempt_id={}", ticket.attempt_id)));
        assert!(prompt.contains("Execution guidance:\nbe terse"));
    }

    #[test]
    fn a_parked_attempt_is_not_reissued_but_an_unseen_one_is_recovered_once() {
        let mut team = team();
        team.tasks = vec![task("t1", &[])];
        let DispatchDecision::Dispatch(first) = plan_dispatch(&mut team, "dev", None, None) else {
            panic!();
        };
        // Parked: the scheduler saw the member go idle with this attempt.
        assert_eq!(
            plan_dispatch(&mut team, "dev", Some(&first.attempt_id), None),
            DispatchDecision::Nothing { changed: true }
        );
        // Cold: nothing was observed, so the open attempt rotates once.
        let DispatchDecision::Dispatch(recovered) = plan_dispatch(&mut team, "dev", None, None)
        else {
            panic!();
        };
        assert!(recovered.recovered_owned);
        assert_eq!(recovered.attempt, 2);
        assert_ne!(recovered.attempt_id, first.attempt_id);
        // A failed recovery restores the earlier generation and parks it.
        let parked = rollback_dispatch(&mut team, &recovered).unwrap();
        assert_eq!(parked.as_deref(), Some(first.attempt_id.as_str()));
        assert_eq!(team.tasks[0].attempt, Some(1));
        assert_eq!(team.tasks[0].status, TaskStatus::Claimed);
    }

    #[test]
    fn a_failed_fresh_dispatch_returns_the_task_to_the_pool() {
        let mut team = team();
        team.tasks = vec![task("t1", &[])];
        let DispatchDecision::Dispatch(ticket) = plan_dispatch(&mut team, "dev", None, None) else {
            panic!();
        };
        assert_eq!(rollback_dispatch(&mut team, &ticket), Some(None));
        assert_eq!(team.tasks[0].status, TaskStatus::Pending);
        assert_eq!(team.tasks[0].assignee, None);
        // A handoff that replaced the capability wins over the rollback.
        let DispatchDecision::Dispatch(again) = plan_dispatch(&mut team, "dev", None, None) else {
            panic!();
        };
        team.tasks[0].attempt_id = Some("other".into());
        assert_eq!(rollback_dispatch(&mut team, &again), None);
    }

    #[test]
    fn dependency_outputs_are_truncated_oldest_first() {
        let items: Vec<DependencyOutput> = (0..10)
            .map(|index| DependencyOutput {
                id: format!("t{index}"),
                subject: "s".into(),
                profile_seed_id: None,
                output: Some("x".repeat(DEPENDENCY_OUTPUT_MAX_CHARS + 10)),
            })
            .collect();
        let text = format_dependency_outputs(&items);
        assert!(text.chars().count() <= DEPENDENCY_OUTPUTS_TOTAL_MAX_CHARS);
        assert!(text.contains("- t9 s:"));
        assert!(!text.contains("- t0 s:"));
        assert!(text.contains("[truncated]"));
        assert_eq!(format_dependency_outputs(&[]), "(none)");
    }

    #[test]
    fn quality_assignments_carry_the_structured_payload() {
        let mut team = team();
        let mut implementation = task("t1", &[]);
        implementation.kind = Some(TaskKind::Implementation);
        implementation.acceptance = Some(vec!["it \"works\"".into()]);
        implementation.verify = Some(vec!["cargo test".into()]);
        team.tasks = vec![implementation];
        let DispatchDecision::Dispatch(ticket) = plan_dispatch(&mut team, "dev", None, None) else {
            panic!();
        };
        let prompt = assignment_prompt(&ticket, ".agent-teams", "alpha");
        assert!(prompt.contains(
            r#"acceptanceResults: [{"criterion":"it \"works\"","status":"passed","evidence":"<what proved it>"}]"#
        ));
        assert!(prompt.contains(
            r#"commandsRun: [{"command":"cargo test","status":"passed","exitCode":0,"evidence":"<observed result>"}]"#
        ));
        assert!(prompt.contains("changedPaths: list the actual"));
        assert!(prompt.contains("Contract:\nKind: implementation"));
    }
}
