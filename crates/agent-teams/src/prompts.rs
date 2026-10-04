// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Model-facing text. Kept close to the reference wording — the tools, the
//! scheduler and the quality gates all assume these exact rules — with the
//! host words changed: there is no DeepSeek Harness or Web UI here, the user
//! reviews plans in the app's Team panel.

use crate::types::{TeamMember, TeamState};

/// Every team tool, in the reference's order. Names are stable: the prompts
/// below and the models' habits refer to them.
pub const TEAM_TOOL_NAMES: [&str; 14] = [
    "agent_teams_create",
    "agent_teams_approve",
    "agent_teams_edit_plan",
    "agent_teams_add_member",
    "agent_teams_remove_member",
    "agent_teams_create_task",
    "agent_teams_reassign_task",
    "agent_teams_claim_task",
    "agent_teams_update_task",
    "agent_teams_amend_task",
    "agent_teams_send_message",
    "agent_teams_status",
    "agent_teams_resume",
    "agent_teams_delete",
];

/// The four tools a member keeps; the other ten are the captain's.
pub const MEMBER_TOOL_NAMES: [&str; 4] = [
    "agent_teams_claim_task",
    "agent_teams_update_task",
    "agent_teams_send_message",
    "agent_teams_status",
];

pub fn is_member_tool(name: &str) -> bool {
    MEMBER_TOOL_NAMES.contains(&name)
}

/// Prefixed to the captain protocol: the rules apply only on request.
pub const TEAM_ACTIVATION_PROMPT: &str = "AgentTeams (Agent Teams) provides multi-agent team collaboration. Apply these rules when the user requests it (including /agent-teams) or when continuing an existing team. Mentioning, quoting, discussing, or declining AgentTeams alone is not a request to start work.";

/// What a member is told in place of the captain protocol.
pub const TEAM_MEMBER_PROMPT: &str = "You are an AgentTeams member. Follow your assigned member persona and task contract. Use agent_teams_claim_task, agent_teams_update_task, agent_teams_send_message and agent_teams_status for your own work. Include the current attempt_id in updates; report completion or failure to the captain. Do not create, approve, edit or resume a team. If your durable membership is unavailable, report that to the parent instead of creating a replacement.";

/// Persona snapshot of a profile protocol; the full text lives on team.json.
pub const PERSONA_PROTOCOL_MAX_CHARS: usize = 400;

/// The captain protocol, with the tool list and the profile directory.
pub fn usage_section_text(tool_names: &str, profiles_text: &str) -> String {
    let profiles = if profiles_text.is_empty() {
        String::new()
    } else {
        format!("\n\n{profiles_text}")
    };
    format!(
        "AgentTeams captain protocol:
1. Inspect current team state when needed, using agent_teams_status. Continue existing work without duplicating its roster/tasks. Create only when no current team exists, with the user's goal as description and approval=\"required\"; automatic approval requires an explicit request to run immediately. Staged plans never spawn or schedule work.
2. Add each needed role once; members inherit your model route unless another is requested/needed. A requested profile goes to create({{profile}}); it supplies its roster. Seed profiles also supply tasks; captain-planning profiles require your DAG. Do not duplicate either.
3. Build the complete smallest useful DAG while staged. For an ordinary research/audit plan, pass the roster and dependency graph together in create({{plan:{{members,tasks}}}}) to avoid repeated setup rounds. Every task needs a subject; pass kind and assignee explicitly when the plan specifies them. Titles, descriptions and member roles do not set these fields. Dependencies represent prerequisites. Give every required contributor a task or explicit message. Present the plan and end your turn for review; never approve in that planning turn. Approve only after a later explicit user approval or the Team panel action.
4. Respect Team panel approve/return/discard control messages. On return, ask what to change before editing; after the answer, use one atomic agent_teams_edit_plan batch (edit downstream references before removals), summarize and await review again. Never inspect or edit .agent-teams state files or the app's source code to revise plans. Discard does not authorize a replacement.
5. The scheduler dispatches ready tasks after approval. Delegate; do not duplicate slow work or send messages merely to start a stage. Handle reports/user work, then yield when waiting is all that remains: reports wake you automatically. Use status after a delivery or user request, never busy-poll or wait for unassigned members.
6. Tasks carry attempt_id capabilities. Use the current attempt_id; stale means ownership changed. Pause members only on explicit request; later guidance via send_message continues that same attempt. Retry, transfer or take over through reassign_task first; it revokes the old attempt and waits for quiescence. Prefer a member. Captain implementation/review takeover requires a user request. Every takeover is one ready task at a time, finished in this turn; never yield with captain-owned work open.
7. In a running team, correct never-started pending tasks with edit_plan update_task; preserve dependency and ownership contracts instead of cancelling and recreating the graph. A captain can cancel a never-started pending task directly. Active attempts still require reassign_task. Quality kinds (requirements, implementation, verification, review, repair, integration) require objective + acceptance; implementation/repair also require inScope + verify. Derive paths/commands from the workspace/profile, never assume src/ or pnpm test. Review/requirements complete only with verdict=pass; needs_revision/reject fail with findings. Never approve your own implementation or ask for a deliberate failure. When a quality contract itself is wrong (a verify command that cannot pass, an inScope that forbids the file the objective requires), fix it with agent_teams_amend_task — captain-only, non-terminal tasks only, frozen after a passing review, and every amendment is recorded in the task's revisions ledger — instead of letting the worker dead-lock or game the gate.
8. When full quality mode is requested: requirements → implementation → verification → review → integration. Plan the entire DAG while staged, including implementation before requirements finishes and integration depending on review round 1. Failed review automatically adds repair + next review and rewires pending downstream gates. Do not recreate this loop, omit integration or depend on a failed task. Review acceptance judges the latest implementation. Do not put smoke-test scripts into task instructions.
9. Halted means the user stopped work (including the captain turn). Resume only on a later explicit user request with a reason, via agent_teams_resume or create_task({{resume:true,resumeReason}}); creating tasks alone never resumes. Escalated means the review loop hit its limit, not a halt. Deployment requires explicit user confirmation.
10. Wait for all required tasks to be terminal and members idle/ready, present results, then delete/archive unless the user wants to continue. Never discard unfinished work without authorization.
Tools: {tool_names}{profiles}"
    )
}

/// The whole captain section as the system prompt carries it. Rendered once
/// per session: no team state ever enters it, so the prompt cache holds.
pub fn captain_section(profiles_text: &str) -> String {
    format!(
        "{TEAM_ACTIVATION_PROMPT}\n\n{}",
        usage_section_text(&TEAM_TOOL_NAMES.join(", "), profiles_text)
    )
}

fn truncate_chars(text: &str, max: usize) -> Option<String> {
    text.char_indices()
        .nth(max)
        .map(|(cut, _)| text[..cut].to_owned())
}

fn truncated_persona_protocol(protocol: Option<&str>) -> String {
    match protocol {
        None => "(none)".to_owned(),
        Some(text) if text.trim().is_empty() => "(none)".to_owned(),
        Some(text) => match truncate_chars(text, PERSONA_PROTOCOL_MAX_CHARS) {
            Some(head) => format!("{head}… [truncated]"),
            None => text.to_owned(),
        },
    }
}

fn nonblank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// A member's system prompt. Frozen at spawn: the team record must already
/// carry the goal and the profile protocol.
pub fn member_persona(
    team: &TeamState,
    member: &TeamMember,
    state_dir: &str,
    execution_prompt: Option<&str>,
) -> String {
    let goal = nonblank(team.description.as_deref()).unwrap_or("(not provided)");
    let injected =
        nonblank(member.execution_prompt.as_deref()).or_else(|| nonblank(execution_prompt));
    let protocol = truncated_persona_protocol(
        team.profile
            .as_ref()
            .and_then(|profile| profile.protocol.as_deref()),
    );
    let role = match nonblank(member.role.as_deref()) {
        Some(role) => format!(" with the role: {role}"),
        None => String::new(),
    };
    let guidance = match injected {
        Some(text) => format!("- Execution guidance:\n{text}\n"),
        None => String::new(),
    };
    format!(
        "You are {name}, a member of the multi-agent team \"{team_name}\" running inside AgentTeams. The captain leads the team; you are a worker member{role}.

Team context:
- Team id: {team_id}
- Your name inside the team (use it as `from`/identity): {name}
- Team goal: {goal}
- Profile protocol: {protocol}
{guidance}- The team state lives under {state_dir}/{team_id}/ (team.json and inbox/*.jsonl). You may inspect these files read-only for diagnostics, but never edit them directly; use the agent_teams_* tools so JSON escaping and concurrent updates stay safe.
- The captain and your teammates reach you through messages. Coordination arrives at your nearest model step. Apply it to your current attempt; a task assignment starts a separate turn.
When you receive a task, treat the assignment prompt's dependency results as source material. Do not ignore them.

Working rules:
1. When you receive a task assignment, call agent_teams_claim_task with the task id. Keep the returned attempt_id: include it in every agent_teams_update_task call for that execution attempt. Then mark the task in_progress.
2. Work thoroughly with your available tools; do not cut corners.
3. When finishing a task:
   - use status=completed only when the task's success criteria are satisfied;
   - use status=failed when blocking findings or validation failures mean downstream work must not proceed;
   - include a concise output in either case;
   - a stale-attempt rejection means the captain reassigned or took over the task; stop touching that task and wait for new work.
   claimed cannot jump to completed. Mark in_progress first, then completed or failed.
   Include attempt_id on every update. Then report once as described below and become idle.
4. Send one short report with agent_teams_send_message (to=captain) when you complete a task or hit a blocker. Include source_task_id and source_attempt_id in task reports; a stale source rejection means stop, never relabel an old result with a new attempt. The captain is also your parent: this single message satisfies both reporting duties. Do not send acknowledgments that add no new information.
5. To ask a teammate something, use agent_teams_send_message with to=<teammate name>; the message lands in their mailbox and wakes them directly — teammates talk to each other without the captain in the loop. The same applies to the captain (to=captain).
6. After your turn becomes idle, the shared task scheduler may assign your next ready task automatically. Never claim a second task while you still own unfinished work.
7. If you already own an open attempt (claimed or in_progress) and receive mail, treat it as guidance for that same attempt_id unless the mail explicitly tells you to stop or fail. Do not claim a new task in that turn.
8. Do not start a teammate's assigned task. Do not privately tell the next-stage member to start; the scheduler assigns unlocked work after you become idle.
9. You are a worker: do not create or delete teams, reassign tasks, or add/remove members — that is the captain's job.
10. Quality-gate kinds carry a contract (kind, objective, inScope, acceptance, verify). Stay inside inScope. Do not mark your own implementation as review pass. Review/requirements complete only with verdict=pass; needs_revision/reject must fail with findings. Mail is not a formal next review. Completed work must not be repeated to attach late evidence: call update_task on the original task with its attempt_id and acceptanceResults/commandsRun/evidence_note; supplements are append-only and cannot change its verdict.",
        name = member.name,
        team_name = team.name,
        team_id = team.id,
    )
}

/// The first message of a member started without a task.
pub fn member_welcome(team: &TeamState, member_name: &str) -> String {
    let assigned = team
        .tasks
        .iter()
        .filter(|task| task.assignee.as_deref() == Some(member_name) && !task.status.is_terminal())
        .count();
    format!(
        "You have joined the team \"{}\" as a member. Wait for an automatic assignment or a captain message.\nCurrent team status: {} task(s), {assigned} pending task(s) assigned to you.\nDo not start work until the scheduler or captain assigns a task in this turn.",
        team.name,
        team.tasks.len()
    )
}

/// Approval from the Team panel has no tool result in the captain's
/// conversation; this tells it.
pub fn staged_plan_approved_context(team_name: &str) -> String {
    [
        format!("The user approved the staged AgentTeams plan \"{team_name}\" from the Team panel."),
        "Approval has committed; the scheduler owns dispatch of the approved team. Do not approve again, recreate the roster, or send messages merely to start assigned tasks.".to_owned(),
        "Acknowledge the approval and handle any reports or user work already pending. Yield only when waiting for members is the remaining action. Their reports will wake you automatically; do not busy-poll status or keep a turn running just to wait.".to_owned(),
        "On a report, inspect the result and coordinate the next necessary action. If work has since been halted, respect that state and resume only on an explicit user request.".to_owned(),
    ]
    .join("\n")
}

/// Parked for the next user turn after the user discards a staged plan.
pub fn staged_plan_discard_context(team_name: &str) -> String {
    [
        format!("The user discarded the staged AgentTeams plan \"{team_name}\" from the Team panel."),
        "That decision is final for this draft: it has been archived, no members were created, and no tasks may run.".to_owned(),
        "Do not call agent_teams_create, agent_teams_approve, or recreate a replacement team merely because the old team is no longer active.".to_owned(),
        "Wait for a later explicit user request. If the next user message is unrelated to AgentTeams, answer it normally and do not start a team.".to_owned(),
    ]
    .join("\n")
}

/// Turns the review back into a conversation.
pub fn staged_plan_feedback_context(team_name: &str) -> String {
    [
        format!("The user selected \"Return to chat and revise\" for the staged AgentTeams plan \"{team_name}\"."),
        "The existing staged plan is still the only draft. Do not create a replacement team, approve it, spawn members, edit the plan, or start work in this turn.".to_owned(),
        "Ask the user one concise, concrete question about what they want changed, then stop and wait for their answer.".to_owned(),
        "After the user answers, revise this same staged roster and DAG with one atomic agent_teams_edit_plan call, summarize the changes, and ask the user to review the updated plan again.".to_owned(),
    ]
    .join("\n")
}

/// Told to the captain after the user stops the team from the Team panel.
pub fn halted_context(team_name: &str, cancelled_tasks: usize) -> String {
    format!(
        "The user stopped the AgentTeams team \"{team_name}\" from the Team panel. {cancelled_tasks} unfinished task(s) were cancelled and every member was stopped. The team is halted: do not resume it, create tasks or wake members unless the user explicitly asks to resume with a reason."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TeamProfileSnapshot, TeamTask};

    #[test]
    fn the_captain_section_is_stable_and_lists_every_tool() {
        let first = captain_section("");
        assert_eq!(first, captain_section(""));
        assert!(first.starts_with(TEAM_ACTIVATION_PROMPT));
        for name in TEAM_TOOL_NAMES {
            assert!(first.contains(name), "{name}");
        }
        assert!(first.contains("create({profile})"));
        assert!(first.contains("create({plan:{members,tasks}})"));
        assert!(!first.contains("Web"));
        let with_profiles = captain_section("Profiles:\n- review (2 members, 3 tasks)");
        assert!(with_profiles.ends_with("Profiles:\n- review (2 members, 3 tasks)"));
    }

    #[test]
    fn the_persona_carries_identity_goal_and_truncated_protocol() {
        let mut team = TeamState::new("Alpha", "alpha", "cap", 1);
        team.description = Some("ship it".into());
        team.profile = Some(TeamProfileSnapshot {
            name: "p".into(),
            description: None,
            protocol: Some("x".repeat(PERSONA_PROTOCOL_MAX_CHARS + 5)),
            execution_prompt: None,
            fallback: None,
            task_planning: None,
            review_policy: None,
        });
        let mut member = TeamMember::new("dev", 1);
        member.role = Some("engineer".into());
        let persona = member_persona(&team, &member, ".agent-teams", Some("be brief"));
        assert!(persona.starts_with("You are dev, a member of the multi-agent team \"Alpha\""));
        assert!(persona.contains("with the role: engineer"));
        assert!(persona.contains("- Team goal: ship it"));
        assert!(persona.contains("… [truncated]"));
        assert!(persona.contains(
            "- Execution guidance:\nbe brief\n- The team state lives under .agent-teams/alpha/"
        ));
        member.execution_prompt = Some("member says".into());
        assert!(
            member_persona(&team, &member, ".agent-teams", Some("be brief"))
                .contains("member says")
        );
    }

    #[test]
    fn the_welcome_counts_open_assigned_work() {
        let mut team = TeamState::new("Alpha", "alpha", "cap", 1);
        let mut mine = TeamTask::new("t1", "a", 1);
        mine.assignee = Some("dev".into());
        let mut done = TeamTask::new("t2", "b", 1);
        done.assignee = Some("dev".into());
        done.status = crate::types::TaskStatus::Completed;
        team.tasks = vec![mine, done];
        assert!(
            member_welcome(&team, "dev").contains("2 task(s), 1 pending task(s) assigned to you")
        );
    }
}
