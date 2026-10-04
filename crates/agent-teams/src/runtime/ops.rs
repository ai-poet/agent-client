// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The `agent_teams_*` operations and the Team panel's plan controls.
//!
//! Each operation re-reads the team under its lock, re-derives the caller's
//! identity from that fresh record, applies the rules, writes, and only then
//! wakes anyone — a member or the captain is never woken while the lock is
//! held, except for the lazy start in [`TeamRuntime::dispatch_member`],
//! which the host makes non-blocking.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};

use crate::key::{CAPTAIN_KEY, sanitize_key};
use crate::mailbox;
use crate::profiles::{NormalizedTeamProfile, normalize_team_profile_value, resolve_team_profile};
use crate::prompts::{
    staged_plan_approved_context, staged_plan_discard_context, staged_plan_feedback_context,
};
use crate::quality::{
    ContractAmendmentInput, CreateTaskInput, DEFAULT_REVIEW_OBJECTIVE, QualityCompletionUpdate,
    ResumeStatus, append_task_evidence, apply_quality_follow_up, build_coverage_matrix,
    can_declare_delivery, describe_quality_loop, evaluate_quality_completion,
    parse_acceptance_results, parse_command_results, parse_findings, resume_team_state,
    sanitize_review_acceptance, sanitize_review_objective, validate_create_task,
};
use crate::scheduler::{collect_completed_dependency_outputs, format_dependency_outputs};
use crate::transitions::{
    begin_task_attempt, cancel_unfinished_task, invalidate_task_attempt, transition_error,
    unsatisfied_dependencies,
};
use crate::types::{
    MemberStatus, PlanReviewState, ReviewVerdict, TaskKind, TaskStatus, TeamMember, TeamMessage,
    TeamPhase, TeamState, TeamTask, now_ms,
};

use super::args::{Args, sub};
use super::render::{
    StatusInboxItem, StatusInboxSummary, StatusMember, StatusProfile, StatusTask, StatusView,
};
use super::{
    CaptainRoute, DeliveryMode, Identity, MemberRoute, OpResult, RouteRequest, TeamRuntime,
    global_locks, require_member, require_member_mut, require_task, task_index, text,
    trimmed_optional,
};

// ---------------------------------------------------------------------------
// Member routes
// ---------------------------------------------------------------------------

/// Resolve one member's route against the captain's. Members inherit the
/// captain's route; the effort is inherited only when the route is the same,
/// because an effort id belongs to one model. `"default"` asks for the
/// target model's own default.
pub fn resolve_member_route(
    captain: &CaptainRoute,
    request: &RouteRequest,
    default_effort: Option<&str>,
) -> OpResult<MemberRoute> {
    let explicit = |value: &Option<String>, error: &str| -> OpResult<Option<String>> {
        match value.as_deref().map(str::trim) {
            None => Ok(None),
            Some("") => Err(error.to_owned()),
            Some(text) => Ok(Some(text.to_owned())),
        }
    };
    let mut provider = explicit(&request.provider, "member LLM provider must not be empty")?;
    let mut model = explicit(&request.model, "member model must not be empty")?;
    let default_model = explicit(
        &request.default_model,
        "configured memberModel must not be empty",
    )?;
    let effort = explicit(
        &request.reasoning_effort,
        "member reasoning effort must not be empty",
    )?;
    // A picker id (`openai::gpt-6-sol`) names its platform itself.
    if provider.is_none()
        && let Some((platform, bare)) = model.as_deref().and_then(|id| id.split_once("::"))
        && !platform.is_empty()
        && !bare.is_empty()
    {
        provider = Some(platform.to_owned());
        model = Some(bare.to_owned());
    }
    if provider.is_some() && model.is_none() {
        return Err("an explicit member LLM provider requires an explicit member model".to_owned());
    }
    let (default_provider, default_model) =
        match default_model.as_deref().and_then(|id| id.split_once("::")) {
            Some((platform, bare)) if !platform.is_empty() && !bare.is_empty() => {
                (Some(platform.to_owned()), Some(bare.to_owned()))
            }
            _ => (None, default_model),
        };
    let used_default = model.is_none() && default_model.is_some();
    let provider = provider
        .or(if used_default { default_provider } else { None })
        .or_else(|| captain.provider.clone());
    let model = model.or(default_model).or_else(|| captain.model.clone());
    let (Some(provider), Some(model)) = (provider, model) else {
        return Err(
            "cannot resolve the member LLM route from the current captain session".to_owned(),
        );
    };
    let same_route =
        Some(&provider) == captain.provider.as_ref() && Some(&model) == captain.model.as_ref();
    let reasoning_effort = match effort.as_deref() {
        Some("default") => None,
        Some(effort) => Some(effort.to_owned()),
        None if same_route => captain.reasoning_effort.clone(),
        None if used_default => default_effort.map(str::to_owned),
        None => None,
    };
    Ok(MemberRoute {
        provider,
        model,
        reasoning_effort,
        fallback: request.fallback.clone(),
    })
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct CreatedMember {
    pub member_name: String,
    pub member_id: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreatedTask {
    pub task_id: String,
    pub seed_id: String,
    pub subject: String,
    pub status: String,
    pub kind: Option<String>,
    pub assignee: Option<String>,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateResult {
    pub team_id: String,
    pub team_name: String,
    pub state_dir: String,
    pub phase: String,
    pub profile: Option<String>,
    pub task_planning: Option<String>,
    pub members: Option<Vec<CreatedMember>>,
    pub tasks: Option<Vec<CreatedTask>>,
}

impl CreateResult {
    pub fn render(&self) -> String {
        let head = if self.phase == "staged" {
            format!(
                "Team \"{}\" plan created under {}. It is staged: finish the roster and DAG, then wait for the user to edit and approve it. Do not start or approve it yourself.",
                self.team_name, self.state_dir
            )
        } else {
            format!(
                "Team \"{}\" created (id {}) under {}. You are the captain.",
                self.team_name, self.team_id, self.state_dir
            )
        };
        match &self.tasks {
            None => head,
            Some(tasks) => {
                let lines: Vec<String> = tasks
                    .iter()
                    .map(|task| {
                        format!(
                            "{} [{}]: {}; assignee={}; dependencies={}",
                            task.task_id,
                            task.seed_id,
                            task.subject,
                            task.assignee.as_deref().unwrap_or("unassigned"),
                            task.dependencies.join(",")
                        )
                    })
                    .collect();
                format!("{head}\n{}", lines.join("\n"))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditPlanResult {
    pub status: String,
    pub team_id: String,
    pub members: usize,
    pub tasks: usize,
    pub dependencies: usize,
    pub roster: Vec<String>,
    pub graph: Vec<String>,
}

impl EditPlanResult {
    pub fn render(&self) -> String {
        let staged = self.status == "staged";
        format!(
            "{} updated atomically ({} members, {} tasks, {} dependencies). {}\n{}",
            if staged {
                "Staged plan"
            } else {
                "Pending task graph"
            },
            self.members,
            self.tasks,
            self.dependencies,
            if staged {
                "No members were spawned and no tasks were scheduled."
            } else {
                "The scheduler will dispatch any newly ready work."
            },
            self.graph.join("\n")
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApproveResult {
    pub team_id: String,
    pub team_name: String,
    pub members: usize,
    pub tasks: usize,
}

impl ApproveResult {
    pub fn render(&self) -> String {
        format!(
            "Team {} approved and running ({} members, {} tasks).",
            self.team_id, self.members, self.tasks
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AddMemberResult {
    pub member_name: String,
    pub member_id: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub status: String,
    pub phase: String,
}

impl AddMemberResult {
    pub fn render(&self) -> String {
        if self.phase == "staged" {
            return format!(
                "Member \"{}\" added to the staged roster ({}/{}); no child was spawned.",
                self.member_name, self.provider, self.model
            );
        }
        let session = if self.member_id.is_empty() {
            "starts with first ready task"
        } else {
            &self.member_id
        };
        let effort = self
            .reasoning_effort
            .as_ref()
            .map(|effort| format!(", reasoning {effort}"))
            .unwrap_or_default();
        format!(
            "Member \"{}\" added (session {session}, {}/{}{effort}, status {}).",
            self.member_name, self.provider, self.model, self.status
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoveMemberResult {
    pub member_name: String,
    pub status: String,
    pub requeued_tasks: Vec<String>,
}

impl RemoveMemberResult {
    pub fn render(&self) -> String {
        let requeued = if self.requeued_tasks.is_empty() {
            "none".to_owned()
        } else {
            self.requeued_tasks.join(", ")
        };
        format!(
            "Member \"{}\" removed (status {}); requeued tasks: {requeued}.",
            self.member_name, self.status
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTaskResult {
    pub task_id: String,
    pub subject: String,
    pub status: String,
    pub kind: String,
    pub assignee: Option<String>,
}

impl CreateTaskResult {
    pub fn render(&self) -> String {
        let owner = match &self.assignee {
            Some(name) if !name.is_empty() => format!("assigned to {name}"),
            _ => "unassigned shared pool".to_owned(),
        };
        format!(
            "Task \"{}\" created as {} (status {}, kind {}, {owner}).",
            self.subject, self.task_id, self.status, self.kind
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReassignResult {
    pub task_id: String,
    pub previous_assignee: String,
    pub assignee: String,
    pub status: String,
    pub attempt: u64,
    pub attempt_id: Option<String>,
}

impl ReassignResult {
    pub fn render(&self) -> String {
        let previous = if self.previous_assignee.is_empty() {
            "unassigned"
        } else {
            &self.previous_assignee
        };
        let attempt_id = self
            .attempt_id
            .as_ref()
            .map(|id| format!(", attempt_id {id}"))
            .unwrap_or_default();
        format!(
            "Task {} reassigned {previous} → {} (attempt {}, status {}{attempt_id}).",
            self.task_id, self.assignee, self.attempt, self.status
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClaimResult {
    pub task_id: String,
    pub status: String,
    pub assignee: String,
    pub attempt: u64,
    pub attempt_id: Option<String>,
    pub task_details: String,
}

impl ClaimResult {
    pub fn render(&self) -> String {
        let attempt_id = self
            .attempt_id
            .as_ref()
            .map(|id| format!(", attempt_id {id}"))
            .unwrap_or_default();
        format!(
            "Task {} claimed by {} (attempt {}{attempt_id}, status {}).\n{}",
            self.task_id, self.assignee, self.attempt, self.status, self.task_details
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateTaskResult {
    pub task_id: String,
    pub status: String,
    pub output: Option<String>,
    pub attempt: u64,
    pub attempt_id: Option<String>,
    pub evidence_count: Option<usize>,
    pub follow_up: Option<String>,
}

impl UpdateTaskResult {
    pub fn render(&self) -> String {
        let mut text = format!(
            "Task {} attempt {} → {}",
            self.task_id, self.attempt, self.status
        );
        if let Some(output) = &self.output {
            text.push_str(&format!("\nOutput: {output}"));
        }
        if let Some(count) = self.evidence_count {
            text.push_str(&format!(
                "\nSupplemental evidence records: {count}. Original result unchanged."
            ));
        }
        if let Some(follow_up) = self.follow_up.as_ref().filter(|text| !text.is_empty()) {
            text.push_str(&format!("\n{follow_up}"));
        }
        text
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmendResult {
    pub task_id: String,
    pub status: String,
    pub revised_fields: String,
    pub revision_count: usize,
    pub contract: String,
}

impl AmendResult {
    pub fn render(&self) -> String {
        format!(
            "Task {} contract amended ({}); {} revision(s) on record, status {}. New contract: {}",
            self.task_id, self.revised_fields, self.revision_count, self.status, self.contract
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SendMessageResult {
    pub message_id: String,
    pub from: String,
    pub to: String,
    pub delivered: String,
}

impl SendMessageResult {
    pub fn render(&self) -> String {
        format!(
            "Message {} {} → {} delivered via {}.",
            self.message_id, self.from, self.to, self.delivered
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResumeResult {
    pub status: String,
    pub team_id: String,
    pub reason: String,
}

impl ResumeResult {
    pub fn render(&self) -> String {
        if self.status == "already_running" {
            format!("Team {} is already running.", self.team_id)
        } else {
            format!("Team {} resumed ({}).", self.team_id, self.reason)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeleteResult {
    pub team_name: String,
}

impl DeleteResult {
    pub fn render(&self) -> String {
        format!("Team \"{}\" ended and archived.", self.team_name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HaltResult {
    pub team_name: String,
    pub cancelled_tasks: usize,
    pub already_halted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContinueResult {
    pub team_id: String,
    pub already_waiting: bool,
}

// ---------------------------------------------------------------------------
// Plan edits
// ---------------------------------------------------------------------------

/// One edit of a plan (`agent_teams_edit_plan` operation).
#[derive(Clone, Debug, PartialEq)]
pub enum PlanMutation {
    UpdateMember {
        member_name: String,
        role: Option<String>,
        provider: String,
        model: String,
        reasoning_effort: Option<String>,
        execution_prompt: Option<String>,
    },
    UpdateTask {
        task_id: String,
        subject: String,
        description: Option<String>,
        assignee: Option<String>,
        dependencies: Vec<String>,
    },
    AddTask {
        subject: String,
        description: Option<String>,
        assignee: Option<String>,
        dependencies: Vec<String>,
    },
    RemoveTask {
        task_id: String,
    },
    RemoveMember {
        member_name: String,
    },
}

impl PlanMutation {
    fn action(&self) -> &'static str {
        match self {
            PlanMutation::UpdateMember { .. } => "update_member",
            PlanMutation::UpdateTask { .. } => "update_task",
            PlanMutation::AddTask { .. } => "add_task",
            PlanMutation::RemoveTask { .. } => "remove_task",
            PlanMutation::RemoveMember { .. } => "remove_member",
        }
    }
}

fn dedup_trimmed(items: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty() && seen.insert(item.clone()))
        .collect()
}

fn require_staged(team: &TeamState) -> OpResult<()> {
    if !team.is_staged() {
        return Err(format!(
            "team \"{}\" is already running; its plan can no longer be edited",
            team.name
        ));
    }
    if team.is_halted() {
        return Err(format!(
            "team \"{}\" is halted, not awaiting plan approval",
            team.name
        ));
    }
    Ok(())
}

/// References and cycles, before a staged graph can be saved or run.
pub fn validate_staged_graph(team: &TeamState, require_runnable: bool) -> OpResult<()> {
    let members: Vec<&TeamMember> = team.active_members().collect();
    if require_runnable && members.is_empty() {
        return Err("add at least one member before approving the plan".to_owned());
    }
    if require_runnable && team.tasks.is_empty() {
        return Err("add at least one task before approving the plan".to_owned());
    }
    let member_names: HashSet<&str> = members.iter().map(|member| member.name.as_str()).collect();
    let task_ids: HashSet<&str> = team.tasks.iter().map(|task| task.id.as_str()).collect();
    for task in &team.tasks {
        if task.subject.trim().is_empty() {
            return Err(format!("task \"{}\" must have a subject", task.id));
        }
        if let Some(assignee) = &task.assignee
            && assignee != CAPTAIN_KEY
            && !member_names.contains(assignee.as_str())
        {
            return Err(format!(
                "task \"{}\" assignee \"{assignee}\" is not an active member",
                task.id
            ));
        }
        for dependency in &task.dependencies {
            if dependency == &task.id {
                return Err(format!("task \"{}\" cannot depend on itself", task.id));
            }
            if !task_ids.contains(dependency.as_str()) {
                return Err(format!(
                    "task \"{}\" depends on unknown task \"{dependency}\"",
                    task.id
                ));
            }
        }
    }
    fn visit<'a>(
        id: &'a str,
        team: &'a TeamState,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> OpResult<()> {
        if visiting.contains(id) {
            return Err(format!(
                "task dependency graph contains a cycle at \"{id}\""
            ));
        }
        if visited.contains(id) {
            return Ok(());
        }
        visiting.insert(id);
        if let Some(task) = team.task(id) {
            for dependency in &task.dependencies {
                visit(dependency, team, visiting, visited)?;
            }
        }
        visiting.remove(id);
        visited.insert(id);
        Ok(())
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for task in &team.tasks {
        visit(&task.id, team, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn member_open_task<'a>(
    team: &'a TeamState,
    member_name: &str,
    except: Option<&str>,
) -> Option<&'a TeamTask> {
    team.tasks.iter().find(|task| {
        Some(task.id.as_str()) != except
            && task.assignee.as_deref() == Some(member_name)
            && task.status.is_open_attempt()
    })
}

fn captain_open_task<'a>(team: &'a TeamState, except: Option<&str>) -> Option<&'a TeamTask> {
    team.tasks.iter().find(|task| {
        Some(task.id.as_str()) != except
            && task.assignee.as_deref() == Some(CAPTAIN_KEY)
            && !task.status.is_terminal()
    })
}

fn task_details(team: &TeamState, task: &TeamTask) -> String {
    let list = |items: &Option<Vec<String>>, separator: &str| {
        items.as_deref().unwrap_or_default().join(separator)
    };
    [
        task.subject.clone(),
        task.description.clone().unwrap_or_default(),
        format!("Kind: {}", task.kind_or_work()),
        format!("Objective: {}", task.objective.clone().unwrap_or_default()),
        format!(
            "In scope: {}; Out of scope: {}",
            list(&task.in_scope, ", "),
            list(&task.out_of_scope, ", ")
        ),
        format!("Acceptance: {}", list(&task.acceptance, "; ")),
        format!("Verify: {}", list(&task.verify, "; ")),
        format!(
            "Dependency results:\n{}",
            format_dependency_outputs(&collect_completed_dependency_outputs(&team.tasks, &task.id))
        ),
    ]
    .join("\n")
}

fn edit_plan_summary(team: &TeamState) -> EditPlanResult {
    EditPlanResult {
        status: team.phase_or_running().as_str().to_owned(),
        team_id: team.id.clone(),
        members: team.members.len(),
        tasks: team.tasks.len(),
        dependencies: team.tasks.iter().map(|task| task.dependencies.len()).sum(),
        roster: team
            .members
            .iter()
            .map(|member| {
                format!(
                    "{} ({}; {}/{})",
                    member.name,
                    member
                        .role
                        .as_deref()
                        .filter(|role| !role.is_empty())
                        .unwrap_or("member"),
                    member.provider.as_deref().unwrap_or_default(),
                    member.model.as_deref().unwrap_or_default()
                )
            })
            .collect(),
        graph: team
            .tasks
            .iter()
            .map(|task| {
                let owner = task
                    .assignee
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("shared");
                let deps = if task.dependencies.is_empty() {
                    String::new()
                } else {
                    format!("; depends on {}", task.dependencies.join(", "))
                };
                format!("{}: {} -> {owner}{deps}", task.id, task.subject)
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

impl TeamRuntime {
    fn resolve_route(&self, request: &RouteRequest) -> OpResult<MemberRoute> {
        resolve_member_route(
            &self.host.captain_route(),
            request,
            self.config.member_reasoning_effort.as_deref(),
        )
    }

    fn validate_routes(&self, routes: &[MemberRoute]) -> OpResult<()> {
        for route in routes {
            self.host.validate_route(route)?;
        }
        Ok(())
    }

    fn state_dir_of(&self, team_id: &str) -> String {
        self.root.team_dir(team_id).display().to_string()
    }

    /// `agent_teams_create`.
    pub fn create(&self, caller: &str, input: &Value) -> OpResult<CreateResult> {
        let args = Args::new("agent_teams_create", input)?;
        let team_name = args.req_str("name")?.trim().to_owned();
        if team_name.is_empty() {
            return Err("team name must not be empty".to_owned());
        }
        let description = args.opt_str("description")?;
        let profile_name = trimmed_optional(args.opt_str("profile")?.as_deref());
        let plan = args.raw("plan").cloned();
        if plan.as_ref().is_some_and(|plan| !plan.is_object()) {
            return Err("agent_teams_create: \"plan\" must be an object".to_owned());
        }
        let approval = args.opt_enum("approval", &["required", "automatic"])?;
        let staged = approval.as_deref() == Some("required");
        if profile_name.is_some() && plan.is_some() {
            return Err("choose either a configured profile or an inline plan".to_owned());
        }
        let team_id = sanitize_key(&team_name);

        let created = global_locks().with(&self.captain_lock_key(), || -> OpResult<TeamState> {
            if let Some(current) = self.root.find_team_by_participant(caller).map_err(text)? {
                let leads = current.captain_session_id == caller;
                let (relationship, guidance) = if leads {
                    ("lead", "Use agent_teams_status and continue the existing team. Do not delete and recreate it merely to continue work. End it only when the user explicitly wants a separate new team.")
                } else {
                    ("belong to", "Continue your assigned member work and report to your captain; do not create a separate team.")
                };
                return Err(format!(
                    "you already {relationship} team \"{}\" (id {}). {guidance}",
                    current.name, current.id
                ));
            }
            if caller != self.captain_id() {
                return Err("only the captain session can create a team".to_owned());
            }
            self.with_team_lock(&team_id, || {
                if self.root.read_team(&team_id).map_err(text)?.is_some() {
                    return Err(format!(
                        "team id \"{team_id}\" is taken by another captain — pick a different team name"
                    ));
                }
                let mut state = TeamState::new(&team_name, &team_id, caller, now_ms());
                state.description = description.clone();
                if staged {
                    state.phase = Some(TeamPhase::Staged);
                    state.plan_review_state = Some(PlanReviewState::AwaitingReview);
                }
                if profile_name.is_some() || plan.is_some() {
                    let profile = match (&profile_name, &plan) {
                        (Some(name), _) => resolve_team_profile(&self.config.profiles, name, self.config.max_members)?,
                        (None, Some(plan)) => {
                            normalize_team_profile_value("inline-plan", plan, self.config.max_members)?
                        }
                        (None, None) => unreachable!(),
                    };
                    self.expand_profile(&mut state, &profile, plan.is_some())?;
                }
                self.root.create_team_dir(&state).map_err(text)?;
                self.host.team_changed(&state.id);
                Ok(state)
            })
        })?;
        self.kick_team(&created.id);

        let snapshot = self
            .root
            .read_team(&created.id)
            .ok()
            .flatten()
            .unwrap_or(created);
        let state_dir = self.state_dir_of(&snapshot.id);
        let phase = snapshot.phase_or_running().as_str().to_owned();
        if snapshot.profile.is_none() && plan.is_none() {
            return Ok(CreateResult {
                team_id: snapshot.id,
                team_name: snapshot.name,
                state_dir,
                phase,
                profile: None,
                task_planning: None,
                members: None,
                tasks: None,
            });
        }
        Ok(CreateResult {
            profile: snapshot
                .profile
                .as_ref()
                .map(|profile| profile.name.clone()),
            task_planning: Some(
                snapshot
                    .profile
                    .as_ref()
                    .and_then(|profile| profile.task_planning)
                    .map(|planning| planning.as_str())
                    .unwrap_or("seed")
                    .to_owned(),
            ),
            members: Some(
                snapshot
                    .members
                    .iter()
                    .map(|member| CreatedMember {
                        member_name: member.name.clone(),
                        member_id: member.id.clone(),
                        provider: member.provider.clone().unwrap_or_default(),
                        model: member.model.clone().unwrap_or_default(),
                        reasoning_effort: member.reasoning_effort.clone(),
                        status: member.status.as_str().to_owned(),
                    })
                    .collect(),
            ),
            tasks: Some(
                snapshot
                    .tasks
                    .iter()
                    .map(|task| CreatedTask {
                        task_id: task.id.clone(),
                        seed_id: task.profile_seed_id.clone().unwrap_or_default(),
                        subject: task.subject.clone(),
                        status: task.status.as_str().to_owned(),
                        kind: task.kind.map(|kind| kind.as_str().to_owned()),
                        assignee: task.assignee.clone(),
                        dependencies: task.dependencies.clone(),
                    })
                    .collect(),
            ),
            team_id: snapshot.id,
            team_name: snapshot.name,
            state_dir,
            phase,
        })
    }

    /// Expand a profile (or inline plan) into the draft: roster with routes,
    /// seed tasks in topological order with ids `t1..tN`.
    fn expand_profile(
        &self,
        draft: &mut TeamState,
        profile: &NormalizedTeamProfile,
        inline: bool,
    ) -> OpResult<()> {
        let mut routes = Vec::with_capacity(profile.members.len());
        for template in &profile.members {
            routes.push(
                self.resolve_route(&RouteRequest {
                    provider: template.provider.clone(),
                    model: template.model.clone(),
                    default_model: self.config.member_model.clone(),
                    reasoning_effort: template.reasoning_effort.clone(),
                    fallback: template
                        .fallback
                        .clone()
                        .or_else(|| profile.snapshot.fallback.clone())
                        .or_else(|| self.config.fallback.clone()),
                })?,
            );
        }
        self.validate_routes(&routes)?;
        let now = now_ms();
        let seed_to_actual: BTreeMap<&str, String> = profile
            .tasks
            .iter()
            .enumerate()
            .map(|(index, template)| (template.id.as_str(), format!("t{}", index + 1)))
            .collect();
        if !inline {
            draft.profile = Some(profile.snapshot.clone());
            draft.review_policy = profile.snapshot.review_policy.clone();
        }
        draft.members = profile
            .members
            .iter()
            .zip(routes)
            .map(|(template, route)| {
                let mut member = TeamMember::new(&template.name, now);
                member.role = template.role.clone();
                member.provider = Some(route.provider);
                member.model = Some(route.model);
                member.reasoning_effort = route.reasoning_effort;
                member.execution_prompt = template
                    .execution_prompt
                    .clone()
                    .or_else(|| profile.snapshot.execution_prompt.clone())
                    .or_else(|| self.config.execution_prompt.clone());
                member.fallback = route.fallback;
                member
            })
            .collect();
        draft.tasks = profile
            .tasks
            .iter()
            .enumerate()
            .map(|(index, template)| {
                let mut task = TeamTask::new(format!("t{}", index + 1), &template.subject, now);
                task.profile_seed_id = Some(template.id.clone());
                task.description = template.description.clone();
                task.assignee = template.assignee.clone();
                task.dependencies = template
                    .dependencies
                    .iter()
                    .map(|dependency| {
                        seed_to_actual
                            .get(dependency.as_str())
                            .cloned()
                            .unwrap_or_else(|| dependency.clone())
                    })
                    .collect();
                task.attempt = Some(0);
                task
            })
            .collect();
        draft.task_seq = profile.tasks.len() as u64;
        Ok(())
    }

    /// Apply a batch of plan edits atomically. `allow_pending_edits` lets a
    /// running team correct never-started tasks (the tool); the Team panel
    /// only edits staged plans.
    pub fn update_plan_batch(
        &self,
        team_id: &str,
        mutations: &[PlanMutation],
        allow_pending_edits: bool,
    ) -> OpResult<TeamState> {
        if mutations.is_empty() {
            return Err("at least one staged plan operation is required".to_owned());
        }
        self.with_team_lock(team_id, || {
            let mut fresh = self.require_fresh_captain(team_id)?;
            let staged = fresh.is_staged();
            if !staged && !allow_pending_edits {
                require_staged(&fresh)?;
            }
            if fresh.is_halted() {
                return Err("team is halted; resume before editing tasks".to_owned());
            }
            if !staged && mutations.iter().any(|mutation| mutation.action() != "update_task") {
                return Err("a running team only permits update_task edits to pending, never-started tasks; roster and removal edits require a staged plan".to_owned());
            }
            for mutation in mutations {
                match mutation {
                    PlanMutation::UpdateMember {
                        member_name,
                        role,
                        provider,
                        model,
                        reasoning_effort,
                        execution_prompt,
                    } => {
                        let fallback = {
                            let member = require_member(&fresh, member_name)?;
                            if member.is_spawned() {
                                return Err(format!(
                                    "staged member \"{}\" was already spawned",
                                    member.name
                                ));
                            }
                            member.fallback.clone()
                        };
                        let route = self.resolve_route(&RouteRequest {
                            provider: Some(provider.clone()),
                            model: Some(model.clone()),
                            default_model: None,
                            reasoning_effort: trimmed_optional(reasoning_effort.as_deref()),
                            fallback,
                        })?;
                        let member = require_member_mut(&mut fresh, member_name)?;
                        member.role = trimmed_optional(role.as_deref());
                        member.provider = Some(route.provider);
                        member.model = Some(route.model);
                        member.reasoning_effort = route.reasoning_effort;
                        member.execution_prompt = trimmed_optional(execution_prompt.as_deref());
                    }
                    PlanMutation::UpdateTask {
                        task_id,
                        subject,
                        description,
                        assignee,
                        dependencies,
                    } => {
                        let index = task_index(&fresh, task_id)?;
                        let task = &mut fresh.tasks[index];
                        if task.status != TaskStatus::Pending
                            || task.attempt_number() != 0
                            || task.is_reassigning()
                        {
                            return Err(format!(
                                "task \"{}\" has already started and cannot be edited",
                                task.id
                            ));
                        }
                        if !staged && assignee.as_deref() == Some(CAPTAIN_KEY) {
                            return Err("use reassign_task for captain takeover".to_owned());
                        }
                        let subject = subject.trim();
                        if subject.is_empty() {
                            return Err("task subject must not be empty".to_owned());
                        }
                        task.subject = subject.to_owned();
                        task.description = trimmed_optional(description.as_deref());
                        task.assignee = trimmed_optional(assignee.as_deref());
                        task.dependencies = dedup_trimmed(dependencies);
                        task.updated_at = now_ms();
                    }
                    PlanMutation::AddTask {
                        subject,
                        description,
                        assignee,
                        dependencies,
                    } => {
                        let subject = subject.trim();
                        if subject.is_empty() {
                            return Err("task subject must not be empty".to_owned());
                        }
                        fresh.task_seq += 1;
                        let mut task = TeamTask::new(format!("t{}", fresh.task_seq), subject, now_ms());
                        task.description = trimmed_optional(description.as_deref());
                        task.assignee = trimmed_optional(assignee.as_deref());
                        task.dependencies = dedup_trimmed(dependencies);
                        task.attempt = Some(0);
                        task.kind = Some(TaskKind::Work);
                        fresh.tasks.push(task);
                    }
                    PlanMutation::RemoveTask { task_id } => {
                        let task = require_task(&fresh, task_id)?;
                        let id = task.id.clone();
                        if let Some(dependent) = fresh
                            .tasks
                            .iter()
                            .find(|candidate| candidate.dependencies.contains(&id))
                        {
                            return Err(format!(
                                "task \"{id}\" is still required by \"{}\"; update that dependency before removing the task",
                                dependent.id
                            ));
                        }
                        fresh.tasks.retain(|candidate| candidate.id != id);
                    }
                    PlanMutation::RemoveMember { member_name } => {
                        let member = require_member(&fresh, member_name)?;
                        if member.is_spawned() {
                            return Err(format!(
                                "staged member \"{}\" was already spawned",
                                member.name
                            ));
                        }
                        let name = member.name.clone();
                        let owned: Vec<&str> = fresh
                            .tasks
                            .iter()
                            .filter(|task| task.assignee.as_deref() == Some(name.as_str()))
                            .map(|task| task.id.as_str())
                            .collect();
                        if !owned.is_empty() {
                            return Err(format!(
                                "member \"{name}\" still owns planned tasks: {}; update or remove those tasks first",
                                owned.join(", ")
                            ));
                        }
                        fresh.members.retain(|candidate| candidate.name != name);
                    }
                }
            }
            validate_staged_graph(&fresh, false)?;
            if staged {
                fresh.plan_review_state = Some(PlanReviewState::AwaitingReview);
            } else {
                for mutation in mutations {
                    let PlanMutation::UpdateTask { task_id, .. } = mutation else {
                        continue;
                    };
                    let task = require_task(&fresh, task_id)?.clone();
                    let mut others = fresh.clone();
                    others.tasks.retain(|item| item.id != task.id);
                    validate_create_task(&others, &CreateTaskInput::from(&task))?;
                }
            }
            self.write(&fresh)?;
            Ok(fresh)
        })
    }

    /// `agent_teams_edit_plan`.
    pub fn edit_plan(&self, caller: &str, input: &Value) -> OpResult<EditPlanResult> {
        let args = Args::new("agent_teams_edit_plan", input)?;
        let team = self.require_captain_team(caller)?;
        let operations = args
            .opt_objects("operations")?
            .ok_or_else(|| "agent_teams_edit_plan: \"operations\" is required".to_owned())?;
        if operations.is_empty() {
            return Err("at least one staged plan operation is required".to_owned());
        }
        let mut mutations = Vec::with_capacity(operations.len());
        for (index, operation) in operations.iter().enumerate() {
            let op = sub("agent_teams_edit_plan", operation);
            let action = op
                .opt_enum(
                    "action",
                    &[
                        "update_member",
                        "update_task",
                        "add_task",
                        "remove_task",
                        "remove_member",
                    ],
                )?
                .ok_or_else(|| "agent_teams_edit_plan: \"action\" is required".to_owned())?;
            let label = format!("operation {} ({action})", index + 1);
            let member_name = || -> OpResult<String> {
                let name = op
                    .opt_str("member_name")?
                    .map(|name| name.trim().to_owned())
                    .unwrap_or_default();
                if name.is_empty() {
                    return Err(format!("{label} requires member_name"));
                }
                Ok(name)
            };
            let task_id = || -> OpResult<String> {
                let id = op
                    .opt_str("task_id")?
                    .map(|id| id.trim().to_owned())
                    .unwrap_or_default();
                if id.is_empty() {
                    return Err(format!("{label} requires task_id"));
                }
                Ok(id)
            };
            mutations.push(match action.as_str() {
                "update_member" => {
                    let name = member_name()?;
                    let member = require_member(&team, &name)?;
                    let nonblank = |value: Option<String>| {
                        value
                            .map(|text| text.trim().to_owned())
                            .filter(|text| !text.is_empty())
                    };
                    PlanMutation::UpdateMember {
                        member_name: name,
                        role: op.opt_str("role")?.or_else(|| member.role.clone()),
                        provider: nonblank(op.opt_str("provider")?)
                            .or_else(|| member.provider.clone())
                            .unwrap_or_default(),
                        model: nonblank(op.opt_str("model")?)
                            .or_else(|| member.model.clone())
                            .unwrap_or_default(),
                        reasoning_effort: op
                            .opt_str("reasoning_effort")?
                            .or_else(|| member.reasoning_effort.clone()),
                        execution_prompt: op
                            .opt_str("execution_prompt")?
                            .or_else(|| member.execution_prompt.clone()),
                    }
                }
                "update_task" => {
                    let id = task_id()?;
                    let task = require_task(&team, &id)?;
                    PlanMutation::UpdateTask {
                        task_id: id,
                        subject: op
                            .opt_str("subject")?
                            .unwrap_or_else(|| task.subject.clone()),
                        description: op
                            .opt_str("description")?
                            .or_else(|| task.description.clone()),
                        assignee: op.opt_str("assignee")?.or_else(|| task.assignee.clone()),
                        dependencies: op
                            .opt_strings("dependencies")?
                            .unwrap_or_else(|| task.dependencies.clone()),
                    }
                }
                "add_task" => {
                    let subject = op
                        .opt_str("subject")?
                        .map(|text| text.trim().to_owned())
                        .unwrap_or_default();
                    if subject.is_empty() {
                        return Err(format!("{label} requires a non-empty subject"));
                    }
                    PlanMutation::AddTask {
                        subject,
                        description: op.opt_str("description")?,
                        assignee: op.opt_str("assignee")?,
                        dependencies: op.opt_strings("dependencies")?.unwrap_or_default(),
                    }
                }
                "remove_task" => PlanMutation::RemoveTask {
                    task_id: task_id()?,
                },
                _ => PlanMutation::RemoveMember {
                    member_name: member_name()?,
                },
            });
        }
        let updated = self.update_plan_batch(&team.id, &mutations, true)?;
        if !updated.is_staged() {
            self.kick_team(&team.id);
        }
        Ok(edit_plan_summary(&updated))
    }

    /// `agent_teams_approve`.
    pub fn approve(&self, caller: &str, input: &Value) -> OpResult<ApproveResult> {
        let args = Args::new("agent_teams_approve", input)?;
        if args.req_str("confirmation")?.trim().is_empty() {
            return Err("explicit user approval text is required".to_owned());
        }
        let team = self.require_captain_team(caller)?;
        self.approve_staged(&team.id)
    }

    /// Approve a staged plan: drop removed placeholders, check the graph and
    /// every member route, and start running.
    pub fn approve_staged(&self, team_id: &str) -> OpResult<ApproveResult> {
        let approved = self.with_team_lock(team_id, || -> OpResult<ApproveResult> {
            let mut fresh = self.require_fresh_captain(team_id)?;
            require_staged(&fresh)?;
            fresh.members.retain(|member| !member.is_removed());
            validate_staged_graph(&fresh, true)?;
            let mut routes = Vec::with_capacity(fresh.members.len());
            for member in fresh.members.iter_mut() {
                let route = self.resolve_route(&RouteRequest {
                    provider: member.provider.clone(),
                    model: member.model.clone(),
                    default_model: None,
                    reasoning_effort: member.reasoning_effort.clone(),
                    fallback: member.fallback.clone(),
                })?;
                member.provider = Some(route.provider.clone());
                member.model = Some(route.model.clone());
                member.reasoning_effort = route.reasoning_effort.clone();
                routes.push(route);
            }
            self.validate_routes(&routes)?;
            fresh.phase = Some(TeamPhase::Running);
            fresh.plan_review_state = None;
            fresh.approved_at = Some(now_ms());
            self.write(&fresh)?;
            Ok(ApproveResult {
                team_id: fresh.id.clone(),
                team_name: fresh.name.clone(),
                members: fresh.members.len(),
                tasks: fresh.tasks.len(),
            })
        })?;
        self.kick_team(team_id);
        Ok(approved)
    }

    /// The Team panel's "Approve & run": approve, then tell the captain,
    /// whose conversation has no tool result for it.
    pub fn approve_from_panel(&self, team_id: &str) -> OpResult<ApproveResult> {
        let approved = self.approve_staged(team_id)?;
        if !self
            .host
            .steer_captain(&staged_plan_approved_context(&approved.team_name))
        {
            tracing::warn!("agent-teams: approval notification failed for {team_id}");
        }
        Ok(approved)
    }

    /// The Team panel's "Return to chat and revise".
    pub fn continue_staged_planning(&self, team_id: &str) -> OpResult<ContinueResult> {
        let prepared = self.with_team_lock(team_id, || -> OpResult<(String, bool)> {
            let mut fresh = self.require_fresh_captain(team_id)?;
            require_staged(&fresh)?;
            if fresh.plan_review_state == Some(PlanReviewState::AwaitingFeedback) {
                return Ok((fresh.name.clone(), true));
            }
            fresh.plan_review_state = Some(PlanReviewState::AwaitingFeedback);
            self.write(&fresh)?;
            Ok((fresh.name.clone(), false))
        })?;
        let (team_name, already_waiting) = prepared;
        if already_waiting {
            return Ok(ContinueResult {
                team_id: team_id.to_owned(),
                already_waiting: true,
            });
        }
        // End a planning turn still producing tool calls; the follow-up runs
        // after it, so it cannot race ahead and recreate the team.
        self.host.cancel_captain();
        if !self
            .host
            .followup_captain(&staged_plan_feedback_context(&team_name))
        {
            self.with_team_lock(team_id, || -> OpResult<()> {
                let mut fresh = self.require_fresh_captain(team_id)?;
                if fresh.is_staged()
                    && fresh.plan_review_state == Some(PlanReviewState::AwaitingFeedback)
                {
                    fresh.plan_review_state = Some(PlanReviewState::AwaitingReview);
                    self.write(&fresh)?;
                }
                Ok(())
            })?;
            return Err("the captain could not take the revision request".to_owned());
        }
        Ok(ContinueResult {
            team_id: team_id.to_owned(),
            already_waiting: false,
        })
    }

    /// The Team panel's "Discard plan": archive the draft, keep the decision
    /// for the next user turn, and stop the captain's planning turn.
    pub fn discard_staged(&self, team_id: &str) -> OpResult<String> {
        let team_name = self.with_team_lock(team_id, || -> OpResult<String> {
            let fresh = self.require_fresh_captain(team_id)?;
            require_staged(&fresh)?;
            self.root.archive_team_dir(&fresh.id).map_err(text)?;
            self.host.team_changed(&fresh.id);
            Ok(fresh.name)
        })?;
        self.host
            .park_captain_context(&staged_plan_discard_context(&team_name));
        self.host.cancel_captain();
        Ok(team_name)
    }

    /// Stop a team from the Team panel: unfinished work is cancelled, members
    /// are stopped, and the captain's turn ends.
    pub fn halt(&self, team_id: &str) -> OpResult<HaltResult> {
        let (result, member_ids) =
            self.with_team_lock(team_id, || -> OpResult<(HaltResult, Vec<String>)> {
                let mut fresh = self.require_fresh_captain(team_id)?;
                let member_ids: Vec<String> = fresh
                    .members
                    .iter()
                    .filter(|member| member.is_spawned() && !member.is_removed())
                    .map(|member| member.id.clone())
                    .collect();
                if fresh.is_halted() {
                    let cancelled = fresh
                        .tasks
                        .iter()
                        .filter(|task| task.status == TaskStatus::Cancelled)
                        .count();
                    return Ok((
                        HaltResult {
                            team_name: fresh.name.clone(),
                            cancelled_tasks: cancelled,
                            already_halted: true,
                        },
                        member_ids,
                    ));
                }
                let mut cancelled = 0;
                for task in fresh.tasks.iter_mut() {
                    if task.status.is_terminal() {
                        continue;
                    }
                    cancel_unfinished_task(task, Some("Stopped from the captain chat."));
                    cancelled += 1;
                }
                for member in fresh.members.iter_mut() {
                    if !member.is_removed() {
                        member.status = MemberStatus::Idle;
                    }
                }
                fresh.halted = Some(true);
                fresh.halted_at = Some(now_ms());
                self.write(&fresh)?;
                Ok((
                    HaltResult {
                        team_name: fresh.name.clone(),
                        cancelled_tasks: cancelled,
                        already_halted: false,
                    },
                    member_ids,
                ))
            })?;
        // Persist the stop first, then stop the captain before draining, so its
        // running turn cannot observe the halt and resume it.
        self.host.cancel_captain();
        let drained = self.host.drain_members(&member_ids);
        // A member stopping can wake the captain once more; close again.
        self.host.cancel_captain();
        drained?;
        Ok(result)
    }

    /// `agent_teams_add_member`.
    pub fn add_member(&self, caller: &str, input: &Value) -> OpResult<AddMemberResult> {
        let args = Args::new("agent_teams_add_member", input)?;
        let raw_name = args.req_str("name")?;
        let role = args.opt_str("role")?;
        let provider = args.opt_str("provider")?;
        let model = args.opt_str("model")?;
        let effort = args.opt_str("reasoning_effort")?;
        let execution_prompt = args.opt_str("executionPrompt")?;
        let team = self.require_captain_team(caller)?;
        let created = self.with_team_lock(&team.id, || {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let member_name = raw_name.trim().to_owned();
            if member_name.is_empty() {
                return Err("member name must not be empty".to_owned());
            }
            let key = sanitize_key(&member_name);
            if key == CAPTAIN_KEY {
                return Err(format!(
                    "member name \"{raw_name}\" is reserved for the captain"
                ));
            }
            if fresh
                .members
                .iter()
                .any(|member| sanitize_key(&member.name) == key)
            {
                return Err(format!(
                    "member name \"{raw_name}\" has already been used in team \"{}\"",
                    fresh.name
                ));
            }
            if fresh.active_members().count() >= self.config.max_members {
                return Err(format!(
                    "team \"{}\" is at its member cap ({})",
                    fresh.name, self.config.max_members
                ));
            }
            let route = self.resolve_route(&RouteRequest {
                provider: provider.clone(),
                model: model.clone(),
                default_model: self.config.member_model.clone(),
                reasoning_effort: effort.clone(),
                fallback: self.config.fallback.clone(),
            })?;
            self.validate_routes(std::slice::from_ref(&route))?;
            let mut member = TeamMember::new(&member_name, now_ms());
            member.role = role.clone();
            member.provider = Some(route.provider.clone());
            member.model = Some(route.model.clone());
            member.reasoning_effort = route.reasoning_effort.clone();
            member.fallback = route.fallback.clone();
            member.execution_prompt = trimmed_optional(execution_prompt.as_deref());
            fresh.members.push(member);
            self.write(&fresh)?;
            Ok(AddMemberResult {
                member_name,
                member_id: String::new(),
                provider: route.provider,
                model: route.model,
                reasoning_effort: route.reasoning_effort,
                status: MemberStatus::Idle.as_str().to_owned(),
                phase: fresh.phase_or_running().as_str().to_owned(),
            })
        })?;
        self.kick_member(&team.id, &created.member_name);
        let latest = self
            .root
            .read_team(&team.id)
            .ok()
            .flatten()
            .and_then(|team| team.member(&created.member_name).cloned());
        Ok(match latest {
            Some(member) => AddMemberResult {
                member_id: member.id,
                status: member.status.as_str().to_owned(),
                ..created
            },
            None => created,
        })
    }

    /// `agent_teams_remove_member`.
    pub fn remove_member(&self, caller: &str, input: &Value) -> OpResult<RemoveMemberResult> {
        let args = Args::new("agent_teams_remove_member", input)?;
        let name = args.req_str("name")?;
        let team = self.require_captain_team(caller)?;
        let (member, requeued) = self.with_team_lock(&team.id, || {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let Some(index) = fresh.members.iter().position(|member| member.name == name) else {
                return Err(format!("no member \"{name}\" in team \"{}\"", fresh.name));
            };
            let member_name = fresh.members[index].name.clone();
            let mut requeued = Vec::new();
            for task in fresh.tasks.iter_mut() {
                if task.assignee.as_deref() != Some(member_name.as_str())
                    || task.status == TaskStatus::Completed
                {
                    continue;
                }
                invalidate_task_attempt(task, None, false);
                requeued.push(task.id.clone());
            }
            fresh.members[index].status = MemberStatus::Removed;
            let unread = mailbox::read_unread(&self.root, &fresh.id, &member_name).map_err(text)?;
            mailbox::discard(
                &self.root,
                &fresh.id,
                &member_name,
                &mailbox::ids_of(&unread),
            )
            .map_err(text)?;
            self.write(&fresh)?;
            Ok((fresh.members[index].clone(), requeued))
        })?;
        if member.is_spawned() {
            global_locks()
                .with(&format!("retired:{}", self.root.lock_key()), || {
                    self.root.record_retired_member_ids([member.id.as_str()])
                })
                .map_err(text)?;
            self.host.drain_members(std::slice::from_ref(&member.id))?;
        }
        self.kick_team(&team.id);
        Ok(RemoveMemberResult {
            member_name: member.name,
            status: member.status.as_str().to_owned(),
            requeued_tasks: requeued,
        })
    }

    /// `agent_teams_create_task`.
    pub fn create_task(&self, caller: &str, input: &Value) -> OpResult<CreateTaskResult> {
        let args = Args::new("agent_teams_create_task", input)?;
        let mut normalized: Map<String, Value> = args.map().clone();
        crate::validate::normalize_blank_optional_task_fields(&mut normalized);
        let clean = Args::new("agent_teams_create_task", &Value::Object(normalized))?;
        let subject = args.req_str("subject")?;
        if args.opt_u32("round")? == Some(0) {
            return Err("round must be a 1-based positive integer".to_owned());
        }
        let kind = match clean.opt_enum(
            "kind",
            &[
                "work",
                "requirements",
                "implementation",
                "verification",
                "review",
                "repair",
                "integration",
            ],
        )? {
            Some(kind) => TaskKind::parse(&kind),
            None => None,
        };
        let request = CreateTaskInput {
            subject: subject.clone(),
            description: args.opt_str("description")?,
            dependencies: args.opt_strings("dependencies")?,
            assignee: args.opt_str("assignee")?,
            kind,
            round: args.opt_u32("round")?,
            objective: clean.opt_str("objective")?,
            in_scope: clean.opt_strings("inScope")?,
            out_of_scope: clean.opt_strings("outOfScope")?,
            acceptance: clean.opt_strings("acceptance")?,
            verify: clean.opt_strings("verify")?,
            deliverables: clean.opt_strings("deliverables")?,
            non_goals: clean.opt_strings("nonGoals")?,
            reviewed_task_id: clean.opt_str("reviewedTaskId")?,
            source_task_id: clean.opt_str("sourceTaskId")?,
            source_finding_ids: clean.opt_strings("sourceFindingIds")?,
            coverage_of: clean.opt_strings("coverageOf")?,
            resume: args.opt_bool("resume")?,
            resume_reason: args.opt_str("resumeReason")?,
        };
        let team = self.require_captain_team(caller)?;
        let created = self.with_team_lock(&team.id, || {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let gate = validate_create_task(&fresh, &request)?;
            if fresh.is_halted() {
                let resumed = resume_team_state(&mut fresh, request.resume_reason.as_deref().unwrap_or_default());
                if resumed.status != ResumeStatus::Resumed {
                    return Err(resumed.error.unwrap_or_else(|| {
                        "team is halted; call agent_teams_resume or pass resume=true with resumeReason".to_owned()
                    }));
                }
            }
            let dependencies = request.dependencies.clone().unwrap_or_default();
            for dependency in &dependencies {
                if fresh.task(dependency).is_none() {
                    return Err(format!(
                        "dependency \"{dependency}\" does not exist in team \"{}\"",
                        fresh.name
                    ));
                }
            }
            if let Some(assignee) = &request.assignee {
                require_member(&fresh, assignee)?;
            }
            let kind = gate.kind;
            let reviewish = matches!(kind, TaskKind::Review | TaskKind::Requirements);
            let objective = if reviewish {
                Some(sanitize_review_objective(request.objective.as_deref(), DEFAULT_REVIEW_OBJECTIVE))
            } else {
                request.objective.clone()
            };
            let acceptance = if reviewish {
                Some(sanitize_review_acceptance(request.acceptance.as_deref()))
            } else {
                request.acceptance.clone()
            };
            let now = now_ms();
            let mut task = TeamTask::new(format!("t{}", fresh.task_seq + 1), &subject, now);
            task.description = request.description.clone();
            task.assignee = request.assignee.clone();
            task.dependencies = dependencies;
            task.attempt = Some(0);
            task.kind = Some(kind);
            task.round = request.round;
            task.objective = objective;
            task.in_scope = request.in_scope.clone();
            task.out_of_scope = request.out_of_scope.clone();
            task.acceptance = acceptance;
            task.verify = request.verify.clone();
            task.deliverables = request.deliverables.clone();
            task.non_goals = request.non_goals.clone();
            task.reviewed_task_id = request.reviewed_task_id.clone();
            task.source_task_id = request.source_task_id.clone();
            task.source_finding_ids = request.source_finding_ids.clone();
            task.coverage_of = request.coverage_of.clone();
            fresh.task_seq += 1;
            let result = CreateTaskResult {
                task_id: task.id.clone(),
                subject: task.subject.clone(),
                status: task.status.as_str().to_owned(),
                kind: task.kind_or_work().as_str().to_owned(),
                assignee: task.assignee.clone(),
            };
            fresh.tasks.push(task);
            self.write(&fresh)?;
            Ok(result)
        })?;
        self.kick_team(&team.id);
        Ok(created)
    }

    /// `agent_teams_reassign_task`: revoke, quiesce the old owner, then hand
    /// over — two locked phases around the drain.
    pub fn reassign_task(&self, caller: &str, input: &Value) -> OpResult<ReassignResult> {
        let args = Args::new("agent_teams_reassign_task", input)?;
        let task_id = args.req_str("task_id")?;
        let target = args.req_str("assignee")?.trim().to_owned();
        let _reason = args.opt_str("reason")?;
        let team = self.require_captain_team(caller)?;
        if target.is_empty() {
            return Err("reassignment assignee must not be empty".to_owned());
        }
        struct Revoked {
            previous_assignee: String,
            previous_member: Option<TeamMember>,
            handoff_id: Option<String>,
        }
        let revoked = self.with_team_lock(&team.id, || -> OpResult<Revoked> {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let index = task_index(&fresh, &task_id)?;
            let task = &fresh.tasks[index];
            if task.status == TaskStatus::Completed {
                return Err(format!(
                    "completed task {} is immutable and cannot be reassigned",
                    task.id
                ));
            }
            if task.is_reassigning() {
                let previous = fresh.members.iter().find(|member| {
                    Some(&member.id) == task.handoff_from_member_id.as_ref() && member.is_stopping()
                });
                let Some(previous) = previous.filter(|_| task.assignee.as_deref() == Some(target.as_str())) else {
                    return Err(format!("task {} is already being reassigned", task.id));
                };
                return Ok(Revoked {
                    previous_assignee: previous.name.clone(),
                    previous_member: Some(previous.clone()),
                    handoff_id: task.handoff_id.clone(),
                });
            }
            if target == CAPTAIN_KEY {
                if let Some(busy) = captain_open_task(&fresh, Some(&task_id)) {
                    return Err(format!(
                        "captain is busy with {}; complete or reassign it before taking over {task_id}",
                        busy.id
                    ));
                }
                let pending = unsatisfied_dependencies(&fresh.tasks, &fresh.tasks[index].dependencies);
                if !pending.is_empty() {
                    return Err(format!(
                        "task {task_id} is blocked by unfinished dependencies: {} — complete them before captain takeover",
                        pending.join(", ")
                    ));
                }
            } else {
                let member = require_member(&fresh, &target)?;
                if let Some(busy) = member_open_task(&fresh, &member.name, Some(&task_id)) {
                    return Err(format!(
                        "member \"{}\" is busy with {}; finish or reassign it first",
                        member.name, busy.id
                    ));
                }
            }
            let task = &fresh.tasks[index];
            let previous_assignee = task.assignee.clone().unwrap_or_default();
            let previous_index = if !task.status.is_open_attempt()
                || task.assignee.is_none()
                || task.assignee.as_deref() == Some(CAPTAIN_KEY)
            {
                None
            } else {
                fresh.members.iter().position(|member| {
                    Some(&member.name) == task.assignee.as_ref() && !member.is_removed()
                })
            };
            invalidate_task_attempt(&mut fresh.tasks[index], Some(&target), true);
            let mut previous_member = None;
            if let Some(member_index) = previous_index {
                fresh.members[member_index].stopping = Some(true);
                let member = fresh.members[member_index].clone();
                fresh.tasks[index].handoff_from_member_id = Some(member.id.clone());
                let unread = mailbox::read_unread(&self.root, &fresh.id, &member.name).map_err(text)?;
                mailbox::discard(&self.root, &fresh.id, &member.name, &mailbox::ids_of(&unread))
                    .map_err(text)?;
                previous_member = Some(member);
            }
            let handoff_id = fresh.tasks[index].handoff_id.clone();
            self.write(&fresh)?;
            Ok(Revoked {
                previous_assignee,
                previous_member,
                handoff_id,
            })
        })?;

        let quiescence = match &revoked.previous_member {
            Some(member) if member.is_spawned() => {
                self.host.drain_members(std::slice::from_ref(&member.id))
            }
            _ => Ok(()),
        };

        self.with_team_lock(&team.id, || -> OpResult<()> {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let index = task_index(&fresh, &task_id)?;
            let task = &fresh.tasks[index];
            if task.handoff_id != revoked.handoff_id
                || task.assignee.as_deref() != Some(target.as_str())
                || !task.is_reassigning()
            {
                return Err(format!(
                    "task {} changed during reassignment; refusing to overwrite the newer state",
                    task.id
                ));
            }
            fresh.tasks[index].reassigning = Some(quiescence.is_err());
            if quiescence.is_ok() {
                if let Some(previous) = &revoked.previous_member
                    && let Some(member) = fresh
                        .members
                        .iter_mut()
                        .find(|member| member.id == previous.id)
                {
                    member.stopping = None;
                }
                fresh.tasks[index].handoff_from_member_id = None;
                if target == CAPTAIN_KEY {
                    let task = &mut fresh.tasks[index];
                    begin_task_attempt(task, CAPTAIN_KEY);
                    // The captain is already in the turn that asked for the
                    // takeover; there is no member claim to move it on.
                    task.status = TaskStatus::InProgress;
                    task.updated_at = now_ms();
                }
            }
            self.write(&fresh)
        })?;
        quiescence?;
        if target != CAPTAIN_KEY {
            self.kick_member(&team.id, &target);
        }
        let current = self
            .root
            .read_team(&team.id)
            .map_err(text)?
            .ok_or_else(|| format!("team \"{}\" ended during reassignment", team.name))?;
        let task = require_task(&current, &task_id)?;
        Ok(ReassignResult {
            task_id: task.id.clone(),
            previous_assignee: revoked.previous_assignee,
            assignee: task.assignee.clone().unwrap_or_default(),
            status: task.status.as_str().to_owned(),
            attempt: task.attempt_number(),
            attempt_id: task.attempt_id.clone(),
        })
    }

    /// `agent_teams_claim_task`.
    pub fn claim_task(&self, caller: &str, input: &Value) -> OpResult<ClaimResult> {
        let args = Args::new("agent_teams_claim_task", input)?;
        let task_id = args.req_str("task_id")?;
        let requested_assignee = args.opt_str("assignee")?;
        let team = self.require_participant_team(caller)?;
        self.with_team_lock(&team.id, || {
            let (mut fresh, identity) = self.require_fresh_participant(&team.id, caller)?;
            let index = task_index(&fresh, &task_id)?;
            let task = &fresh.tasks[index];
            if task.is_reassigning() {
                return Err(format!(
                    "task {} is being reassigned; wait for the handoff to finish",
                    task.id
                ));
            }
            let mut assignee = task.assignee.clone();
            match &identity {
                Identity::Captain => {
                    if requested_assignee.is_some()
                        || task.assignee.as_deref() != Some(CAPTAIN_KEY)
                        || !task.status.is_open_attempt()
                    {
                        return Err("claim_task is for members claiming their own task; captains must use agent_teams_reassign_task to assign and wake a member".to_owned());
                    }
                }
                Identity::Member(name) => {
                    if requested_assignee.is_some() {
                        return Err("members cannot set assignee when claiming a task".to_owned());
                    }
                    if let Some(owner) = &assignee
                        && owner != name
                    {
                        return Err(format!("task {} is assigned to \"{owner}\", not you", task.id));
                    }
                    assignee = Some(name.clone());
                }
            }
            if task.status.is_open_attempt() {
                if assignee.is_none() || task.assignee != assignee {
                    return Err(format!(
                        "task {} is already claimed by \"{}\"",
                        task.id,
                        task.assignee.as_deref().unwrap_or("nobody")
                    ));
                }
                return Ok(ClaimResult {
                    task_details: task_details(&fresh, task),
                    task_id: task.id.clone(),
                    status: task.status.as_str().to_owned(),
                    assignee: assignee.unwrap_or_default(),
                    attempt: task.attempt_number(),
                    attempt_id: task.attempt_id.clone(),
                });
            }
            let pending = unsatisfied_dependencies(&fresh.tasks, &task.dependencies);
            if !pending.is_empty() {
                return Err(format!(
                    "task {} is blocked by unfinished dependencies: {} — complete them first",
                    task.id,
                    pending.join(", ")
                ));
            }
            if let Some(error) = transition_error(task.status, TaskStatus::Claimed) {
                return Err(error);
            }
            let Some(assignee) = assignee else {
                return Err("claiming an unassigned task needs an assignee (claim on behalf of a member)".to_owned());
            };
            if let Some(busy) = member_open_task(&fresh, &assignee, Some(&task_id)) {
                return Err(format!(
                    "member \"{assignee}\" is busy with {}; finish or reassign it first",
                    busy.id
                ));
            }
            let attempt_id = begin_task_attempt(&mut fresh.tasks[index], &assignee);
            self.write(&fresh)?;
            let task = &fresh.tasks[index];
            Ok(ClaimResult {
                task_details: task_details(&fresh, task),
                task_id: task.id.clone(),
                status: task.status.as_str().to_owned(),
                assignee: task.assignee.clone().unwrap_or_default(),
                attempt: task.attempt_number(),
                attempt_id: Some(attempt_id),
            })
        })
    }

    /// `agent_teams_update_task`.
    pub fn update_task(&self, caller: &str, input: &Value) -> OpResult<UpdateTaskResult> {
        let args = Args::new("agent_teams_update_task", input)?;
        let task_id = args.req_str("task_id")?;
        let attempt_id = args.opt_str("attempt_id")?;
        let status = args
            .opt_enum(
                "status",
                &["in_progress", "completed", "failed", "cancelled"],
            )?
            .and_then(|status| TaskStatus::parse(&status));
        let output = args.opt_str("output")?;
        let evidence_note = args.opt_str("evidence_note")?;
        let verdict = args
            .opt_enum("verdict", &["pass", "needs_revision", "reject"])?
            .and_then(|verdict| ReviewVerdict::parse(&verdict));
        let mut normalized = args.map().clone();
        crate::validate::normalize_blank_optional_task_fields(&mut normalized);
        let changed_paths = Args::new("agent_teams_update_task", &Value::Object(normalized))?
            .opt_strings("changedPaths")?;
        let team = self.require_participant_team(caller)?;
        let mut follow_up_message: Option<TeamMessage> = None;
        let updated = self.with_team_lock(&team.id, || -> OpResult<UpdateTaskResult> {
            let (mut fresh, identity) = self.require_fresh_participant(&team.id, caller)?;
            let index = task_index(&fresh, &task_id)?;
            let task = &fresh.tasks[index];
            if identity.is_captain()
                && task.assignee.is_some()
                && task.assignee.as_deref() != Some(CAPTAIN_KEY)
                && !task.status.is_terminal()
                && !(status == Some(TaskStatus::Cancelled)
                    && task.status == TaskStatus::Pending
                    && task.attempt_number() == 0
                    && !task.is_reassigning())
            {
                return Err(format!(
                    "task {} is owned by member \"{}\"; call agent_teams_reassign_task with assignee=\"captain\" before takeover",
                    task.id,
                    task.assignee.as_deref().unwrap_or_default()
                ));
            }
            if let Identity::Member(name) = &identity {
                if task.assignee.as_deref() != Some(name.as_str()) {
                    return Err(format!(
                        "task {} is assigned to \"{}\", not you",
                        task.id,
                        task.assignee.as_deref().unwrap_or("nobody")
                    ));
                }
                if let Some(current) = &task.attempt_id {
                    match attempt_id.as_deref().map(str::trim) {
                        None | Some("") => {
                            return Err(format!(
                                "missing attempt_id for task {}. Retry this update with attempt_id=\"{current}\" from your current assignment. This is a missing parameter, not a revoked attempt; do not restart the work or request reassignment.",
                                task.id
                            ));
                        }
                        Some(given) if given != current => {
                            return Err(format!(
                                "stale attempt for task {}: expected the current attempt_id; stop work and request fresh assignment",
                                task.id
                            ));
                        }
                        Some(_) => {}
                    }
                }
            }
            let findings = parse_findings(args.raw("findings"))?;
            let acceptance_results = parse_acceptance_results(args.raw("acceptanceResults"))?;
            let commands_run = parse_command_results(args.raw("commandsRun"))?;
            let update = QualityCompletionUpdate {
                status,
                output: output.clone(),
                verdict,
                findings: findings.clone(),
                changed_paths: changed_paths.clone(),
                acceptance_results: acceptance_results.clone(),
                commands_run: commands_run.clone(),
                evidence_note: evidence_note.clone(),
            };
            if task.status.is_terminal() {
                let task = &mut fresh.tasks[index];
                let appended = append_task_evidence(task, &update, identity.name())?;
                let result = UpdateTaskResult {
                    task_id: task.id.clone(),
                    status: task.status.as_str().to_owned(),
                    output: task.output.clone(),
                    attempt: task.attempt_number(),
                    attempt_id: task.attempt_id.clone(),
                    evidence_count: Some(task.supplemental_evidence.as_ref().map_or(0, Vec::len)),
                    follow_up: None,
                };
                if appended {
                    self.write(&fresh)?;
                }
                return Ok(result);
            }
            if evidence_note.as_deref().is_some_and(|note| !note.trim().is_empty()) {
                return Err("evidence_note is for terminal tasks; record active work with output and structured evidence".to_owned());
            }
            evaluate_quality_completion(task, &update).map_err(|rejection| rejection.to_string())?;
            let task = &mut fresh.tasks[index];
            if let Some(next) = status {
                if let Some(error) = transition_error(task.status, next) {
                    return Err(error);
                }
                task.status = next;
            }
            if let Some(output) = &output {
                task.output = Some(output.clone());
            }
            if let Some(verdict) = verdict {
                task.verdict = Some(verdict);
            }
            if findings.is_some() {
                task.findings = findings;
            }
            if changed_paths.is_some() {
                task.changed_paths = changed_paths.clone();
            }
            if acceptance_results.is_some() {
                task.acceptance_results = acceptance_results;
            }
            if commands_run.is_some() {
                task.commands_run = commands_run;
            }
            task.updated_at = now_ms();
            let closed = task.clone();
            let prior: BTreeMap<String, Vec<String>> = fresh
                .tasks
                .iter()
                .map(|item| (item.id.clone(), item.dependencies.clone()))
                .collect();
            let follow_up = (closed.status == TaskStatus::Failed
                && matches!(closed.verdict, Some(ReviewVerdict::NeedsRevision | ReviewVerdict::Reject)))
            .then(|| apply_quality_follow_up(&mut fresh, &closed));
            let mut summary = None;
            if let Some(follow_up) = &follow_up
                && !follow_up.created.is_empty()
            {
                let rewired: Vec<String> = fresh
                    .tasks
                    .iter()
                    .filter(|item| prior.get(&item.id).is_some_and(|before| before != &item.dependencies))
                    .map(|item| format!("{} -> {}", item.id, item.dependencies.join(",")))
                    .collect();
                let created: Vec<String> = follow_up
                    .created
                    .iter()
                    .map(|item| {
                        let deps = if item.dependencies.is_empty() {
                            "none".to_owned()
                        } else {
                            item.dependencies.join(",")
                        };
                        format!(
                            "{} ({}, owner={}, deps={deps})",
                            item.id,
                            item.kind_or_work(),
                            item.assignee.as_deref().unwrap_or("unassigned")
                        )
                    })
                    .collect();
                let rewired = if rewired.is_empty() {
                    String::new()
                } else {
                    format!(" Updated dependencies: {}.", rewired.join("; "))
                };
                let text = format!(
                    "Automatic quality follow-up for {}: {}.{rewired} Use these tasks; do not create duplicate repair/review work.",
                    closed.id,
                    created.join("; ")
                );
                follow_up_message = Some(TeamMessage::new(CAPTAIN_KEY, CAPTAIN_KEY, text.clone()));
                summary = Some(text);
            }
            if follow_up.as_ref().is_some_and(|follow_up| follow_up.escalated) {
                let verdict = closed.verdict.map(|verdict| verdict.as_str()).unwrap_or_default();
                let kind = closed.kind.map(|kind| kind.as_str()).unwrap_or("review");
                mailbox::append(
                    &self.root,
                    &fresh.id,
                    CAPTAIN_KEY,
                    &TeamMessage::new(
                        CAPTAIN_KEY,
                        CAPTAIN_KEY,
                        format!(
                            "Quality-gate loop escalated after {} ({kind} verdict={verdict}). Automatic repair/review stopped.",
                            closed.id
                        ),
                    ),
                )
                .map_err(text)?;
            }
            self.write(&fresh)?;
            if let Some(message) = &follow_up_message {
                mailbox::append(&self.root, &fresh.id, CAPTAIN_KEY, message).map_err(text)?;
            }
            let task = &fresh.tasks[index];
            Ok(UpdateTaskResult {
                task_id: task.id.clone(),
                status: task.status.as_str().to_owned(),
                output: task.output.clone(),
                attempt: task.attempt_number(),
                attempt_id: task.attempt_id.clone(),
                evidence_count: None,
                follow_up: summary,
            })
        })?;
        if let Some(message) = follow_up_message {
            self.deliver_captain_mail(&team.id, &[message]);
        }
        self.kick_team(&team.id);
        Ok(updated)
    }

    /// `agent_teams_amend_task`.
    pub fn amend_task(&self, caller: &str, input: &Value) -> OpResult<AmendResult> {
        let args = Args::new("agent_teams_amend_task", input)?;
        let task_id = args.req_str("task_id")?;
        let reason = args.req_str("reason")?;
        let mut amendment = ContractAmendmentInput {
            objective: args.opt_str("objective")?,
            acceptance: args.opt_strings("acceptance")?,
            verify: args.opt_strings("verify")?,
            in_scope: args.opt_strings("inScope")?,
            out_of_scope: args.opt_strings("outOfScope")?,
        };
        // Blank means absent, as everywhere else.
        if amendment
            .objective
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
        {
            amendment.objective = None;
        }
        for list in [
            &mut amendment.acceptance,
            &mut amendment.verify,
            &mut amendment.in_scope,
            &mut amendment.out_of_scope,
        ] {
            if let Some(items) = list.as_mut() {
                items.retain(|item| !item.trim().is_empty());
                if items.is_empty() {
                    *list = None;
                }
            }
        }
        let team = self.require_captain_team(caller)?;
        self.with_team_lock(&team.id, || {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            require_task(&fresh, &task_id)?;
            let revision = crate::quality::amend_task_contract(
                &mut fresh,
                &task_id,
                &amendment,
                CAPTAIN_KEY,
                &reason,
            )?;
            let index = task_index(&fresh, &task_id)?;
            fresh.tasks[index].updated_at = now_ms();
            self.write(&fresh)?;
            let task = &fresh.tasks[index];
            // Key order as the reference writes it.
            let mut contract = Vec::new();
            if let Some(objective) = &task.objective {
                contract.push(format!(
                    "\"objective\":{}",
                    serde_json::to_string(objective).unwrap_or_default()
                ));
            }
            for (key, value) in [
                ("acceptance", &task.acceptance),
                ("verify", &task.verify),
                ("inScope", &task.in_scope),
                ("outOfScope", &task.out_of_scope),
            ] {
                if let Some(items) = value {
                    contract.push(format!(
                        "\"{key}\":{}",
                        serde_json::to_string(items).unwrap_or_default()
                    ));
                }
            }
            Ok(AmendResult {
                task_id: task.id.clone(),
                status: task.status.as_str().to_owned(),
                revised_fields: revision.fields.join(", "),
                revision_count: task.revisions.as_ref().map_or(0, Vec::len),
                contract: format!("{{{}}}", contract.join(",")),
            })
        })
    }

    /// `agent_teams_send_message`.
    pub fn send_message(&self, caller: &str, input: &Value) -> OpResult<SendMessageResult> {
        let args = Args::new("agent_teams_send_message", input)?;
        let to = args.req_str("to")?.trim().to_owned();
        let content = args.req_str("content")?;
        let claimed_from = args.opt_str("from")?;
        let source_task_id = trimmed_optional(args.opt_str("source_task_id")?.as_deref());
        let source_attempt_id = trimmed_optional(args.opt_str("source_attempt_id")?.as_deref());
        let team = self.require_participant_team(caller)?;
        enum Prepared {
            Duplicate(TeamMessage, String),
            Captain(TeamMessage, String, Identity),
            Member(TeamMessage, String, String),
        }
        let prepared = self.with_team_lock(&team.id, || -> OpResult<Prepared> {
            let (fresh, identity) = self.require_fresh_participant(&team.id, caller)?;
            let from = identity.name().to_owned();
            if let Some(claimed) = &claimed_from
                && claimed != &from
            {
                return Err(format!(
                    "agent_teams_send_message: \"from\" must be your own identity (\"{from}\"), not \"{claimed}\""
                ));
            }
            if source_task_id.is_some() != source_attempt_id.is_some() {
                return Err("send_message requires source_task_id and source_attempt_id together".to_owned());
            }
            let source: Option<&TeamTask> = match &source_task_id {
                None => match &identity {
                    Identity::Member(name) => member_open_task(&fresh, name, None).or_else(|| {
                        fresh
                            .tasks
                            .iter()
                            .filter(|item| item.assignee.as_deref() == Some(name.as_str()) && item.attempt_id.is_some())
                            .max_by_key(|item| item.updated_at)
                    }),
                    Identity::Captain => None,
                },
                Some(id) => Some(require_task(&fresh, id)?),
            };
            if source_task_id.is_some()
                && source.is_none_or(|task| {
                    task.assignee.as_deref() != Some(identity.name()) || task.attempt_id != source_attempt_id
                })
            {
                return Err("stale or foreign source attempt; stop sending results from the revoked task".to_owned());
            }
            let source_fields = source.filter(|task| task.attempt_id.is_some()).map(|task| {
                (task.id.clone(), task.attempt_id.clone(), task.status)
            });
            let owned = if to == CAPTAIN_KEY {
                None
            } else {
                let member = require_member(&fresh, &to)?;
                member_open_task(&fresh, &member.name, None).cloned()
            };
            let duplicate = mailbox::read(&self.root, &fresh.id, &to)
                .map_err(text)?
                .into_iter()
                .find(|message| {
                    message.from == from
                        && message.content == content
                        && message.task_id == owned.as_ref().map(|task| task.id.clone())
                        && message.attempt_id == owned.as_ref().and_then(|task| task.attempt_id.clone())
                        && message.source_task_id == source_fields.as_ref().map(|fields| fields.0.clone())
                        && message.source_attempt_id == source_fields.as_ref().and_then(|fields| fields.1.clone())
                        && message.source_task_status == source_fields.as_ref().map(|fields| fields.2)
                        && mailbox::is_current_mail(&fresh, message)
                        && (message.attempt_id.is_some()
                            || message.source_attempt_id.is_some()
                            || message.read_at.is_none())
                });
            if let Some(duplicate) = duplicate {
                return Ok(Prepared::Duplicate(duplicate, from));
            }
            let stamp = |message: &mut TeamMessage| {
                if let Some((task, attempt, status)) = &source_fields {
                    message.source_task_id = Some(task.clone());
                    message.source_attempt_id = attempt.clone();
                    message.source_task_status = Some(*status);
                }
                message.delivery_claimed_at = Some(now_ms());
            };
            if to == CAPTAIN_KEY {
                let mut message = TeamMessage::new(&from, CAPTAIN_KEY, &content);
                stamp(&mut message);
                mailbox::append(&self.root, &fresh.id, CAPTAIN_KEY, &message).map_err(text)?;
                return Ok(Prepared::Captain(message, from, identity));
            }
            if fresh.is_halted() {
                return Err(format!(
                    "team \"{}\" is halted; call agent_teams_resume before waking a member",
                    fresh.name
                ));
            }
            let recipient = require_member(&fresh, &to)?.name.clone();
            let mut message = TeamMessage::new(&from, &recipient, &content);
            stamp(&mut message);
            if let Some(task) = &owned
                && task.attempt_id.is_some()
            {
                message.task_id = Some(task.id.clone());
                message.attempt_id = task.attempt_id.clone();
            }
            mailbox::append(&self.root, &fresh.id, &recipient, &message).map_err(text)?;
            Ok(Prepared::Member(message, from, recipient))
        })?;
        match prepared {
            Prepared::Duplicate(message, from) => Ok(SendMessageResult {
                message_id: message.id,
                from,
                to: message.to,
                delivered: "duplicate".to_owned(),
            }),
            Prepared::Captain(message, from, identity) => {
                let delivered = if identity.is_captain() {
                    self.with_team_lock(&team.id, || {
                        let _ = mailbox::release_delivery(
                            &self.root,
                            &team.id,
                            CAPTAIN_KEY,
                            std::slice::from_ref(&message.id),
                        );
                    });
                    "mailbox"
                } else if self.deliver_captain_mail(&team.id, std::slice::from_ref(&message)) {
                    "live"
                } else {
                    "mailbox"
                };
                Ok(SendMessageResult {
                    message_id: message.id,
                    from,
                    to: CAPTAIN_KEY.to_owned(),
                    delivered: delivered.to_owned(),
                })
            }
            Prepared::Member(message, from, recipient) => {
                let text_to_send =
                    mailbox::mailbox_prompt(&recipient, std::slice::from_ref(&message));
                let accepted = self.dispatch_member(
                    &team.id,
                    &recipient,
                    &text_to_send,
                    DeliveryMode::Steer,
                    message.attempt_id.as_deref(),
                );
                let ids = std::slice::from_ref(&message.id);
                self.with_team_lock(&team.id, || {
                    let _ = if accepted {
                        mailbox::acknowledge(&self.root, &team.id, &recipient, ids)
                    } else {
                        mailbox::release_delivery(&self.root, &team.id, &recipient, ids)
                    };
                });
                Ok(SendMessageResult {
                    message_id: message.id,
                    from,
                    to: recipient,
                    delivered: if accepted { "wake" } else { "mailbox" }.to_owned(),
                })
            }
        }
    }

    /// `agent_teams_status`.
    pub fn status(&self, caller: &str, _input: &Value) -> OpResult<StatusView> {
        let located = self.require_participant_team(caller)?;
        if located.captain_session_id == caller {
            self.kick_team(&located.id);
        }
        let (team, identity) = self.with_team_lock(&located.id, || {
            self.require_fresh_participant(&located.id, caller)
        })?;
        let members: Vec<StatusMember> = team
            .members
            .iter()
            .filter(|member| !member.is_removed())
            .map(|member| StatusMember {
                name: member.name.clone(),
                role: member.role.clone().unwrap_or_default(),
                provider: member.provider.clone().unwrap_or_default(),
                model: member.model.clone().unwrap_or_default(),
                reasoning_effort: member.reasoning_effort.clone().unwrap_or_default(),
                status: member.status.as_str().to_owned(),
                activity: if member.is_spawned() {
                    self.host.member_activity(&member.id).as_str().to_owned()
                } else {
                    "unspawned".to_owned()
                },
                spawn_error: member.spawn_error.clone(),
            })
            .collect();
        let tasks: Vec<StatusTask> = team
            .tasks
            .iter()
            .map(|task| StatusTask {
                id: task.id.clone(),
                subject: task.subject.clone(),
                status: task.status.as_str().to_owned(),
                assignee: task.assignee.clone().unwrap_or_default(),
                dependencies: task.dependencies.clone(),
                attempt: task.attempt_number(),
                attempt_id: task.attempt_id.clone().unwrap_or_default(),
                reassigning: task.is_reassigning(),
                kind: task.kind_or_work().as_str().to_owned(),
                round: task.round,
                verdict: task.verdict.map(|verdict| verdict.as_str().to_owned()),
                supplemental_evidence: task
                    .supplemental_evidence
                    .as_ref()
                    .map(|evidence| serde_json::to_string(evidence).unwrap_or_default()),
                findings_open: task
                    .findings
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|finding| finding.resolved != Some(true))
                    .count(),
                seed_id: task.profile_seed_id.clone(),
                output: task.output.clone(),
            })
            .collect();

        let mut warnings = Vec::new();
        let mut warning_count = 0usize;
        let (captain_inbox, own_inbox, member_inboxes) =
            self.with_team_lock(&team.id, || -> OpResult<_> {
                let mut read_current = |agent: &str| -> OpResult<Vec<TeamMessage>> {
                    let (_, malformed) =
                        mailbox::read_with_warnings(&self.root, &team.id, agent).map_err(text)?;
                    for line in malformed {
                        warning_count += 1;
                        if warnings.len() < 10 {
                            warnings.push(format!("{agent} mailbox line {}", line.line));
                        }
                    }
                    mailbox::read_current(&self.root, &team, agent).map_err(text)
                };
                let captain_inbox = if identity.is_captain() {
                    read_current(CAPTAIN_KEY)?
                } else {
                    Vec::new()
                };
                let own_inbox: Vec<TeamMessage> = match &identity {
                    Identity::Member(name) => read_current(name)?.into_iter().take(10).collect(),
                    Identity::Captain => Vec::new(),
                };
                let mut member_inboxes = BTreeMap::new();
                for member in members.iter().filter(|member| match &identity {
                    Identity::Captain => true,
                    Identity::Member(name) => &member.name == name,
                }) {
                    let messages = read_current(&member.name)?;
                    if let Some(latest) = messages.last() {
                        member_inboxes.insert(
                            member.name.clone(),
                            StatusInboxSummary {
                                count: messages.len(),
                                latest: latest.content.chars().take(200).collect(),
                            },
                        );
                    }
                }
                Ok((captain_inbox, own_inbox, member_inboxes))
            })?;
        let mut goal_items: Vec<String> = Vec::new();
        for task in &team.tasks {
            for item in task.coverage_of.as_deref().unwrap_or_default() {
                if !goal_items.contains(item) {
                    goal_items.push(item.clone());
                }
            }
        }
        let coverage = build_coverage_matrix(&goal_items, &team.tasks);
        let delivery = can_declare_delivery(&team);
        let quality_loop = describe_quality_loop(&team);
        let shown_captain: Vec<TeamMessage> = captain_inbox.into_iter().take(10).collect();
        let acknowledged: Vec<String> = if identity.is_captain() {
            mailbox::ids_of(&shown_captain)
        } else {
            mailbox::ids_of(&own_inbox)
        };
        let inbox_item = |message: &TeamMessage| StatusInboxItem {
            from: message.from.clone(),
            content: mailbox::mailbox_content(message),
            ts: message.ts,
        };
        let view = StatusView {
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            description: team.description.clone().unwrap_or_default(),
            phase: team.phase_or_running().as_str().to_owned(),
            halted: quality_loop.halted,
            escalated: quality_loop.escalated,
            loop_state: quality_loop.state.as_str().to_owned(),
            loop_summary: quality_loop.summary.clone(),
            deliverable: quality_loop.deliverable,
            coverage,
            delivery,
            profile: team.profile.as_ref().map(|profile| StatusProfile {
                name: profile.name.clone(),
                protocol: profile
                    .protocol
                    .as_ref()
                    .map(|protocol| protocol.chars().take(240).collect()),
                task_planning: profile
                    .task_planning
                    .map(|planning| planning.as_str().to_owned()),
            }),
            viewer: identity.name().to_owned(),
            members,
            tasks,
            captain_inbox: shown_captain.iter().map(inbox_item).collect(),
            member_inbox: own_inbox.iter().map(inbox_item).collect(),
            member_inboxes,
            mailbox_warnings: warnings,
            mailbox_warning_count: warning_count,
        };
        if !acknowledged.is_empty() {
            let agent = if identity.is_captain() {
                CAPTAIN_KEY.to_owned()
            } else {
                identity.name().to_owned()
            };
            self.with_team_lock(&team.id, || {
                let _ = mailbox::acknowledge(&self.root, &team.id, &agent, &acknowledged);
            });
        }
        Ok(view)
    }

    /// `agent_teams_resume`.
    pub fn resume(&self, caller: &str, input: &Value) -> OpResult<ResumeResult> {
        let args = Args::new("agent_teams_resume", input)?;
        let reason = args.req_str("reason")?;
        let team = self.require_captain_team(caller)?;
        let result = self.with_team_lock(&team.id, || {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let resumed = resume_team_state(&mut fresh, &reason);
            if resumed.status == ResumeStatus::Rejected {
                return Err(resumed
                    .error
                    .unwrap_or_else(|| "resume rejected".to_owned()));
            }
            if resumed.status == ResumeStatus::Resumed {
                self.write(&fresh)?;
            }
            Ok(ResumeResult {
                status: resumed.status.as_str().to_owned(),
                team_id: fresh.id.clone(),
                reason: reason.clone(),
            })
        })?;
        if result.status == "resumed" {
            self.kick_team(&team.id);
        }
        Ok(result)
    }

    /// `agent_teams_delete`: stop everyone, then archive.
    pub fn delete(&self, caller: &str, _input: &Value) -> OpResult<DeleteResult> {
        let team = self.require_captain_team(caller)?;
        let roster = self.with_team_lock(&team.id, || -> OpResult<Vec<TeamMember>> {
            let mut fresh = self.require_fresh_captain(&team.id)?;
            let roster = fresh.members.clone();
            for index in 0..fresh.members.len() {
                let name = fresh.members[index].name.clone();
                let unread = mailbox::read_unread(&self.root, &fresh.id, &name).map_err(text)?;
                mailbox::discard(&self.root, &fresh.id, &name, &mailbox::ids_of(&unread))
                    .map_err(text)?;
                if fresh.members[index].is_removed() {
                    continue;
                }
                fresh.members[index].status = MemberStatus::Removed;
                for task in fresh.tasks.iter_mut() {
                    if task.assignee.as_deref() == Some(name.as_str()) && !task.status.is_terminal()
                    {
                        invalidate_task_attempt(task, None, false);
                    }
                }
            }
            self.write(&fresh)?;
            Ok(roster)
        })?;
        let ids: Vec<String> = roster
            .iter()
            .filter(|member| member.is_spawned())
            .map(|member| member.id.clone())
            .collect();
        global_locks()
            .with(&format!("retired:{}", self.root.lock_key()), || {
                self.root
                    .record_retired_member_ids(ids.iter().map(String::as_str))
            })
            .map_err(text)?;
        let drained = self.host.drain_members(&ids);
        self.with_team_lock(&team.id, || {
            self.require_fresh_captain(&team.id)?;
            self.root.archive_team_dir(&team.id).map_err(text)?;
            self.host.team_changed(&team.id);
            Ok::<(), String>(())
        })?;
        drained?;
        Ok(DeleteResult {
            team_name: team.name,
        })
    }

    /// Run one tool by name and render its result for the model.
    pub fn call_tool(&self, name: &str, caller: &str, input: &Value) -> OpResult<String> {
        match name {
            "agent_teams_create" => self.create(caller, input).map(|result| result.render()),
            "agent_teams_approve" => self.approve(caller, input).map(|result| result.render()),
            "agent_teams_edit_plan" => self.edit_plan(caller, input).map(|result| result.render()),
            "agent_teams_add_member" => {
                self.add_member(caller, input).map(|result| result.render())
            }
            "agent_teams_remove_member" => self
                .remove_member(caller, input)
                .map(|result| result.render()),
            "agent_teams_create_task" => self
                .create_task(caller, input)
                .map(|result| result.render()),
            "agent_teams_reassign_task" => self
                .reassign_task(caller, input)
                .map(|result| result.render()),
            "agent_teams_claim_task" => {
                self.claim_task(caller, input).map(|result| result.render())
            }
            "agent_teams_update_task" => self
                .update_task(caller, input)
                .map(|result| result.render()),
            "agent_teams_amend_task" => {
                self.amend_task(caller, input).map(|result| result.render())
            }
            "agent_teams_send_message" => self
                .send_message(caller, input)
                .map(|result| result.render()),
            "agent_teams_status" => self.status(caller, input).map(|view| view.render()),
            "agent_teams_resume" => self.resume(caller, input).map(|result| result.render()),
            "agent_teams_delete" => self.delete(caller, input).map(|result| result.render()),
            other => Err(format!("unknown AgentTeams tool \"{other}\"")),
        }
    }
}
