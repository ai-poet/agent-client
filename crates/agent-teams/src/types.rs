// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Durable AgentTeams state.
//!
//! A team is one directory under the state root holding `team.json` plus an
//! `inbox/` of per-agent JSONL mailboxes. The JSON keys are the reference
//! plugin's (camelCase), so a state directory reads the same in both. Keys
//! this build does not know are kept in each record's `extra` and written
//! back unchanged.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Milliseconds since the Unix epoch, the unit every timestamp here uses.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Task lifecycle statuses in progression order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Claimed,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 6] = [
        TaskStatus::Pending,
        TaskStatus::Claimed,
        TaskStatus::InProgress,
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ];

    /// The wire spelling, as the prompts and tool results show it.
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Claimed => "claimed",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }

    /// After these a task can no longer be claimed or worked on.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }

    /// Claimed or in progress: an attempt is running.
    pub fn is_open_attempt(self) -> bool {
        matches!(self, TaskStatus::Claimed | TaskStatus::InProgress)
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured quality-gate kind. Absent means `work`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Requirements,
    Implementation,
    Verification,
    Review,
    Repair,
    Integration,
    Work,
}

impl TaskKind {
    pub const ALL: [TaskKind; 7] = [
        TaskKind::Requirements,
        TaskKind::Implementation,
        TaskKind::Verification,
        TaskKind::Review,
        TaskKind::Repair,
        TaskKind::Integration,
        TaskKind::Work,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Requirements => "requirements",
            TaskKind::Implementation => "implementation",
            TaskKind::Verification => "verification",
            TaskKind::Review => "review",
            TaskKind::Repair => "repair",
            TaskKind::Integration => "integration",
            TaskKind::Work => "work",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl std::fmt::Display for TaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Review / requirements conclusion. Only `pass` may complete those kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Pass,
    NeedsRevision,
    Reject,
}

impl ReviewVerdict {
    pub const ALL: [ReviewVerdict; 3] = [
        ReviewVerdict::Pass,
        ReviewVerdict::NeedsRevision,
        ReviewVerdict::Reject,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewVerdict::Pass => "pass",
            ReviewVerdict::NeedsRevision => "needs_revision",
            ReviewVerdict::Reject => "reject",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|verdict| verdict.as_str() == value)
    }
}

impl std::fmt::Display for ReviewVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Finding severity used by review / requirements output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Low,
    Medium,
    High,
    Blocker,
}

impl FindingSeverity {
    pub const ALL: [FindingSeverity; 4] = [
        FindingSeverity::Low,
        FindingSeverity::Medium,
        FindingSeverity::High,
        FindingSeverity::Blocker,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FindingSeverity::Low => "low",
            FindingSeverity::Medium => "medium",
            FindingSeverity::High => "high",
            FindingSeverity::Blocker => "blocker",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|severity| severity.as_str() == value)
    }

    /// High and blocker findings keep a review from passing.
    pub fn is_high(self) -> bool {
        matches!(self, FindingSeverity::High | FindingSeverity::Blocker)
    }
}

/// `passed` / `failed`, for acceptance criteria and verification commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckStatus::Passed => "passed",
            CheckStatus::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "passed" => Some(CheckStatus::Passed),
            "failed" => Some(CheckStatus::Failed),
            _ => None,
        }
    }
}

/// One structured review finding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    /// Stable id, for example `SEC-001`.
    pub id: String,
    pub severity: FindingSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub problem: String,
    pub required_fix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<bool>,
}

/// One acceptance criterion result recorded at completion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceResult {
    pub criterion: String,
    pub status: CheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// One verification command result recorded at completion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub command: String,
    pub status: CheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// Profile / team review-loop limits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements_min_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements_max_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_max_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_repair_attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<Vec<String>>,
}

/// One captain-only contract amendment recorded on a quality task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRevision {
    /// Epoch ms when the amendment was applied.
    pub at: u64,
    /// Identity that applied it (`captain`).
    pub by: String,
    /// Why the previous contract was wrong; kept for the audit trail.
    pub reason: String,
    /// Amended contract field names (`objective`, `acceptance`, …).
    pub fields: Vec<String>,
    /// Previous values of the amended fields; fields absent before are omitted.
    pub previous: Map<String, Value>,
}

/// Append-only observations; never replace the terminal verdict or unlock a gate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvidence {
    pub at: u64,
    pub by: String,
    pub attempt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_results: Option<Vec<AcceptanceResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands_run: Option<Vec<CommandResult>>,
}

/// One task of a team's task list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTask {
    /// Stable task id from the profile template; absent for ad-hoc tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_seed_id: Option<String>,
    /// Stable task id within the team (`t1`, `t2`, …).
    pub id: String,
    /// Brief title for the task.
    pub subject: String,
    /// What needs to be done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: TaskStatus,
    /// Member name (or `captain`) the task is assigned to; unassigned tasks
    /// await a claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Task ids that must reach `completed` before this task can be claimed.
    pub dependencies: Vec<String>,
    /// The worker's written result, set when the task completes or fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Monotonic execution generation. Reassignment/retry invalidates every
    /// older attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u64>,
    /// Capability for the current claimed/in-progress attempt. Members must
    /// present it when updating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// Opaque generation for a revocation/handoff that has not started its
    /// next attempt yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
    /// Previous activation retained until a handoff drain succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_from_member_id: Option<String>,
    /// A handoff is quiescing the old owner; the scheduler must not dispatch
    /// it yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reassigning: Option<bool>,
    /// Quality-gate kind. Missing values are treated as `work`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TaskKind>,
    /// Review / requirements / repair loop index, 1-based when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<ReviewVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<Vec<ReviewFinding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_scope: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_of_scope: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverables: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_goals: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_results: Option<Vec<AcceptanceResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands_run: Option<Vec<CommandResult>>,
    /// Supplemental observations, attributed to their original execution
    /// generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplemental_evidence: Option<Vec<TaskEvidence>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_attempt: Option<u64>,
    /// Repair source: the implementation / previous successful artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_finding_ids: Option<Vec<String>>,
    /// User-constraint / goal items this task claims to cover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_of: Option<Vec<String>>,
    /// Captain-only contract amendments, oldest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revisions: Option<Vec<TaskRevision>>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TeamTask {
    /// A fresh pending task with nothing but its identity and dependencies.
    pub fn new(id: impl Into<String>, subject: impl Into<String>, now: u64) -> Self {
        Self {
            profile_seed_id: None,
            id: id.into(),
            subject: subject.into(),
            description: None,
            status: TaskStatus::Pending,
            assignee: None,
            dependencies: Vec::new(),
            output: None,
            attempt: None,
            attempt_id: None,
            handoff_id: None,
            handoff_from_member_id: None,
            reassigning: None,
            kind: None,
            round: None,
            verdict: None,
            findings: None,
            objective: None,
            in_scope: None,
            out_of_scope: None,
            acceptance: None,
            verify: None,
            deliverables: None,
            non_goals: None,
            changed_paths: None,
            acceptance_results: None,
            commands_run: None,
            supplemental_evidence: None,
            reviewed_task_id: None,
            reviewed_attempt: None,
            source_task_id: None,
            source_finding_ids: None,
            coverage_of: None,
            revisions: None,
            created_at: now,
            updated_at: now,
            extra: Map::new(),
        }
    }

    /// The kind, with absent read as `work`.
    pub fn kind_or_work(&self) -> TaskKind {
        self.kind.unwrap_or(TaskKind::Work)
    }

    pub fn is_reassigning(&self) -> bool {
        self.reassigning == Some(true)
    }

    pub fn attempt_number(&self) -> u64 {
        self.attempt.unwrap_or(0)
    }
}

/// Member lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Idle,
    Working,
    Removed,
}

impl MemberStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MemberStatus::Idle => "idle",
            MemberStatus::Working => "working",
            MemberStatus::Removed => "removed",
        }
    }
}

/// A configured second-choice route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamModelFallback {
    pub provider: String,
    pub model: String,
}

/// One team member: a continuable sub-agent plus its team-side record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    /// Durable member session id (empty until spawned).
    pub id: String,
    /// Unique display name inside the team.
    pub name: String,
    /// Role description, e.g. `researcher`, `engineer`, `reviewer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Resolved provider (gateway platform) captured when this member was
    /// created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Resolved model captured when this member was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Resolved reasoning effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Prompt specific to this member's execution turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_prompt: Option<String>,
    /// Configured second-choice route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<TeamModelFallback>,
    /// Active route after fallback, without changing the primary route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    /// Whether the fallback route is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_active: Option<bool>,
    pub joined_at: u64,
    pub status: MemberStatus,
    /// Execution admission is closed while a handoff is drained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopping: Option<bool>,
    /// Last member-start failure, so the captain sees why a member never
    /// started. Cleared by the next successful start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_error: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TeamMember {
    pub fn new(name: impl Into<String>, now: u64) -> Self {
        Self {
            id: String::new(),
            name: name.into(),
            role: None,
            provider: None,
            model: None,
            reasoning_effort: None,
            execution_prompt: None,
            fallback: None,
            active_provider: None,
            active_model: None,
            fallback_active: None,
            joined_at: now,
            status: MemberStatus::Idle,
            stopping: None,
            spawn_error: None,
            extra: Map::new(),
        }
    }

    pub fn is_removed(&self) -> bool {
        self.status == MemberStatus::Removed
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping == Some(true)
    }

    pub fn is_spawned(&self) -> bool {
        !self.id.is_empty()
    }
}

/// One mailbox message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMessage {
    pub id: String,
    /// `captain` or a member name.
    pub from: String,
    /// `captain` or a member name.
    pub to: String,
    pub content: String,
    pub ts: u64,
    /// Process-local delivery lease; keeps two delivery paths from racing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_claimed_at: Option<u64>,
    /// Set once the recipient's live inbox accepted the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<u64>,
    /// Set once the recipient consumed it or was shown it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<u64>,
    /// Guidance scoped to the recipient's execution generation, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// Source execution generation, independent of the recipient's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task_status: Option<TaskStatus>,
    /// Cancelled delivery is kept for audit but must not wake the recipient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discarded_at: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TeamMessage {
    /// A fresh message record.
    pub fn new(from: impl Into<String>, to: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from: from.into(),
            to: to.into(),
            content: content.into(),
            ts: now_ms(),
            delivery_claimed_at: None,
            delivered_at: None,
            read_at: None,
            task_id: None,
            attempt_id: None,
            source_task_id: None,
            source_attempt_id: None,
            source_task_status: None,
            discarded_at: None,
            extra: Map::new(),
        }
    }

    /// Neither read nor discarded.
    pub fn is_unread(&self) -> bool {
        self.read_at.is_none() && self.discarded_at.is_none()
    }
}

/// Who plans the task graph of a profile team.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPlanning {
    /// The profile provides the roster and constraints; the captain designs
    /// the graph.
    Captain,
    /// The profile's fixed task graph is expanded as-is.
    Seed,
}

impl TaskPlanning {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskPlanning::Captain => "captain",
            TaskPlanning::Seed => "seed",
        }
    }
}

/// Snapshot of the named profile used to seed a team.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamProfileSnapshot {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<TeamModelFallback>,
    /// Frozen planning mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_planning: Option<TaskPlanning>,
    /// Frozen review-loop policy from the creating profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_policy: Option<ReviewPolicy>,
}

/// Two-phase execution lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamPhase {
    /// A draft awaiting the user's approval; nothing runs.
    Staged,
    Running,
}

impl TeamPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            TeamPhase::Staged => "staged",
            TeamPhase::Running => "running",
        }
    }
}

/// Human-facing review sub-state while a team is staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanReviewState {
    AwaitingReview,
    /// The user returned to chat; the captain must ask what should change
    /// before editing this same draft.
    AwaitingFeedback,
}

impl PlanReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanReviewState::AwaitingReview => "awaiting_review",
            PlanReviewState::AwaitingFeedback => "awaiting_feedback",
        }
    }
}

/// The full durable team record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamState {
    /// Original team name.
    pub name: String,
    /// Sanitized directory id; the team's stable identity.
    pub id: String,
    /// Team purpose/goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Immutable named profile snapshot, when created from a profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<TeamProfileSnapshot>,
    /// Session id of the captain agent that owns this team.
    pub captain_session_id: String,
    pub created_at: u64,
    /// Teammates only; the captain is implicit (the owning session).
    pub members: Vec<TeamMember>,
    pub tasks: Vec<TeamTask>,
    /// Monotonic task id counter.
    pub task_seq: u64,
    /// Missing means `running`, for teams created before staging existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<TeamPhase>,
    /// Missing while staged reads as `awaiting_review`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_review_state: Option<PlanReviewState>,
    /// Written only after a staged plan is explicitly approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<u64>,
    /// Human halt: the team stays on disk, unfinished work is cancelled until
    /// the captain resumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halted_at: Option<u64>,
    /// Review-loop policy snapshot copied from the creating profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_policy: Option<ReviewPolicy>,
    /// Set when an automatic review/repair loop hits its ceiling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalated: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TeamState {
    pub fn new(
        name: impl Into<String>,
        id: impl Into<String>,
        captain_session_id: impl Into<String>,
        now: u64,
    ) -> Self {
        Self {
            name: name.into(),
            id: id.into(),
            description: None,
            profile: None,
            captain_session_id: captain_session_id.into(),
            created_at: now,
            members: Vec::new(),
            tasks: Vec::new(),
            task_seq: 0,
            phase: None,
            plan_review_state: None,
            approved_at: None,
            halted: None,
            halted_at: None,
            review_policy: None,
            escalated: None,
            extra: Map::new(),
        }
    }

    /// The phase, with absent read as `running`.
    pub fn phase_or_running(&self) -> TeamPhase {
        self.phase.unwrap_or(TeamPhase::Running)
    }

    pub fn is_staged(&self) -> bool {
        self.phase_or_running() == TeamPhase::Staged
    }

    pub fn is_halted(&self) -> bool {
        self.halted == Some(true)
    }

    pub fn is_escalated(&self) -> bool {
        self.escalated == Some(true)
    }

    /// The review sub-state of a staged team, absent read as awaiting review.
    pub fn review_state(&self) -> Option<PlanReviewState> {
        self.is_staged().then(|| {
            self.plan_review_state
                .unwrap_or(PlanReviewState::AwaitingReview)
        })
    }

    pub fn task(&self, id: &str) -> Option<&TeamTask> {
        self.tasks.iter().find(|task| task.id == id)
    }

    pub fn task_mut(&mut self, id: &str) -> Option<&mut TeamTask> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// The member by display name, matched on its sanitized key.
    pub fn member(&self, name: &str) -> Option<&TeamMember> {
        let key = crate::key::sanitize_key(name);
        self.members
            .iter()
            .find(|member| crate::key::sanitize_key(&member.name) == key)
    }

    pub fn member_mut(&mut self, name: &str) -> Option<&mut TeamMember> {
        let key = crate::key::sanitize_key(name);
        self.members
            .iter_mut()
            .find(|member| crate::key::sanitize_key(&member.name) == key)
    }

    /// The live (not removed) member whose session id is `id`.
    pub fn member_by_id(&self, id: &str) -> Option<&TeamMember> {
        if id.is_empty() {
            return None;
        }
        self.members
            .iter()
            .find(|member| member.id == id && !member.is_removed())
    }

    pub fn active_members(&self) -> impl Iterator<Item = &TeamMember> {
        self.members.iter().filter(|member| !member.is_removed())
    }
}
