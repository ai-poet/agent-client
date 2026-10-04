// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Pure quality-gate rules: contracts, path audit, completion, follow-up,
//! coverage and resume. Tools and persistence call these; they do no I/O.
//!
//! Translated from `src/quality-gates.ts`, plus the tool-argument parsers
//! (`parseFindings`, `parseAcceptanceResults`, `parseCommandResults`) and
//! `applyQualityFollowUp` from `src/tools.ts`. Every model-visible string is
//! kept byte-for-byte. The `is*` shape validators of the reference
//! (`isReviewFinding`, `isTaskEvidence`, `hasValidQualityTaskFields`,
//! `normalizeBlankOptionalTaskFields`, …) live in [`crate::validate`].

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::key::CAPTAIN_KEY;
use crate::types::{
    AcceptanceResult, CheckStatus, CommandResult, FindingSeverity, ReviewFinding, ReviewPolicy,
    ReviewVerdict, TaskEvidence, TaskKind, TaskRevision, TaskStatus, TeamPhase, TeamState,
    TeamTask, now_ms,
};
use crate::validate::is_valid_evidence;

pub const DEFAULT_REQUIREMENTS_MIN_ROUNDS: u32 = 1;
pub const DEFAULT_REQUIREMENTS_MAX_ROUNDS: u32 = 4;
pub const DEFAULT_CODE_MAX_ROUNDS: u32 = 3;
pub const DEFAULT_MAX_REPAIR_ATTEMPTS: u32 = 2;

/// Every kind with a structured contract (all but `work`).
pub const QUALITY_KINDS: [TaskKind; 6] = [
    TaskKind::Requirements,
    TaskKind::Implementation,
    TaskKind::Verification,
    TaskKind::Review,
    TaskKind::Repair,
    TaskKind::Integration,
];

/// Kinds that write to the workspace and therefore declare `inScope`.
pub const WRITE_KINDS: [TaskKind; 2] = [TaskKind::Implementation, TaskKind::Repair];

const OPEN_STATUSES: [TaskStatus; 3] = [
    TaskStatus::Pending,
    TaskStatus::Claimed,
    TaskStatus::InProgress,
];

pub const DEFAULT_REVIEW_ACCEPTANCE: [&str; 2] = [
    "The latest implementation meets the user goal",
    "No unresolved blocker or high findings",
];

pub const DEFAULT_REVIEW_OBJECTIVE: &str =
    "Review whether the latest implementation satisfies the user goal";

/// `/needs[_ ]revision|拒绝路径|verdict\s*=\s*needs_revision|cannot complete|不能完成|触发拒绝/iu`
static GATE_TEST_CONTRACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)needs[_ ]revision|拒绝路径|verdict\s*=\s*needs_revision|cannot complete|不能完成|触发拒绝")
        .expect("gate-test regex")
});

/// Path-like tokens worth considering as repair-scope candidates. Two shapes:
/// slash paths (`src/parser.ts`, `docs/guide.md`) and bare filenames with a
/// known code/doc extension (`README.md`, `wc.js`). An optional `:line`
/// suffix is tolerated and stripped. The extension allowlist keeps version
/// tokens (`v0.1.17`), hex hashes and prose out of the derived scope.
///
/// JS `\w` / `\d` are ASCII; the Rust classes are spelled out so they stay
/// ASCII here too. Alternation is leftmost-first in both engines.
static REPAIR_SCOPE_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?:[A-Za-z0-9_.\-]+(?:/[A-Za-z0-9_.\-]+)+",
        r"|[A-Za-z0-9_.\-]+\.(?:tsx?|jsx?|mjs|cjs|json|md|txt|ya?ml|py|rs|go|java|html?|css|scss|sh|ps1|toml|xml|sql))",
        r"(?::[0-9]+)?",
    ))
    .expect("repair-scope regex")
});
static REPAIR_SCOPE_LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":[0-9]+$").expect("line-suffix regex"));

static GIT_STATUS_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ MADRCU?!]{1,2}\s+").expect("git-status regex"));
static GIT_RENAME_TARGET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"->\s+(\S+)$").expect("rename regex"));
static SURROUNDING_QUOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^"|"$"#).expect("quote regex"));

// ---------------------------------------------------------------------------
// Result and input types
// ---------------------------------------------------------------------------

/// Where a changed path falls relative to a task's declared scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathClassification {
    InScope,
    OutOfScope,
    Undeclared,
    Illegal,
}

impl PathClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            PathClassification::InScope => "in_scope",
            PathClassification::OutOfScope => "out_of_scope",
            PathClassification::Undeclared => "undeclared",
            PathClassification::Illegal => "illegal",
        }
    }
}

impl fmt::Display for PathClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `agent_teams_create_task` arguments (camelCase on the wire, as in the
/// reference). Blank optionals should be normalized away first
/// ([`crate::validate::normalize_blank_optional_task_fields`] on the argument
/// map). A `kind` the enum does not know fails deserialization; use
/// [`parse_task_kind`] for the reference's error text.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateTaskInput {
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<TaskKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_of_scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deliverables: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub non_goals: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_finding_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_of: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_reason: Option<String>,
}

impl From<&TeamTask> for CreateTaskInput {
    /// Re-validate an existing (edited) task as if it were being created, as
    /// the reference's plan-batch editor does.
    fn from(task: &TeamTask) -> Self {
        Self {
            subject: task.subject.clone(),
            description: task.description.clone(),
            dependencies: Some(task.dependencies.clone()),
            assignee: task.assignee.clone(),
            kind: task.kind,
            round: task.round,
            objective: task.objective.clone(),
            in_scope: task.in_scope.clone(),
            out_of_scope: task.out_of_scope.clone(),
            acceptance: task.acceptance.clone(),
            verify: task.verify.clone(),
            deliverables: task.deliverables.clone(),
            non_goals: task.non_goals.clone(),
            reviewed_task_id: task.reviewed_task_id.clone(),
            source_task_id: task.source_task_id.clone(),
            source_finding_ids: task.source_finding_ids.clone(),
            coverage_of: task.coverage_of.clone(),
            resume: None,
            resume_reason: None,
        }
    }
}

/// A create request that passed the gates (`ValidateCreateTaskResult` with
/// `ok: true`).
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedTask {
    /// The normalized kind (absent reads as `work`).
    pub kind: TaskKind,
    /// The reference's partial task: subject, kind, dependencies and every
    /// contract field the input carried. `id`, `attempt`, `status` (pending)
    /// and the timestamps are placeholders for the caller to assign; review /
    /// requirements objective and acceptance are not sanitized here.
    pub task: TeamTask,
    /// The team was halted and the request carried `resume=true` with a
    /// reason: the reference returned the team with `halted: false` and no
    /// `haltedAt`. The caller applies it (see [`resume_team_state`]).
    pub clears_halt: bool,
}

/// One `agent_teams_update_task` request as the completion gate reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QualityCompletionUpdate {
    pub status: Option<TaskStatus>,
    pub output: Option<String>,
    pub verdict: Option<ReviewVerdict>,
    pub findings: Option<Vec<ReviewFinding>>,
    pub changed_paths: Option<Vec<String>>,
    pub acceptance_results: Option<Vec<AcceptanceResult>>,
    pub commands_run: Option<Vec<CommandResult>>,
    /// Only read by [`append_task_evidence`] (the reference's
    /// `QualityCompletionUpdate & { evidence_note }`).
    pub evidence_note: Option<String>,
}

/// A completion the gate refused (`QualityCompletionResult` with `ok: false`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRejection {
    pub error: String,
    /// Set when the only acceptable outcome is a different status
    /// (a failed verify command must fail the task).
    pub required_status: Option<TaskStatus>,
}

impl CompletionRejection {
    fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            required_status: None,
        }
    }
}

impl fmt::Display for CompletionRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error)
    }
}

impl std::error::Error for CompletionRejection {}

/// A follow-up task the gate wants created; [`apply_quality_follow_up`]
/// turns drafts into team tasks.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFollowUpTask {
    /// Draft-local id (`repair-round-2`) other drafts may reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub kind: TaskKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_of_scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_finding_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_task_id: Option<String>,
}

impl PlannedFollowUpTask {
    fn new(kind: TaskKind) -> Self {
        Self {
            id: None,
            kind,
            subject: None,
            assignee: None,
            dependencies: None,
            round: None,
            objective: None,
            in_scope: None,
            out_of_scope: None,
            acceptance: None,
            verify: None,
            source_task_id: None,
            source_finding_ids: None,
            reviewed_task_id: None,
        }
    }
}

/// The reference's `PlanQualityFollowUpResult`; its `tasks` mirror of
/// `created` and the `status: 'escalated'` echo of `escalated` are dropped.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PlanQualityFollowUpResult {
    pub created: Vec<PlannedFollowUpTask>,
    pub escalated: bool,
}

impl PlanQualityFollowUpResult {
    fn escalate() -> Self {
        Self {
            created: Vec::new(),
            escalated: true,
        }
    }
}

/// What [`apply_quality_follow_up`] added to the team.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppliedFollowUp {
    pub created: Vec<TeamTask>,
    pub escalated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Missing,
    InProgress,
    Passed,
    Blocked,
}

impl CoverageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageStatus::Missing => "missing",
            CoverageStatus::InProgress => "in_progress",
            CoverageStatus::Passed => "passed",
            CoverageStatus::Blocked => "blocked",
        }
    }
}

/// One goal item of the coverage matrix (status-tool JSON keys).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageRow {
    pub goal_item: String,
    pub task_ids: Vec<String>,
    pub status: CoverageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryResult {
    pub ok: bool,
    pub blockers: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeStatus {
    Resumed,
    AlreadyRunning,
    Rejected,
}

impl ResumeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ResumeStatus::Resumed => "resumed",
            ResumeStatus::AlreadyRunning => "already_running",
            ResumeStatus::Rejected => "rejected",
        }
    }
}

/// The reference's `ResumeTeamResult` without the `team` copy:
/// [`resume_team_state`] edits the team in place instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeTeamResult {
    pub ok: bool,
    pub status: ResumeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityLoopState {
    Running,
    Halted,
    Escalated,
    Deliverable,
    Blocked,
}

impl QualityLoopState {
    pub fn as_str(self) -> &'static str {
        match self {
            QualityLoopState::Running => "running",
            QualityLoopState::Halted => "halted",
            QualityLoopState::Escalated => "escalated",
            QualityLoopState::Deliverable => "deliverable",
            QualityLoopState::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityLoopSnapshot {
    pub state: QualityLoopState,
    pub halted: bool,
    pub escalated: bool,
    pub deliverable: bool,
    pub summary: String,
}

/// One task of the default full quality-delivery graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityGraphDraft {
    pub subject: String,
    pub kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub dependencies: Vec<String>,
    pub objective: String,
    pub acceptance: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_scope: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_of: Option<Vec<String>>,
}

/// Input of [`default_quality_delivery_graph`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QualityGraphInput {
    pub goal: String,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub analyst: Option<String>,
    pub tester: Option<String>,
    pub integrator: Option<String>,
}

/// Captain-only amendment payload: replacement values for contract fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContractAmendmentInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_of_scope: Option<Vec<String>>,
}

const AMENDABLE_CONTRACT_FIELDS: [&str; 5] =
    ["objective", "acceptance", "verify", "inScope", "outOfScope"];

/// Review-loop limits with every count resolved against the defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedReviewPolicy {
    pub requirements_min_rounds: u32,
    pub requirements_max_rounds: u32,
    pub code_max_rounds: u32,
    pub max_repair_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn nonempty_opt(value: Option<&str>) -> bool {
    value.is_some_and(nonempty)
}

fn nonempty_list(value: Option<&[String]>) -> bool {
    value.is_some_and(|items| !items.is_empty() && items.iter().all(|item| nonempty(item)))
}

fn is_write_kind(kind: TaskKind) -> bool {
    WRITE_KINDS.contains(&kind)
}

fn is_open_status(status: TaskStatus) -> bool {
    OPEN_STATUSES.contains(&status)
}

/// `/^[A-Za-z]:/`
fn has_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn finding_key(ids: &[String]) -> String {
    let mut sorted: Vec<&str> = ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.join(",")
}

/// `Number.isSafeInteger` over a JSON number.
fn safe_integer(value: &Value) -> Option<i64> {
    const MAX_SAFE: i64 = (1 << 53) - 1;
    if let Some(int) = value.as_i64() {
        return (-MAX_SAFE..=MAX_SAFE).contains(&int).then_some(int);
    }
    if value.is_u64() {
        return None;
    }
    let float = value.as_f64()?;
    (float.fract() == 0.0 && float.abs() <= MAX_SAFE as f64).then_some(float as i64)
}

// ---------------------------------------------------------------------------
// Kinds and policy
// ---------------------------------------------------------------------------

/// The task's kind, with absent read as `work`.
pub fn task_kind_of(task: &TeamTask) -> TaskKind {
    task.kind_or_work()
}

pub fn is_quality_kind(kind: TaskKind) -> bool {
    QUALITY_KINDS.contains(&kind)
}

/// Parse a model-supplied kind with the reference's error text.
pub fn parse_task_kind(value: &str) -> Result<TaskKind, String> {
    TaskKind::parse(value).ok_or_else(|| format!("unknown task kind \"{value}\""))
}

pub fn is_task_kind(value: &str) -> bool {
    TaskKind::parse(value).is_some()
}

pub fn is_review_verdict(value: &str) -> bool {
    ReviewVerdict::parse(value).is_some()
}

pub fn is_finding_severity(value: &str) -> bool {
    FindingSeverity::parse(value).is_some()
}

pub fn resolve_review_policy(policy: Option<&ReviewPolicy>) -> ResolvedReviewPolicy {
    ResolvedReviewPolicy {
        requirements_min_rounds: policy
            .and_then(|p| p.requirements_min_rounds)
            .unwrap_or(DEFAULT_REQUIREMENTS_MIN_ROUNDS),
        requirements_max_rounds: policy
            .and_then(|p| p.requirements_max_rounds)
            .unwrap_or(DEFAULT_REQUIREMENTS_MAX_ROUNDS),
        code_max_rounds: policy
            .and_then(|p| p.code_max_rounds)
            .unwrap_or(DEFAULT_CODE_MAX_ROUNDS),
        max_repair_attempts: policy
            .and_then(|p| p.max_repair_attempts)
            .unwrap_or(DEFAULT_MAX_REPAIR_ATTEMPTS),
        required_reviewers: policy.and_then(|p| p.required_reviewers.clone()),
    }
}

/// `isReviewPolicy` over raw JSON (`None` = absent = valid). The typed
/// equivalent is [`crate::validate::is_valid_review_policy`].
pub fn is_review_policy(value: Option<&Value>) -> bool {
    const NUMBERS: [&str; 4] = [
        "requirementsMinRounds",
        "requirementsMaxRounds",
        "codeMaxRounds",
        "maxRepairAttempts",
    ];
    let Some(value) = value else {
        return true;
    };
    let Some(record) = value.as_object() else {
        return false;
    };
    for key in NUMBERS {
        let Some(item) = record.get(key) else {
            continue;
        };
        if !safe_integer(item).is_some_and(|number| number >= 1) {
            return false;
        }
    }
    let min = record
        .get("requirementsMinRounds")
        .and_then(safe_integer)
        .unwrap_or(i64::from(DEFAULT_REQUIREMENTS_MIN_ROUNDS));
    let max = record
        .get("requirementsMaxRounds")
        .and_then(safe_integer)
        .unwrap_or(i64::from(DEFAULT_REQUIREMENTS_MAX_ROUNDS));
    if min > max {
        return false;
    }
    if let Some(reviewers) = record.get("requiredReviewers") {
        let Some(reviewers) = reviewers.as_array() else {
            return false;
        };
        if !reviewers
            .iter()
            .all(|item| item.as_str().is_some_and(nonempty))
        {
            return false;
        }
    }
    record
        .keys()
        .all(|key| NUMBERS.contains(&key.as_str()) || key == "requiredReviewers")
}

// ---------------------------------------------------------------------------
// Path rules
// ---------------------------------------------------------------------------

/// Normalize a workspace-relative POSIX path. `None` means illegal.
pub fn normalize_workspace_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('~') || has_drive_prefix(trimmed) {
        return None;
    }
    let posix = trimmed.replace('\\', "/");
    if posix.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in posix.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return None;
        }
        parts.push(part);
    }
    Some(parts.join("/"))
}

/// Whether `path` matches one scope pattern: a file pattern matches exactly,
/// a pattern ending in `/` (or `.` / `./`) matches the directory subtree.
pub fn path_matches_scope(path: &str, pattern: &str) -> bool {
    let Some(normalized_path) = normalize_workspace_path(path) else {
        return false;
    };
    let raw_pattern = pattern.trim().replace('\\', "/");
    if raw_pattern.starts_with('~')
        || raw_pattern.starts_with('/')
        || has_drive_prefix(&raw_pattern)
    {
        return false;
    }
    let directory = raw_pattern.ends_with('/');
    let Some(normalized_pattern) = normalize_workspace_path(&raw_pattern) else {
        return directory && matches!(raw_pattern.as_str(), "./" | "/" | ".");
    };
    if directory || raw_pattern == "./" || raw_pattern == "." {
        if normalized_pattern.is_empty() {
            return true;
        }
        return normalized_path == normalized_pattern
            || normalized_path.starts_with(&format!("{normalized_pattern}/"));
    }
    normalized_path == normalized_pattern
}

fn is_default_excluded(path: &str) -> bool {
    let Some(normalized) = normalize_workspace_path(path) else {
        return false;
    };
    let segments: Vec<&str> = normalized.split('/').collect();
    let base = segments.last().copied().unwrap_or("");
    if segments[0] == ".git" || segments[0] == ".dsh" {
        return true;
    }
    if base == ".env" || base.starts_with(".env.") {
        return true;
    }
    if segments.contains(&"secrets") {
        return true;
    }
    base.starts_with("id_rsa")
}

/// Audit one changed path: illegal, default-excluded / `outOfScope` (which
/// wins over `inScope`), `inScope`, or undeclared.
pub fn classify_changed_path<S: AsRef<str>>(
    path: &str,
    in_scope: &[S],
    out_of_scope: &[S],
) -> PathClassification {
    if normalize_workspace_path(path).is_none() {
        return PathClassification::Illegal;
    }
    if is_default_excluded(path) {
        return PathClassification::OutOfScope;
    }
    if out_of_scope
        .iter()
        .any(|pattern| path_matches_scope(path, pattern.as_ref()))
    {
        return PathClassification::OutOfScope;
    }
    if in_scope
        .iter()
        .any(|pattern| path_matches_scope(path, pattern.as_ref()))
    {
        return PathClassification::InScope;
    }
    PathClassification::Undeclared
}

/// Changed paths from `git status --porcelain` text (or a plain path list):
/// rename targets kept, quotes stripped, illegal paths dropped, deduplicated.
pub fn collect_changed_paths(git_status_text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for raw_line in git_status_text.split('\n') {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let mut candidate = line.to_owned();
        if GIT_STATUS_PREFIX.is_match(line) {
            candidate = match GIT_RENAME_TARGET.captures(line) {
                Some(rename) => rename[1].to_owned(),
                None => GIT_STATUS_PREFIX.replace(line, "").into_owned(),
            };
        }
        let cleaned = SURROUNDING_QUOTE.replace_all(&candidate, "");
        let Some(normalized) = normalize_workspace_path(cleaned.trim()) else {
            continue;
        };
        if seen.insert(normalized.clone()) {
            paths.push(normalized);
        }
    }
    paths
}

/// Entries of `left` that overlap any entry of `right` (either contains the
/// other, or they are equal). Empty when either side is absent.
pub fn in_scope_overlap(left: Option<&[String]>, right: Option<&[String]>) -> Vec<String> {
    let (Some(left), Some(right)) = (left, right) else {
        return Vec::new();
    };
    let mut hits: Vec<String> = Vec::new();
    for a in left {
        for b in right {
            if (path_matches_scope(a, b) || path_matches_scope(b, a) || a == b) && !hits.contains(a)
            {
                hits.push(a.clone());
            }
        }
    }
    hits
}

// ---------------------------------------------------------------------------
// Create contract
// ---------------------------------------------------------------------------

fn dependency_closure_contains(
    tasks: &[TeamTask],
    dependencies: &[String],
    target_id: &str,
) -> bool {
    let by_id: HashMap<&str, &TeamTask> =
        tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut pending: Vec<&str> = dependencies.iter().map(String::as_str).collect();
    let mut visited: HashSet<&str> = HashSet::new();
    while let Some(id) = pending.pop() {
        if visited.contains(id) {
            continue;
        }
        if id == target_id {
            return true;
        }
        visited.insert(id);
        if let Some(task) = by_id.get(id) {
            pending.extend(task.dependencies.iter().map(String::as_str));
        }
    }
    false
}

/// The create-time contract gate (`validateCreateTask`). The team is not
/// changed; see [`ValidatedTask::clears_halt`].
pub fn validate_create_task(
    team: &TeamState,
    input: &CreateTaskInput,
) -> Result<ValidatedTask, String> {
    let kind = input.kind.unwrap_or(TaskKind::Work);
    let label = kind.as_str();

    if team.is_halted() {
        let reason = input.resume_reason.as_deref().map(str::trim).unwrap_or("");
        if input.resume != Some(true) || reason.is_empty() {
            return Err(
                "team is halted; resume with a non-empty reason before create_task".to_owned(),
            );
        }
    }

    if is_quality_kind(kind) {
        if !nonempty_opt(input.objective.as_deref()) {
            return Err(format!("{label} tasks require a non-empty objective"));
        }
        if !nonempty_list(input.acceptance.as_deref()) {
            return Err(format!(
                "{label} tasks require at least one acceptance criterion"
            ));
        }
    }
    if is_write_kind(kind) {
        if !nonempty_list(input.in_scope.as_deref()) {
            return Err(format!("{label} tasks require a non-empty inScope"));
        }
        if !nonempty_list(input.verify.as_deref()) {
            return Err(format!("{label} tasks require a non-empty verify list"));
        }
    }
    if kind == TaskKind::Review {
        let Some(reviewed) = input.reviewed_task_id.as_deref().filter(|id| nonempty(id)) else {
            return Err("review tasks require reviewedTaskId".to_owned());
        };
        if team.task(reviewed).is_none() {
            return Err(format!("reviewed task \"{reviewed}\" does not exist"));
        }
    }
    if kind == TaskKind::Repair {
        let source = input.source_task_id.as_deref().filter(|id| nonempty(id));
        let Some(source) = source.filter(|_| nonempty_list(input.source_finding_ids.as_deref()))
        else {
            return Err(
                "repair tasks require sourceTaskId and at least one sourceFindingId".to_owned(),
            );
        };
        if team.task(source).is_none() {
            return Err(format!("source task \"{source}\" does not exist"));
        }
        let wanted = finding_key(input.source_finding_ids.as_deref().unwrap_or_default());
        let duplicate = team.tasks.iter().find(|item| {
            task_kind_of(item) == TaskKind::Repair
                && is_open_status(item.status)
                && item.source_task_id.as_deref() == Some(source)
                && finding_key(item.source_finding_ids.as_deref().unwrap_or_default()) == wanted
        });
        if let Some(duplicate) = duplicate {
            return Err(format!(
                "repair task {} already covers these findings; use that task instead of creating duplicate work",
                duplicate.id
            ));
        }
    }

    let dependencies = input.dependencies.clone().unwrap_or_default();
    for dependency in &dependencies {
        let Some(upstream) = team.task(dependency) else {
            return Err(format!("dependency \"{dependency}\" does not exist"));
        };
        if matches!(kind, TaskKind::Repair | TaskKind::Review)
            && matches!(upstream.status, TaskStatus::Failed | TaskStatus::Cancelled)
        {
            return Err(format!(
                "{label} must not depend on {} task \"{dependency}\"",
                upstream.status
            ));
        }
    }

    if is_write_kind(kind) && nonempty_list(input.in_scope.as_deref()) {
        for other in &team.tasks {
            if !is_write_kind(task_kind_of(other)) || !is_open_status(other.status) {
                continue;
            }
            if dependencies.contains(&other.id)
                || other
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == "pending-new")
            {
                continue;
            }
            let overlap = in_scope_overlap(input.in_scope.as_deref(), other.in_scope.as_deref());
            if !overlap.is_empty() {
                return Err(format!(
                    "inScope overlaps {} at {}; serialize these tasks or split the paths",
                    other.id,
                    overlap.join(", ")
                ));
            }
        }
    }

    if kind == TaskKind::Implementation {
        let requirements: Vec<&TeamTask> = team
            .tasks
            .iter()
            .filter(|item| task_kind_of(item) == TaskKind::Requirements)
            .collect();
        let passed = requirements.iter().any(|item| {
            item.status == TaskStatus::Completed && item.verdict == Some(ReviewVerdict::Pass)
        });
        // Planning an implementation is safe in either approval mode when its
        // dependency chain fences execution behind requirements. Scheduling
        // and claiming still wait for successful dependency completion.
        let behind_requirements = requirements
            .iter()
            .any(|item| dependency_closure_contains(&team.tasks, &dependencies, &item.id));
        if !requirements.is_empty() && !passed && !behind_requirements {
            return Err(
                "implementation must depend on a requirements task until requirements completes with verdict=pass"
                    .to_owned(),
            );
        }
    }

    let mut task = TeamTask::new(String::new(), input.subject.clone(), 0);
    task.kind = Some(kind);
    task.description = input.description.clone();
    task.assignee = input.assignee.clone();
    task.dependencies = dependencies;
    task.round = input.round;
    task.objective = input.objective.clone();
    task.in_scope = input.in_scope.clone();
    task.out_of_scope = input.out_of_scope.clone();
    task.acceptance = input.acceptance.clone();
    task.verify = input.verify.clone();
    task.deliverables = input.deliverables.clone();
    task.non_goals = input.non_goals.clone();
    task.reviewed_task_id = input.reviewed_task_id.clone();
    task.source_task_id = input.source_task_id.clone();
    task.source_finding_ids = input.source_finding_ids.clone();
    task.coverage_of = input.coverage_of.clone();
    Ok(ValidatedTask {
        kind,
        task,
        clears_halt: team.is_halted() && input.resume == Some(true),
    })
}

// ---------------------------------------------------------------------------
// Completion gate
// ---------------------------------------------------------------------------

fn status_transition_allowed(from: TaskStatus, to: TaskStatus) -> bool {
    use TaskStatus::*;
    let allowed: &[TaskStatus] = match from {
        Pending => &[Claimed, Cancelled],
        Claimed => &[InProgress, Failed, Cancelled],
        InProgress => &[Completed, Failed, Cancelled],
        Completed | Failed | Cancelled => &[],
    };
    allowed.contains(&to)
}

fn open_high_findings(findings: Option<&[ReviewFinding]>) -> usize {
    findings
        .unwrap_or_default()
        .iter()
        .filter(|finding| finding.resolved != Some(true) && finding.severity.is_high())
        .count()
}

/// Every required item passed by name (the last result for a name wins), or a
/// same-length, all-passed report: structured arrays keep the contract order,
/// so a model paraphrasing punctuation in `criterion` is still accepted.
fn results_cover<T>(
    required: Option<&[String]>,
    results: &[T],
    name: impl Fn(&T) -> &str,
    status: impl Fn(&T) -> CheckStatus,
) -> bool {
    let required = required.unwrap_or_default();
    let by_name = |wanted: &str| results.iter().rev().find(|item| name(item) == wanted);
    if required
        .iter()
        .all(|wanted| by_name(wanted).is_some_and(|item| status(item) == CheckStatus::Passed))
    {
        return true;
    }
    results.len() == required.len()
        && results
            .iter()
            .all(|item| status(item) == CheckStatus::Passed)
}

/// The completion gate (`evaluateQualityCompletion`).
pub fn evaluate_quality_completion(
    task: &TeamTask,
    update: &QualityCompletionUpdate,
) -> Result<(), CompletionRejection> {
    let next_status = update.status;
    if let Some(next) = next_status
        && next != task.status
        && !status_transition_allowed(task.status, next)
    {
        return Err(CompletionRejection::new(format!(
            "task status cannot move from \"{}\" to \"{next}\"",
            task.status
        )));
    }

    let kind = task_kind_of(task);
    if kind == TaskKind::Work {
        return Ok(());
    }
    let label = kind.as_str();

    let verdict = update.verdict.or(task.verdict);
    let findings = update.findings.as_deref().or(task.findings.as_deref());
    if matches!(kind, TaskKind::Review | TaskKind::Requirements) {
        if next_status == Some(TaskStatus::Completed) {
            let Some(verdict) = verdict else {
                return Err(CompletionRejection::new(format!(
                    "{label} cannot complete without verdict=pass"
                )));
            };
            if verdict != ReviewVerdict::Pass {
                return Err(CompletionRejection::new(format!(
                    "{label} with verdict={verdict} cannot complete"
                )));
            }
            if open_high_findings(findings) > 0 {
                return Err(CompletionRejection::new(format!(
                    "{label} pass cannot leave unresolved high/blocker findings"
                )));
            }
        }
        if next_status == Some(TaskStatus::Failed)
            && let Some(verdict @ (ReviewVerdict::NeedsRevision | ReviewVerdict::Reject)) = verdict
            && findings.unwrap_or_default().is_empty()
        {
            return Err(CompletionRejection::new(format!(
                "{label} {verdict} requires at least one finding"
            )));
        }
        return Ok(());
    }

    // implementation, repair, verification, integration
    let commands = update
        .commands_run
        .as_deref()
        .or(task.commands_run.as_deref());
    if commands.is_some_and(|items| items.iter().any(|item| item.status == CheckStatus::Failed))
        && next_status == Some(TaskStatus::Completed)
    {
        return Err(CompletionRejection {
            error: "verify failure must fail the task".to_owned(),
            required_status: Some(TaskStatus::Failed),
        });
    }
    if next_status != Some(TaskStatus::Completed) {
        return Ok(());
    }
    let acceptance_results = update
        .acceptance_results
        .as_deref()
        .or(task.acceptance_results.as_deref());
    let acceptance_ok = acceptance_results.is_some_and(|results| {
        results_cover(
            task.acceptance.as_deref(),
            results,
            |item| item.criterion.as_str(),
            |item| item.status,
        )
    });
    if !acceptance_ok {
        return Err(CompletionRejection::new(format!(
            "{label} completion requires passed acceptanceResults for every acceptance item"
        )));
    }
    let verify_ok = commands.is_some_and(|results| {
        results_cover(
            task.verify.as_deref(),
            results,
            |item| item.command.as_str(),
            |item| item.status,
        )
    });
    if !verify_ok {
        return Err(CompletionRejection::new(format!(
            "{label} completion requires a passed commandsRun entry for every verify command"
        )));
    }
    if is_write_kind(kind) {
        let Some(changed) = update
            .changed_paths
            .as_deref()
            .or(task.changed_paths.as_deref())
        else {
            return Err(CompletionRejection::new(format!(
                "{label} completion requires changedPaths"
            )));
        };
        let in_scope = task.in_scope.as_deref().unwrap_or_default();
        let out_of_scope = task.out_of_scope.as_deref().unwrap_or_default();
        for path in changed {
            let classification = classify_changed_path(path, in_scope, out_of_scope);
            if classification != PathClassification::InScope {
                return Err(CompletionRejection::new(format!(
                    "{label} cannot complete: {path} is {classification}"
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Repair scope and contract amendment
// ---------------------------------------------------------------------------

fn unresolved_findings(task: &TeamTask) -> Vec<ReviewFinding> {
    task.findings
        .iter()
        .flatten()
        .filter(|finding| finding.resolved != Some(true))
        .cloned()
        .collect()
}

/// Derive the repair round's inScope from the findings that caused it.
///
/// `finding.file` records where the problem was observed, but the fix often
/// targets a different file named in `requiredFix` (docs vs sample data,
/// config vs code). Deriving the scope from both keeps the generated repair
/// contract satisfiable. Absolute and otherwise illegal paths are dropped;
/// when nothing legal remains, the source task's own inScope (deduplicated)
/// is the fallback. Over-inclusion is accepted: inScope is an audit upper
/// bound.
pub fn repair_scope_from_findings(
    findings: &[ReviewFinding],
    fallback: Option<&[String]>,
) -> Option<Vec<String>> {
    let mut derived: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        let stripped = REPAIR_SCOPE_LINE_SUFFIX.replace(raw, "");
        if let Some(normalized) = normalize_workspace_path(&stripped)
            && !derived.contains(&normalized)
        {
            derived.push(normalized);
        }
    };
    for finding in findings {
        if let Some(file) = finding.file.as_deref().filter(|file| nonempty(file)) {
            push(file);
        }
        for found in REPAIR_SCOPE_PATH_PATTERN.find_iter(&finding.required_fix) {
            push(found.as_str());
        }
    }
    if !derived.is_empty() {
        return Some(derived);
    }
    fallback.map(|items| {
        let mut unique: Vec<String> = Vec::new();
        for item in items {
            if !unique.contains(item) {
                unique.push(item.clone());
            }
        }
        unique
    })
}

/// Controlled contract amendment (the pure rule; tooling keeps it
/// captain-only). The task `task_id` of `team` is amended in place and the
/// recorded revision returned; on any error nothing changes. Amendments
/// replace whole fields; once a review / requirements task passed judgment on
/// the task, its contract is frozen.
pub fn amend_task_contract(
    team: &mut TeamState,
    task_id: &str,
    input: &ContractAmendmentInput,
    by: &str,
    reason: &str,
) -> Result<TaskRevision, String> {
    let Some(task) = team.task(task_id) else {
        return Err(format!(
            "no task \"{task_id}\" in team \"{}\" — use agent_teams_status to list tasks",
            team.name
        ));
    };
    if !nonempty(by) {
        return Err("contract amendment requires a non-empty author identity".to_owned());
    }
    if !nonempty(reason) {
        return Err("contract amendment requires a non-empty reason".to_owned());
    }
    if task.status.is_terminal() {
        return Err(format!(
            "task {} is {}; terminal contracts are immutable",
            task.id, task.status
        ));
    }
    if task_kind_of(task) == TaskKind::Work {
        return Err(format!(
            "task {} has kind=work and no contract to amend",
            task.id
        ));
    }
    if input.objective.is_none()
        && input.acceptance.is_none()
        && input.verify.is_none()
        && input.in_scope.is_none()
        && input.out_of_scope.is_none()
    {
        return Err(format!(
            "amendment requires at least one of: {}",
            AMENDABLE_CONTRACT_FIELDS.join(", ")
        ));
    }

    let mut fields: Vec<String> = Vec::new();
    let mut previous: Map<String, Value> = Map::new();
    let mut remember = |field: &str, value: Option<Value>| {
        fields.push(field.to_owned());
        if let Some(value) = value {
            previous.insert(field.to_owned(), value);
        }
    };
    if let Some(objective) = &input.objective {
        if !nonempty(objective) {
            return Err("amended objective must be a non-empty string".to_owned());
        }
        remember("objective", task.objective.clone().map(Value::String));
    }
    let list_value = |list: &Option<Vec<String>>| {
        list.as_ref()
            .map(|items| Value::Array(items.iter().cloned().map(Value::String).collect()))
    };
    for (field, value, current) in [
        ("acceptance", &input.acceptance, &task.acceptance),
        ("verify", &input.verify, &task.verify),
    ] {
        let Some(value) = value else {
            continue;
        };
        if !nonempty_list(Some(value)) {
            return Err(format!(
                "amended {field} must be a non-empty list of non-empty strings"
            ));
        }
        remember(field, list_value(current));
    }
    for (field, value, current) in [
        ("inScope", &input.in_scope, &task.in_scope),
        ("outOfScope", &input.out_of_scope, &task.out_of_scope),
    ] {
        let Some(value) = value else {
            continue;
        };
        if !nonempty_list(Some(value)) {
            return Err(format!(
                "amended {field} must be a non-empty list of non-empty strings"
            ));
        }
        if let Some(entry) = value
            .iter()
            .find(|entry| normalize_workspace_path(entry).is_none())
        {
            return Err(format!(
                "amended {field} entry \"{entry}\" is not a workspace-relative path (absolute paths and \"..\" can never match scope patterns)"
            ));
        }
        remember(field, list_value(current));
    }
    let review_passed = team.tasks.iter().any(|item| {
        matches!(
            task_kind_of(item),
            TaskKind::Review | TaskKind::Requirements
        ) && item.reviewed_task_id.as_deref() == Some(task_id)
            && item.verdict == Some(ReviewVerdict::Pass)
    });
    if review_passed {
        return Err(format!(
            "task {task_id} already passed review; its contract is frozen"
        ));
    }

    let now = now_ms();
    let revision = TaskRevision {
        at: now,
        by: by.to_owned(),
        reason: reason.to_owned(),
        fields,
        previous,
    };
    let task = team
        .task_mut(task_id)
        .expect("the task was found above and the team was not changed since");
    if let Some(objective) = &input.objective {
        task.objective = Some(objective.clone());
    }
    if let Some(acceptance) = &input.acceptance {
        task.acceptance = Some(acceptance.clone());
    }
    if let Some(verify) = &input.verify {
        task.verify = Some(verify.clone());
    }
    if let Some(in_scope) = &input.in_scope {
        task.in_scope = Some(in_scope.clone());
    }
    if let Some(out_of_scope) = &input.out_of_scope {
        task.out_of_scope = Some(out_of_scope.clone());
    }
    task.revisions
        .get_or_insert_with(Vec::new)
        .push(revision.clone());
    task.updated_at = now;
    Ok(revision)
}

// ---------------------------------------------------------------------------
// Automatic follow-up
// ---------------------------------------------------------------------------

fn schedulable_assignee(
    preferred: Option<&str>,
    team: &TeamState,
    forbidden: Option<&str>,
) -> Option<String> {
    if let Some(preferred) = preferred
        && preferred != CAPTAIN_KEY
        && Some(preferred) != forbidden
        && let Some(live) = team
            .members
            .iter()
            .find(|member| member.name == preferred && !member.is_removed())
    {
        return Some(live.name.clone());
    }
    team.members
        .iter()
        .find(|member| {
            !member.is_removed()
                && member.name != CAPTAIN_KEY
                && Some(member.name.as_str()) != forbidden
        })
        .map(|member| member.name.clone())
}

fn repairs_of<'a>(
    team: &'a TeamState,
    source_task_id: &'a str,
    finding_ids: &[String],
) -> impl Iterator<Item = &'a TeamTask> + 'a {
    let key = finding_key(finding_ids);
    team.tasks.iter().filter(move |item| {
        task_kind_of(item) == TaskKind::Repair
            && item.source_task_id.as_deref() == Some(source_task_id)
            && finding_key(item.source_finding_ids.as_deref().unwrap_or_default()) == key
    })
}

/// Drop previous-round exclusions that intersect this generated repair's scope.
fn without_scope_conflicts(
    out_of_scope: Option<&[String]>,
    in_scope: Option<&[String]>,
) -> Option<Vec<String>> {
    let out_of_scope = out_of_scope?;
    let conflicting: HashSet<String> =
        in_scope_overlap(Some(out_of_scope), Some(in_scope.unwrap_or_default()))
            .into_iter()
            .collect();
    Some(
        out_of_scope
            .iter()
            .filter(|pattern| !conflicting.contains(*pattern))
            .cloned()
            .collect(),
    )
}

/// What a failed review / requirements task should trigger: the next
/// requirements round, a repair plus the next review, or escalation.
pub fn plan_quality_follow_up(team: &TeamState, closed: &TeamTask) -> PlanQualityFollowUpResult {
    let kind = task_kind_of(closed);
    if !matches!(kind, TaskKind::Review | TaskKind::Requirements)
        || closed.status != TaskStatus::Failed
    {
        return PlanQualityFollowUpResult::default();
    }
    if closed.verdict == Some(ReviewVerdict::Reject) {
        return PlanQualityFollowUpResult::escalate();
    }
    if closed.verdict != Some(ReviewVerdict::NeedsRevision) {
        return PlanQualityFollowUpResult::default();
    }

    let policy = resolve_review_policy(team.review_policy.as_ref());
    let next_round = closed.round.unwrap_or(1).saturating_add(1);
    let max_rounds = if kind == TaskKind::Requirements {
        policy.requirements_max_rounds
    } else {
        policy.code_max_rounds
    };
    if next_round > max_rounds {
        return PlanQualityFollowUpResult::escalate();
    }

    let findings = unresolved_findings(closed);
    if kind == TaskKind::Requirements {
        let fixes: Vec<String> = findings.iter().map(|f| f.required_fix.clone()).collect();
        let mut next = PlannedFollowUpTask::new(TaskKind::Requirements);
        next.subject = Some(format!("requirements-round-{next_round}"));
        next.assignee = closed.assignee.clone();
        next.dependencies = Some(Vec::new());
        next.round = Some(next_round);
        next.objective = Some(sanitize_review_objective(
            closed.objective.as_deref(),
            "Converge remaining open questions",
        ));
        next.acceptance = Some(sanitize_review_acceptance(Some(fixes.as_slice())));
        return PlanQualityFollowUpResult {
            created: vec![next],
            escalated: false,
        };
    }

    let Some(source_id) = closed
        .reviewed_task_id
        .clone()
        .or_else(|| closed.source_task_id.clone())
    else {
        return PlanQualityFollowUpResult::default();
    };
    let source = team.task(&source_id);
    let finding_ids: Vec<String> = findings.iter().map(|f| f.id.clone()).collect();
    if repairs_of(team, &source_id, &finding_ids).any(|item| is_open_status(item.status)) {
        return PlanQualityFollowUpResult::default();
    }
    let attempts = repairs_of(team, &source_id, &finding_ids).count();
    if attempts >= policy.max_repair_attempts as usize {
        return PlanQualityFollowUpResult::escalate();
    }

    // inScope is derived from the findings: the observed file plus any
    // workspace-relative paths referenced by the requiredFix instructions.
    let repair_scope =
        repair_scope_from_findings(&findings, source.and_then(|task| task.in_scope.as_deref()));
    let implementer =
        schedulable_assignee(source.and_then(|task| task.assignee.as_deref()), team, None);
    let repair_id = format!("repair-round-{next_round}");
    let mut repair = PlannedFollowUpTask::new(TaskKind::Repair);
    repair.id = Some(repair_id.clone());
    repair.subject = Some(repair_id.clone());
    repair.assignee = implementer.clone();
    repair.dependencies = Some(vec![source_id.clone()]);
    repair.round = Some(next_round);
    repair.objective = Some(
        source
            .and_then(|task| task.objective.clone())
            .or_else(|| closed.objective.clone())
            .unwrap_or_else(|| format!("Fix findings from {source_id}")),
    );
    repair.out_of_scope = without_scope_conflicts(
        source.and_then(|task| task.out_of_scope.as_deref()),
        repair_scope.as_deref(),
    );
    repair.in_scope = repair_scope;
    repair.verify = source.and_then(|task| task.verify.clone());
    repair.acceptance = Some(findings.iter().map(|f| f.required_fix.clone()).collect());
    repair.source_task_id = Some(source_id.clone());
    repair.source_finding_ids = Some(finding_ids);

    let preferred_reviewer = if closed.assignee != implementer {
        closed.assignee.as_deref()
    } else {
        None
    };
    let reviewer = schedulable_assignee(preferred_reviewer, team, implementer.as_deref());
    let review_id = format!("review-round-{next_round}");
    let mut review = PlannedFollowUpTask::new(TaskKind::Review);
    review.id = Some(review_id.clone());
    review.subject = Some(review_id);
    review.assignee = reviewer;
    review.dependencies = Some(vec![repair_id.clone()]);
    review.round = Some(next_round);
    review.objective = Some(sanitize_review_objective(
        closed.objective.as_deref(),
        DEFAULT_REVIEW_OBJECTIVE,
    ));
    review.acceptance = Some(sanitize_review_acceptance(closed.acceptance.as_deref()));
    review.reviewed_task_id = Some(repair_id);
    PlanQualityFollowUpResult {
        created: vec![repair, review],
        escalated: false,
    }
}

/// Plan and apply the follow-up of `closed` (pass a copy when it lives in
/// `team`): new tasks get ids `t{taskSeq}` and `attempt: 0`, draft-local ids
/// are resolved, escalation is recorded on the team, and still-pending tasks
/// that waited on `closed` are rewired to the new terminal gate.
pub fn apply_quality_follow_up(team: &mut TeamState, closed: &TeamTask) -> AppliedFollowUp {
    let planned = plan_quality_follow_up(team, closed);
    if planned.escalated {
        team.escalated = Some(true);
    }
    let existing = team.tasks.len();
    let now = now_ms();
    let mut id_by_subject: HashMap<String, String> = HashMap::new();
    let mut created: Vec<TeamTask> = Vec::new();
    for draft in planned.created {
        team.task_seq += 1;
        let id = format!("t{}", team.task_seq);
        if let Some(draft_id) = &draft.id {
            id_by_subject.insert(draft_id.clone(), id.clone());
        }
        if let Some(subject) = &draft.subject {
            id_by_subject.insert(subject.clone(), id.clone());
        }
        let dependencies = draft
            .dependencies
            .unwrap_or_default()
            .into_iter()
            .map(|dependency| {
                if team.tasks.iter().any(|item| item.id == dependency) {
                    return dependency;
                }
                id_by_subject
                    .get(&dependency)
                    .cloned()
                    .unwrap_or(dependency)
            })
            .collect();
        let subject = draft
            .subject
            .unwrap_or_else(|| format!("{}-round-{}", draft.kind, draft.round.unwrap_or(1)));
        let mut next = TeamTask::new(id, subject, now);
        next.assignee = draft.assignee;
        next.dependencies = dependencies;
        next.attempt = Some(0);
        next.kind = Some(draft.kind);
        next.round = draft.round;
        next.objective = draft.objective;
        next.in_scope = draft.in_scope;
        next.out_of_scope = draft.out_of_scope;
        next.acceptance = draft.acceptance;
        next.verify = draft.verify;
        next.source_task_id = draft.source_task_id;
        next.source_finding_ids = draft.source_finding_ids;
        next.reviewed_task_id = draft
            .reviewed_task_id
            .map(|reviewed| id_by_subject.get(&reviewed).cloned().unwrap_or(reviewed));
        team.tasks.push(next.clone());
        created.push(next);
    }
    // A staged full delivery plan may already contain downstream integration
    // work that points at the first requirements/review gate. When that gate
    // opens an automatic revision loop, move only still-pending downstream
    // edges to the new terminal gate so the approved plan can continue after
    // the repair instead of waiting forever on an intentionally failed task.
    if let Some(replacement) = created.last() {
        for task in &mut team.tasks[..existing] {
            if task.status != TaskStatus::Pending || !task.dependencies.contains(&closed.id) {
                continue;
            }
            for dependency in &mut task.dependencies {
                if *dependency == closed.id {
                    dependency.clone_from(&replacement.id);
                }
            }
            task.updated_at = now;
        }
    }
    AppliedFollowUp {
        created,
        escalated: planned.escalated,
    }
}

// ---------------------------------------------------------------------------
// Coverage, delivery, resume
// ---------------------------------------------------------------------------

pub fn build_coverage_matrix(goal_items: &[String], tasks: &[TeamTask]) -> Vec<CoverageRow> {
    goal_items
        .iter()
        .map(|goal_item| {
            let covering: Vec<&TeamTask> = tasks
                .iter()
                .filter(|item| {
                    item.coverage_of
                        .as_ref()
                        .is_some_and(|items| items.contains(goal_item))
                })
                .collect();
            let task_ids = covering.iter().map(|item| item.id.clone()).collect();
            let status = if covering.is_empty() {
                CoverageStatus::Missing
            } else if covering
                .iter()
                .any(|item| matches!(item.status, TaskStatus::Failed | TaskStatus::Cancelled))
            {
                CoverageStatus::Blocked
            } else if covering
                .iter()
                .all(|item| item.status == TaskStatus::Completed)
            {
                CoverageStatus::Passed
            } else {
                CoverageStatus::InProgress
            };
            CoverageRow {
                goal_item: goal_item.clone(),
                task_ids,
                status,
                evidence: None,
            }
        })
        .collect()
}

/// Whether the captain may report delivery, with every blocker listed.
pub fn can_declare_delivery(team: &TeamState) -> DeliveryResult {
    let mut blockers: Vec<String> = Vec::new();
    if team.phase == Some(TeamPhase::Staged) {
        blockers.push("team plan is awaiting approval".to_owned());
    }
    if team.is_halted() {
        blockers.push("team is halted".to_owned());
    }
    if team.is_escalated() {
        blockers.push("team requires escalation resolution".to_owned());
    }
    if team.tasks.is_empty() {
        blockers.push("team has no completed work".to_owned());
    }
    for item in team
        .tasks
        .iter()
        .filter(|item| !is_quality_kind(task_kind_of(item)))
    {
        if !matches!(item.status, TaskStatus::Completed | TaskStatus::Cancelled) {
            blockers.push(format!(
                "{} ({}) is not completed",
                item.id,
                task_kind_of(item)
            ));
        }
    }
    if !team.tasks.is_empty()
        && team
            .tasks
            .iter()
            .all(|item| item.status == TaskStatus::Cancelled)
    {
        blockers.push("all work was cancelled".to_owned());
    }
    let quality: Vec<&TeamTask> = team
        .tasks
        .iter()
        .filter(|item| is_quality_kind(task_kind_of(item)))
        .collect();
    let implementations: Vec<&TeamTask> = quality
        .iter()
        .copied()
        .filter(|item| is_write_kind(task_kind_of(item)))
        .collect();
    let reviews: Vec<&TeamTask> = quality
        .iter()
        .copied()
        .filter(|item| task_kind_of(item) == TaskKind::Review)
        .collect();

    for item in &quality {
        let kind = task_kind_of(item);
        match item.status {
            TaskStatus::Completed => {
                if matches!(kind, TaskKind::Review | TaskKind::Requirements)
                    && item.verdict != Some(ReviewVerdict::Pass)
                {
                    blockers.push(format!("{} completed without verdict=pass", item.id));
                }
            }
            TaskStatus::Failed => {
                let repaired = match kind {
                    TaskKind::Review => {
                        let target = item
                            .reviewed_task_id
                            .as_deref()
                            .or(item.source_task_id.as_deref());
                        quality.iter().any(|candidate| {
                            task_kind_of(candidate) == TaskKind::Repair
                                && candidate.source_task_id.as_deref() == target
                                && matches!(
                                    candidate.status,
                                    TaskStatus::Pending
                                        | TaskStatus::Claimed
                                        | TaskStatus::InProgress
                                        | TaskStatus::Completed
                                )
                        })
                    }
                    TaskKind::Requirements => quality.iter().any(|candidate| {
                        task_kind_of(candidate) == TaskKind::Requirements
                            && candidate.round.unwrap_or(1) > item.round.unwrap_or(1)
                    }),
                    _ => quality.iter().any(|candidate| {
                        task_kind_of(candidate) == TaskKind::Repair
                            && candidate.source_task_id.as_deref() == Some(item.id.as_str())
                    }),
                };
                if !repaired {
                    blockers.push(format!("{} failed without a follow-up repair", item.id));
                }
            }
            TaskStatus::Cancelled => {}
            _ => blockers.push(format!("{} ({kind}) is not completed", item.id)),
        }
    }

    if implementations
        .iter()
        .any(|item| item.status == TaskStatus::Completed)
        && !reviews.iter().any(|item| {
            item.status == TaskStatus::Completed && item.verdict == Some(ReviewVerdict::Pass)
        })
        && !blockers.iter().any(|blocker| blocker.contains("review"))
    {
        blockers.push("completed implementation has no passing review".to_owned());
    }

    for item in &implementations {
        let in_scope = item.in_scope.as_deref().unwrap_or_default();
        let out_of_scope = item.out_of_scope.as_deref().unwrap_or_default();
        for path in item.changed_paths.iter().flatten() {
            if classify_changed_path(path, in_scope, out_of_scope) != PathClassification::InScope {
                blockers.push(format!("{} has unaudited path {path}", item.id));
            }
        }
    }

    DeliveryResult {
        ok: blockers.is_empty(),
        blockers,
    }
}

/// Explicit resume: a non-empty reason clears a halt in place (`halted:
/// false`, no `haltedAt`); a running team is left untouched. Cancelled tasks
/// stay cancelled.
pub fn resume_team_state(team: &mut TeamState, reason: &str) -> ResumeTeamResult {
    if !nonempty(reason) {
        return ResumeTeamResult {
            ok: false,
            status: ResumeStatus::Rejected,
            error: Some("resume requires a non-empty reason".to_owned()),
        };
    }
    if !team.is_halted() {
        return ResumeTeamResult {
            ok: true,
            status: ResumeStatus::AlreadyRunning,
            error: None,
        };
    }
    team.halted = Some(false);
    team.halted_at = None;
    ResumeTeamResult {
        ok: true,
        status: ResumeStatus::Resumed,
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Review contract hygiene, planning text, loop state
// ---------------------------------------------------------------------------

/// Whether a contract line asks the gate to be tested (submit needs_revision
/// on purpose) rather than the work to be judged.
pub fn looks_like_gate_test_contract(value: Option<&str>) -> bool {
    value.is_some_and(|text| GATE_TEST_CONTRACT.is_match(text))
}

/// The objective trimmed, or `fallback` when blank or a gate test. Pass
/// [`DEFAULT_REVIEW_OBJECTIVE`] for the reference's default.
pub fn sanitize_review_objective(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(text) if nonempty(text) && !looks_like_gate_test_contract(Some(text)) => {
            text.trim().to_owned()
        }
        _ => fallback.to_owned(),
    }
}

/// Acceptance lines trimmed, blanks and gate tests removed; the default review
/// acceptance when nothing remains.
pub fn sanitize_review_acceptance(values: Option<&[String]>) -> Vec<String> {
    let cleaned: Vec<String> = values
        .unwrap_or_default()
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty() && !looks_like_gate_test_contract(Some(item)))
        .map(str::to_owned)
        .collect();
    if cleaned.is_empty() {
        DEFAULT_REVIEW_ACCEPTANCE
            .iter()
            .map(|item| (*item).to_owned())
            .collect()
    } else {
        cleaned
    }
}

/// The full quality-mode graph: requirements → implementation → verification
/// → review → integration.
pub fn default_quality_delivery_graph(input: &QualityGraphInput) -> Vec<QualityGraphDraft> {
    let goal = match input.goal.trim() {
        "" => "the stated user goal",
        trimmed => trimmed,
    };
    let strings = |items: &[&str]| {
        items
            .iter()
            .map(|item| (*item).to_owned())
            .collect::<Vec<_>>()
    };
    let coverage = Some(vec![goal.to_owned()]);
    let tester = input.tester.clone().or_else(|| input.implementer.clone());
    let integrator = input.integrator.clone().or_else(|| input.reviewer.clone());
    vec![
        QualityGraphDraft {
            subject: "requirements-round-1".to_owned(),
            kind: TaskKind::Requirements,
            assignee: input.analyst.clone(),
            dependencies: Vec::new(),
            objective: format!("Converge requirements for: {goal}"),
            acceptance: strings(&[
                "Open questions are closed or explicitly deferred",
                "Acceptance criteria are testable",
            ]),
            in_scope: None,
            verify: None,
            coverage_of: coverage.clone(),
        },
        QualityGraphDraft {
            subject: "implementation".to_owned(),
            kind: TaskKind::Implementation,
            assignee: input.implementer.clone(),
            dependencies: strings(&["requirements-round-1"]),
            objective: format!("Implement the approved requirements for: {goal}"),
            acceptance: strings(&["The implementation matches the approved requirements"]),
            in_scope: Some(strings(&["src/"])),
            verify: Some(strings(&["pnpm test"])),
            coverage_of: coverage.clone(),
        },
        QualityGraphDraft {
            subject: "verification".to_owned(),
            kind: TaskKind::Verification,
            assignee: tester,
            dependencies: strings(&["implementation"]),
            objective: format!("Verify the implementation of: {goal}"),
            acceptance: strings(&["Declared verification commands pass"]),
            in_scope: None,
            verify: None,
            coverage_of: coverage.clone(),
        },
        QualityGraphDraft {
            subject: "review-round-1".to_owned(),
            kind: TaskKind::Review,
            assignee: input.reviewer.clone(),
            dependencies: strings(&["verification"]),
            objective: DEFAULT_REVIEW_OBJECTIVE.to_owned(),
            acceptance: strings(&DEFAULT_REVIEW_ACCEPTANCE),
            in_scope: None,
            verify: None,
            coverage_of: coverage.clone(),
        },
        QualityGraphDraft {
            subject: "integration".to_owned(),
            kind: TaskKind::Integration,
            assignee: integrator,
            dependencies: strings(&["review-round-1"]),
            objective: format!("Confirm the team can declare delivery for: {goal}"),
            acceptance: strings(&["All required quality tasks are completed with passing reviews"]),
            in_scope: None,
            verify: None,
            coverage_of: coverage,
        },
    ]
}

/// Captain guidance for full quality-mode planning.
pub fn quality_planning_prompt() -> String {
    [
        "When the user explicitly requests full quality-mode planning, use this order unless a constraint forbids a stage: requirements → implementation → verification → review → integration.",
        "Build that entire DAG while the team is staged: an implementation may be created before requirements finishes when its dependency chain includes that requirements task. This is supported; do not wait for requirements to run and do not inspect plugin source to confirm it.",
        "A staged integration task may depend on review round 1. If that review later returns needs_revision, the system automatically rewires still-pending downstream dependencies to the generated repair + next-review gate, so keep integration in the original plan instead of omitting or manually recreating it.",
        "Derive inScope and verification commands from the actual workspace or explicit profile; never assume src/ or pnpm test.",
        "Give every quality task a contract. Review acceptance must judge the latest implementation, not whether the gate rejects needs_revision.",
        "Do not write smoke-test scripts into tasks. Do not ask reviewers to submit needs_revision on purpose.",
        "Do not claim implementation or review yourself unless the user asked the captain to take over.",
        "After a failed review, wait for the automatic repair + next review. Do not recreate that loop by hand.",
        "halted means the human stopped the team; call agent_teams_resume before creating more work. escalated means the automatic review loop hit its ceiling; that is not halt.",
    ]
    .join(" ")
}

/// The loop state the status tool reports.
pub fn describe_quality_loop(team: &TeamState) -> QualityLoopSnapshot {
    let delivery = can_declare_delivery(team);
    if team.is_halted() {
        return QualityLoopSnapshot {
            state: QualityLoopState::Halted,
            halted: true,
            escalated: team.is_escalated(),
            deliverable: false,
            summary:
                "Team is halted. Call agent_teams_resume with a reason before creating more work."
                    .to_owned(),
        };
    }
    if delivery.ok {
        return QualityLoopSnapshot {
            state: QualityLoopState::Deliverable,
            halted: false,
            escalated: team.is_escalated(),
            deliverable: true,
            summary: "All required work and quality gates passed. The captain may report delivery."
                .to_owned(),
        };
    }
    if team.is_escalated() {
        return QualityLoopSnapshot {
            state: QualityLoopState::Escalated,
            halted: false,
            escalated: true,
            deliverable: false,
            summary: "Automatic review/repair loop hit its ceiling. The team is still running; do not treat this as halt. Escalate to the user instead of inventing another needs_revision cycle.".to_owned(),
        };
    }
    let open = team.tasks.iter().any(|item| is_open_status(item.status));
    let summary = if open {
        "Work remains on the shared task list; wait for the scheduler or complete owned tasks."
            .to_owned()
    } else {
        let blockers = delivery.blockers.join("; ");
        let blockers = if blockers.is_empty() {
            "unresolved quality gates".to_owned()
        } else {
            blockers
        };
        format!("Delivery is blocked: {blockers}.")
    };
    QualityLoopSnapshot {
        state: if open {
            QualityLoopState::Running
        } else {
            QualityLoopState::Blocked
        },
        halted: false,
        escalated: false,
        deliverable: false,
        summary,
    }
}

// ---------------------------------------------------------------------------
// Supplemental evidence
// ---------------------------------------------------------------------------

/// Items of `items` not already in `existing` (nor repeated within `items`).
fn fresh_items<T: PartialEq + Clone>(items: Option<&[T]>, existing: Vec<&T>) -> Vec<T> {
    let mut seen: Vec<T> = existing.into_iter().cloned().collect();
    let mut fresh = Vec::new();
    for item in items.unwrap_or_default() {
        if seen.contains(item) {
            continue;
        }
        seen.push(item.clone());
        fresh.push(item.clone());
    }
    fresh
}

/// Append observations to a terminal task without touching its result, its
/// verdict or its completion time. `Ok(false)` when nothing new was offered
/// (repeats of this author's observations for the same attempt are dropped).
///
/// The reference compares with key-order-insensitive JSON; typed equality is
/// the same comparison here.
pub fn append_task_evidence(
    task: &mut TeamTask,
    input: &QualityCompletionUpdate,
    by: &str,
) -> Result<bool, String> {
    if !task.status.is_terminal() {
        return Err("supplemental evidence requires a terminal task".to_owned());
    }
    let changed = [
        (
            "status",
            input.status.is_some_and(|status| status != task.status),
        ),
        (
            "output",
            input.output.is_some() && input.output != task.output,
        ),
        (
            "verdict",
            input.verdict.is_some() && input.verdict != task.verdict,
        ),
        (
            "findings",
            input.findings.is_some() && input.findings != task.findings,
        ),
        (
            "changedPaths",
            input.changed_paths.is_some() && input.changed_paths != task.changed_paths,
        ),
    ];
    if let Some((key, _)) = changed.iter().find(|(_, changed)| *changed) {
        return Err(format!(
            "terminal task {} is immutable: cannot change {key}; append evidence with evidence_note, acceptanceResults or commandsRun instead. Do not reclaim or redo completed work.",
            task.id
        ));
    }
    let attempt = task.attempt.unwrap_or(0);
    let prior: Vec<&TaskEvidence> = task
        .supplemental_evidence
        .iter()
        .flatten()
        .filter(|row| row.by == by && row.attempt == attempt && row.attempt_id == task.attempt_id)
        .collect();
    let acceptance_results = fresh_items(
        input.acceptance_results.as_deref(),
        task.acceptance_results
            .iter()
            .flatten()
            .chain(
                prior
                    .iter()
                    .copied()
                    .flat_map(|row| row.acceptance_results.iter().flatten()),
            )
            .collect(),
    );
    let commands_run = fresh_items(
        input.commands_run.as_deref(),
        task.commands_run
            .iter()
            .flatten()
            .chain(
                prior
                    .iter()
                    .copied()
                    .flat_map(|row| row.commands_run.iter().flatten()),
            )
            .collect(),
    );
    let note = input
        .evidence_note
        .as_deref()
        .map(str::trim)
        .filter(|note| {
            !note.is_empty() && !prior.iter().any(|row| row.note.as_deref() == Some(*note))
        })
        .map(str::to_owned);
    if note.is_none() && acceptance_results.is_empty() && commands_run.is_empty() {
        return Ok(false);
    }
    let entry = TaskEvidence {
        at: now_ms(),
        by: by.to_owned(),
        attempt,
        attempt_id: task.attempt_id.clone(),
        note,
        acceptance_results: (!acceptance_results.is_empty()).then_some(acceptance_results),
        commands_run: (!commands_run.is_empty()).then_some(commands_run),
    };
    if !is_valid_evidence(&entry) {
        return Err("invalid supplemental evidence".to_owned());
    }
    task.supplemental_evidence
        .get_or_insert_with(Vec::new)
        .push(entry);
    Ok(true)
}

// ---------------------------------------------------------------------------
// Tool-argument parsers (from tools.ts)
// ---------------------------------------------------------------------------

fn required_text<'a>(raw: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    raw.get(key)
        .and_then(Value::as_str)
        .filter(|text| nonempty(text))
}

fn json_line(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|float| float.fract() == 0.0 && *float >= 0.0 && *float <= u64::MAX as f64)
            .map(|float| float as u64)
    })
}

fn json_exit_code(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|float| {
                float.fract() == 0.0 && *float >= i64::MIN as f64 && *float <= i64::MAX as f64
            })
            .map(|float| float as i64)
    })
}

fn check_status(raw: &Map<String, Value>) -> Option<CheckStatus> {
    raw.get("status")
        .and_then(Value::as_str)
        .and_then(CheckStatus::parse)
}

/// `findings` tool argument. `None` = absent. Ids are trimmed; a blank
/// optional `file` is omitted.
pub fn parse_findings(value: Option<&Value>) -> Result<Option<Vec<ReviewFinding>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err("findings must be an array".to_owned());
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let Some(raw) = item.as_object() else {
                return Err(format!("findings[{index}] must be an object"));
            };
            let id = required_text(raw, "id")
                .ok_or_else(|| format!("findings[{index}].id is required"))?;
            let severity = raw
                .get("severity")
                .and_then(Value::as_str)
                .and_then(FindingSeverity::parse)
                .ok_or_else(|| format!("findings[{index}].severity is invalid"))?;
            let problem = required_text(raw, "problem")
                .ok_or_else(|| format!("findings[{index}].problem is required"))?;
            let required_fix = required_text(raw, "requiredFix")
                .ok_or_else(|| format!("findings[{index}].requiredFix is required"))?;
            Ok(ReviewFinding {
                id: id.trim().to_owned(),
                severity,
                // A blank optional file must be omitted, not persisted.
                file: required_text(raw, "file").map(str::to_owned),
                line: raw.get("line").and_then(json_line),
                problem: problem.to_owned(),
                required_fix: required_fix.to_owned(),
                resolved: raw.get("resolved").and_then(Value::as_bool),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `acceptanceResults` tool argument. `None` = absent.
pub fn parse_acceptance_results(
    value: Option<&Value>,
) -> Result<Option<Vec<AcceptanceResult>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err("acceptanceResults must be an array".to_owned());
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let Some(raw) = item.as_object() else {
                return Err(format!("acceptanceResults[{index}] must be an object"));
            };
            let criterion = required_text(raw, "criterion")
                .ok_or_else(|| format!("acceptanceResults[{index}].criterion is required"))?;
            let status = check_status(raw).ok_or_else(|| {
                format!("acceptanceResults[{index}].status must be passed or failed")
            })?;
            Ok(AcceptanceResult {
                criterion: criterion.to_owned(),
                status,
                evidence: raw
                    .get("evidence")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `commandsRun` tool argument. `None` = absent.
pub fn parse_command_results(value: Option<&Value>) -> Result<Option<Vec<CommandResult>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err("commandsRun must be an array".to_owned());
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let Some(raw) = item.as_object() else {
                return Err(format!("commandsRun[{index}] must be an object"));
            };
            let command = required_text(raw, "command")
                .ok_or_else(|| format!("commandsRun[{index}].command is required"))?;
            let status = check_status(raw)
                .ok_or_else(|| format!("commandsRun[{index}].status must be passed or failed"))?;
            Ok(CommandResult {
                command: command.to_owned(),
                status,
                exit_code: raw.get("exitCode").and_then(json_exit_code),
                evidence: raw
                    .get("evidence")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

#[cfg(test)]
#[path = "quality_tests.rs"]
mod tests;
