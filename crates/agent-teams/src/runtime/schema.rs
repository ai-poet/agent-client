// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! What the model sees of each tool: its description and the JSON Schema of
//! its arguments. Argument names are the reference's, mixed snake and camel
//! case included — the prompts and the models' habits use them.

use serde_json::{Value, json};

/// One tool as offered to the model.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn strings(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// Every team tool, in [`crate::prompts::TEAM_TOOL_NAMES`] order.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "agent_teams_create",
            description: "Create a team. Use approval=required for a two-phase plan: members and tasks remain unspawned/unclaimed until the user reviews the plan in the Team panel and explicitly approves it. Optional profiles expand their configured roster; seed profiles also expand template tasks, while captain profiles leave the graph for the Captain to design. approval=automatic preserves the legacy immediate-execution path.",
            input_schema: object(
                json!({
                    "name": string("Name for the new team (used as its stable id)."),
                    "description": string("Team purpose / the goal the team will work on."),
                    "profile": string("Optional configured profile name."),
                    "plan": {
                        "type": "object",
                        "additionalProperties": false,
                        "description": "Optional complete ordinary-work roster and DAG in one atomic call, instead of separate add_member/create_task rounds. Mutually exclusive with profile. For quality gates use create_task with the explicit quality contract.",
                        "properties": {
                            "members": {
                                "type": "array",
                                "items": object(json!({
                                    "name": { "type": "string" },
                                    "role": { "type": "string" },
                                    "provider": { "type": "string" },
                                    "model": { "type": "string" },
                                    "reasoning_effort": { "type": "string" },
                                }), &["name"]),
                            },
                            "tasks": {
                                "type": "array",
                                "items": object(json!({
                                    "id": string("Local reference used by dependencies in this plan; the result maps it to a durable task id."),
                                    "subject": { "type": "string" },
                                    "description": { "type": "string" },
                                    "assignee": { "type": "string" },
                                    "dependencies": { "type": "array", "items": { "type": "string" } },
                                }), &["id", "subject"]),
                            },
                        },
                        "required": ["members", "tasks"],
                    },
                    "approval": {
                        "type": "string",
                        "enum": ["required", "automatic"],
                        "description": "required stages the plan for explicit user review; automatic starts immediately. Defaults to automatic for API compatibility.",
                    },
                }),
                &["name"],
            ),
        },
        ToolSpec {
            name: "agent_teams_approve",
            description: "Approve and start a staged team plan. Call this only in response to an explicit user approval in a new user turn; never call it during the turn that created or edited the plan. The Team panel's Approve & run button uses the same runtime directly.",
            input_schema: object(
                json!({ "confirmation": string("The user's explicit approval statement.") }),
                &["confirmation"],
            ),
        },
        ToolSpec {
            name: "agent_teams_edit_plan",
            description: "Atomically revise an AgentTeams plan. While staged, edit tasks and roster without starting work. While running, only update_task is allowed, and only for pending tasks with no prior attempt: correct dependencies, assignees or descriptions before they start; newly ready work is scheduled after commit. Never edit active/finished attempts. Submit dependent edits in order. Never modify .agent-teams files directly.",
            input_schema: object(
                json!({
                    "operations": {
                        "type": "array",
                        "description": "One atomic, ordered batch. Running teams allow only update_task for never-started pending tasks. If any operation is invalid, none of the edits are saved.",
                        "items": object(json!({
                            "action": {
                                "type": "string",
                                "enum": ["update_member", "update_task", "add_task", "remove_task", "remove_member"],
                            },
                            "member_name": string("Member name for update_member or remove_member."),
                            "task_id": string("Task id for update_task or remove_task."),
                            "subject": string("Required for add_task; optional replacement for update_task."),
                            "description": string("Optional task description."),
                            "assignee": string("Optional task assignee; an empty string moves it to the shared pool."),
                            "dependencies": strings("Complete replacement dependency list for a task."),
                            "role": string("Optional member role."),
                            "provider": string("Optional member provider; defaults to the current staged route."),
                            "model": string("Optional member model; defaults to the current staged route."),
                            "reasoning_effort": string("Optional member reasoning effort."),
                            "execution_prompt": string("Optional member-specific execution prompt."),
                        }), &["action"]),
                    },
                }),
                &["operations"],
            ),
        },
        ToolSpec {
            name: "agent_teams_add_member",
            description: "Add a member to the team roster. Planning and idle roster rows do not call a model. After approval, the member session starts with its first ready task or explicit message and remains durable for later work.",
            input_schema: object(
                json!({
                    "name": string("Unique member name inside the team."),
                    "role": string("Role of the member (e.g. researcher, engineer, reviewer)."),
                    "provider": string("Optional LLM provider route. Use only when the user explicitly requests a different provider; requires model."),
                    "model": string("Optional model override. Omit for the captain's current model (or the configured memberModel default)."),
                    "reasoning_effort": string("Optional reasoning effort override: one of the target model's supported effort ids, or \"default\" to force its default. When omitted, the captain's effort is inherited only for the same provider/model; a changed route uses the target default."),
                    "executionPrompt": string("Optional member-specific execution prompt. It remains editable while staged."),
                }),
                &["name"],
            ),
        },
        ToolSpec {
            name: "agent_teams_remove_member",
            description: "Remove a member safely: revoke its current attempts, return all unfinished owned tasks to the shared pending pool, interrupt its live turn, and mark it removed.",
            input_schema: object(
                json!({ "name": string("Name of the member to remove.") }),
                &["name"],
            ),
        },
        ToolSpec {
            name: "agent_teams_create_task",
            description: "Create a task in your team's task list. Use kind=work (default) for research, repository audits and general tasks. kind=review is only a quality gate for an existing implementation/repair task via reviewedTaskId. Every call must include a non-empty subject, including verification and review tasks. Tasks can depend on other tasks (dependencies): a task is only claimable once every dependency is completed. Optionally assign it to a member, who still claims it before working.",
            input_schema: object(
                json!({
                    "subject": string("Required non-empty title for this task. Never omit it, including for verification or review tasks."),
                    "description": string("What needs to be done, in detail."),
                    "dependencies": strings("Task ids this task depends on (must be completed before this task can be claimed)."),
                    "assignee": string("Member name when an owner is specified. Omission puts this task in the shared pool; roles, subjects and descriptions do not assign an owner."),
                    "kind": {
                        "type": "string",
                        "enum": ["work", "requirements", "implementation", "verification", "review", "repair", "integration"],
                        "description": "Explicit task kind. Omission means work with no quality gates, even if the subject says implementation/review. Quality kinds require a contract. An implementation may be planned before requirements passes when its dependency chain includes that requirements task.",
                    },
                    "round": { "type": "number", "description": "1-based review / requirements / repair round." },
                    "objective": string("Required non-empty objective for quality kinds."),
                    "inScope": strings("Workspace-relative POSIX paths this task may change."),
                    "outOfScope": strings("Workspace-relative POSIX paths this task must not change."),
                    "acceptance": strings("Acceptance criteria. Required for quality kinds."),
                    "verify": strings("Verification commands. Required for implementation/repair."),
                    "deliverables": strings("Expected deliverable paths or names."),
                    "nonGoals": strings("Explicit non-goals."),
                    "reviewedTaskId": string("Existing AgentTeams implementation/repair task id. Required for kind=review; an external repository audit is kind=work."),
                    "sourceTaskId": string("Source implementation/artifact. Required for kind=repair."),
                    "sourceFindingIds": strings("Finding ids this repair must close."),
                    "coverageOf": strings("User-constraint / goal items this task covers."),
                    "resume": { "type": "boolean", "description": "If true, clear halted in the same lock before creating the task." },
                    "resumeReason": string("Required non-empty reason when resume=true."),
                }),
                &["subject"],
            ),
        },
        ToolSpec {
            name: "agent_teams_reassign_task",
            description: "Atomically retry, reassign, or let the captain take over one ready unfinished/failed task. The old attempt is revoked before its member is interrupted, so late updates cannot overwrite the new owner. Use assignee=\"captain\" only when you will finish that task in this turn; a captain can own only one unfinished takeover at a time, and an unfinished takeover returns to the member pool when the captain becomes idle.",
            input_schema: object(
                json!({
                    "task_id": string("Task to retry/reassign."),
                    "assignee": string("Active member name, or \"captain\" for captain takeover."),
                    "reason": string("Why the task is being retried or reassigned."),
                }),
                &["task_id", "assignee"],
            ),
        },
        ToolSpec {
            name: "agent_teams_claim_task",
            description: "Members claim their own ready task or read their existing attempt_id. Captains must use reassign_task to assign and wake a member; claim_task does not dispatch work. A member cannot own a second unfinished task. The returned attempt_id is required for updates and becomes stale after retry/reassignment.",
            input_schema: object(
                json!({
                    "task_id": string("The task id to claim."),
                    "assignee": string("Deprecated: claim_task only supports a member claiming its own task. Captains must use reassign_task."),
                }),
                &["task_id"],
            ),
        },
        ToolSpec {
            name: "agent_teams_update_task",
            description: "Update a task status/output. Members must supply the current attempt_id returned by claim_task; stale attempts are rejected after takeover/reassignment. Terminal results are immutable, but owners and the captain can append acceptanceResults/commandsRun/evidence_note as attributed supplemental evidence, without reclaiming or changing the verdict. A captain must use reassign_task(assignee=\"captain\") before updating active member-owned work.",
            input_schema: object(
                json!({
                    "task_id": string("The task id to update."),
                    "attempt_id": string("Members must explicitly include the current attempt_id from their assignment/claim in EVERY update, including failed reviews with findings. If omitted, retry with the same current id; omission does not revoke the attempt."),
                    "status": {
                        "type": "string",
                        "enum": ["in_progress", "completed", "failed", "cancelled"],
                        "description": "New status (in_progress, completed, failed, cancelled).",
                    },
                    "output": string("Original result summary; immutable after completion/failure."),
                    "evidence_note": string("Append-only supplementary observation on a terminal task. Does not reopen work or change the original result."),
                    "verdict": {
                        "type": "string",
                        "enum": ["pass", "needs_revision", "reject"],
                        "description": "Required for completing requirements/review. needs_revision and reject must fail the task.",
                    },
                    "findings": {
                        "type": "array",
                        "description": "Structured review findings. Required when verdict is needs_revision or reject; each item needs id, severity, problem, and requiredFix.",
                        "items": object(json!({
                            "id": { "type": "string" },
                            "severity": { "type": "string", "enum": ["low", "medium", "high", "blocker"] },
                            "problem": { "type": "string" },
                            "requiredFix": { "type": "string" },
                            "file": { "type": "string" },
                            "line": { "type": "number" },
                            "resolved": { "type": "boolean" },
                        }), &["id", "severity", "problem", "requiredFix"]),
                    },
                    "changedPaths": strings("Workspace-relative POSIX paths changed by this implementation/repair."),
                    "acceptanceResults": {
                        "type": "array",
                        "description": "Acceptance evidence in contract order: {criterion, status:\"passed\"|\"failed\", evidence?}. Supply one item per acceptance criterion.",
                        "items": object(json!({
                            "criterion": { "type": "string" },
                            "status": { "type": "string", "enum": ["passed", "failed"] },
                            "evidence": { "type": "string" },
                        }), &["criterion", "status"]),
                    },
                    "commandsRun": {
                        "type": "array",
                        "description": "Verification evidence in contract order: {command, status:\"passed\"|\"failed\", exitCode?, evidence?}. Supply one item per verify command.",
                        "items": object(json!({
                            "command": { "type": "string" },
                            "status": { "type": "string", "enum": ["passed", "failed"] },
                            "exitCode": { "type": "number" },
                            "evidence": { "type": "string" },
                        }), &["command", "status"]),
                    },
                }),
                &["task_id"],
            ),
        },
        ToolSpec {
            name: "agent_teams_amend_task",
            description: "Captain-only controlled contract amendment for one non-terminal quality task: replace a wrong objective/acceptance/verify/inScope/outOfScope when the original contract makes honest completion impossible (for example a verify command that cannot pass, or an inScope that forbids the file the objective names). The amendment is appended to the task's revisions ledger with previous values and the reason, and is rejected once a review/requirements task has passed judgment on this task. Members cannot amend contracts; the implementer re-reads the amended contract before its next quality gate. Lists are full replacements, not deltas.",
            input_schema: object(
                json!({
                    "task_id": string("Task whose contract is being amended."),
                    "reason": string("Why the current contract is wrong; recorded in the revisions ledger."),
                    "objective": string("Replacement objective."),
                    "acceptance": strings("Replacement acceptance criteria (full list, not a delta)."),
                    "verify": strings("Replacement verification commands (full list, not a delta)."),
                    "inScope": strings("Replacement workspace-relative inScope paths (full list)."),
                    "outOfScope": strings("Replacement workspace-relative outOfScope paths (full list)."),
                }),
                &["task_id", "reason"],
            ),
        },
        ToolSpec {
            name: "agent_teams_send_message",
            description: "Send coordination or current-task guidance directly to the captain or a teammate. A running recipient receives it at the next model step; an idle recipient wakes. Messages are durably retained until read. Use task creation/reassignment for a new unit of work, not repeated status nudges.",
            input_schema: object(
                json!({
                    "source_task_id": string("Sender task, NOT the recipient task. Members include it with source_attempt_id. Captains sending guidance normally omit both source fields."),
                    "source_attempt_id": string("Sender execution capability paired with source_task_id. Stale reports are rejected. Omit for ordinary captain guidance."),
                    "to": string("Recipient: \"captain\" or a member name."),
                    "content": string("The message text."),
                    "from": string("Sender (defaults to the caller: the captain, or the calling member)."),
                }),
                &["to", "content"],
            ),
        },
        ToolSpec {
            name: "agent_teams_status",
            description: "Team snapshot: members with live activity and tasks with status/assignee/dependencies/output. Captains also see every team mailbox; members see only their own inbox. Use after mailbox progress deliveries or for an explicit status request. After dispatch, end your turn while members work; do not repeatedly poll.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolSpec {
            name: "agent_teams_resume",
            description: "Explicitly resume a halted team. Requires a non-empty reason. Does not recreate cancelled tasks; only still-pending work is scheduled.",
            input_schema: object(
                json!({ "reason": string("Why the team is being resumed.") }),
                &["reason"],
            ),
        },
        ToolSpec {
            name: "agent_teams_delete",
            description: "End and archive your team: interrupts members and moves the current tasks and mailboxes out of active state for later inspection. Use when the work is done or explicitly abandoned. A same-name archive replaces its previous generation.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::TEAM_TOOL_NAMES;

    #[test]
    fn every_tool_is_specified_in_order() {
        let names: Vec<&str> = tool_specs().iter().map(|spec| spec.name).collect();
        assert_eq!(names, TEAM_TOOL_NAMES);
    }

    #[test]
    fn schemas_keep_the_reference_argument_names() {
        let specs = tool_specs();
        let props = |name: &str| {
            specs
                .iter()
                .find(|spec| spec.name == name)
                .unwrap()
                .input_schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        assert!(props("agent_teams_update_task").contains(&"acceptanceResults".to_owned()));
        assert!(props("agent_teams_update_task").contains(&"evidence_note".to_owned()));
        assert!(props("agent_teams_add_member").contains(&"executionPrompt".to_owned()));
        assert!(props("agent_teams_add_member").contains(&"reasoning_effort".to_owned()));
        assert!(props("agent_teams_create_task").contains(&"resumeReason".to_owned()));
        for spec in &specs {
            assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
            assert!(!spec.description.contains("Web"), "{}", spec.name);
        }
    }
}
