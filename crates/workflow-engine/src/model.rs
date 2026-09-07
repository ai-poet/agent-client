//! Staged multi-agent workflows: the pure model.
//!
//! A run is a small DAG of nodes, each of which becomes one agent session
//! when it starts. This module owns the shape of that graph, the rules for
//! which node may start next, the budget that stops a runaway loop, the
//! layout geometry the canvas draws, and the on-disk store. It knows nothing
//! about sessions, providers, or GPUI — the desktop feeds it outcomes and
//! reads its decisions, so every rule here is unit-testable in isolation.
//!
//! Two rules carry most of the design:
//!
//! * **One writer at a time.** Every node of a run shares one worktree, and
//!   the per-turn checkpoint snapshots the whole tree. Two sessions writing
//!   at once would cross-attribute each other's changes, and a reader would
//!   see a half-applied tree. So a node that writes runs alone; readers may
//!   run side by side while nothing writes.
//! * **Nobody's word is final.** The planner proposes; the user approves.
//!   A node's own claim of success is checked against an optional command's
//!   exit code, and the desktop verifies file changes with git rather than
//!   trusting the transcript. This module records the outcome it is given.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use sub2api::brand;
use sub2api::global_config::atomic_write_private;

/// Summaries kept on a node are cut here; the planner gets less still.
pub const SUMMARY_CAP: usize = 2000;
/// Finished runs kept in the store beyond the ones still going.
pub const KEEP_FINISHED_RUNS: usize = 30;
/// Readers that may run side by side while nothing writes.
pub const DEFAULT_MAX_READERS: usize = 3;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// Monotonic within a process, so two runs created in the same millisecond
/// still get distinct ids.
fn next_run_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("w-{}-{n}", now_ms())
}

// --- graph ---------------------------------------------------------------

/// The worktree every node of a run shares.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceRef {
    pub path: String,
    pub branch: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Form being filled in; nothing has started.
    #[default]
    Draft,
    /// A planner call is in flight.
    Planning,
    /// Proposed nodes wait for the user.
    AwaitingApproval,
    Running,
    /// The user paused it, a budget tripped, or the planner failed.
    Paused,
    Done,
    Failed,
}

impl RunStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    #[default]
    Auto,
    UserApproval,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// The planner suggested it; the user has not approved it yet.
    #[default]
    Proposed,
    /// Approved, waiting on its dependencies.
    Pending,
    /// Dependencies met, waiting for a scheduling slot.
    Ready,
    Running,
    /// The agent is waiting on a permission or a question.
    AwaitingInput,
    Done,
    Failed,
    /// A dependency failed (or a rework trigger never fired).
    Skipped,
    Canceled,
}

impl NodeStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Skipped | Self::Canceled
        )
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Running | Self::AwaitingInput)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Node {
    /// `n1`, `n2`, … — the planner addresses dependencies by these.
    pub id: String,
    /// Free text: "Draft PRD", "Implement", "Review", "Test".
    pub role: String,
    /// `ProviderKind::id()` as a string, so this crate needs no protocol dep.
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
    /// `true` → the session starts in Plan mode.
    #[serde(default)]
    pub plan_mode: bool,
    pub prompt: String,
    /// The node changes files. At most one such node runs at a time.
    #[serde(default)]
    pub writes: bool,
    #[serde(default)]
    pub gate: Gate,
    /// Objective check run after the agent finishes; a non-zero exit fails
    /// the node no matter what the agent reported.
    #[serde(default)]
    pub check_command: Option<String>,
    // --- runtime ---
    #[serde(default)]
    pub session_id: Option<String>,
    /// The turn this node submitted. A later turn the user starts in the
    /// same session must not advance the workflow.
    #[serde(default)]
    pub expected_turn_id: Option<String>,
    #[serde(default)]
    pub status: NodeStatus,
    #[serde(default)]
    pub summary: Option<String>,
    /// What the node changed, as `git diff --numstat` text.
    #[serde(default)]
    pub numstat: Option<String>,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    #[serde(default)]
    pub finished_at_ms: Option<i64>,
    #[serde(default)]
    pub attempts: u8,
    /// Canvas position. Persisted so a node the user moved stays put.
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// The user moved it; layout leaves it alone.
    #[serde(default)]
    pub pinned: bool,
}

impl Node {
    pub fn new(id: &str, role: &str, provider_id: &str, prompt: &str) -> Self {
        Self {
            id: id.to_owned(),
            role: role.to_owned(),
            provider_id: provider_id.to_owned(),
            model: None,
            plan_mode: false,
            prompt: prompt.to_owned(),
            writes: false,
            gate: Gate::Auto,
            check_command: None,
            session_id: None,
            expected_turn_id: None,
            status: NodeStatus::Proposed,
            summary: None,
            numstat: None,
            started_at_ms: None,
            finished_at_ms: None,
            attempts: 0,
            x: 0.0,
            y: 0.0,
            pinned: false,
        }
    }

    pub fn writes(mut self, writes: bool) -> Self {
        self.writes = writes;
        self
    }

    pub fn plan_mode(mut self, plan_mode: bool) -> Self {
        self.plan_mode = plan_mode;
        self
    }

    pub fn status(mut self, status: NodeStatus) -> Self {
        self.status = status;
        self
    }

    pub fn elapsed_ms(&self, now: i64) -> Option<i64> {
        let started = self.started_at_ms?;
        Some(self.finished_at_ms.unwrap_or(now).saturating_sub(started))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// `to` needs `from` to have succeeded.
    #[default]
    DependsOn,
    /// `to` runs only when `from` failed — a rework node.
    OnFailure,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: EdgeKind,
}

impl Edge {
    pub fn depends(from: &str, to: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: EdgeKind::DependsOn,
        }
    }

    pub fn on_failure(from: &str, to: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: EdgeKind::OnFailure,
        }
    }
}

// --- budget and timeline -------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Budget {
    #[serde(default = "Budget::default_max_nodes")]
    pub max_nodes: usize,
    #[serde(default = "Budget::default_max_planner_calls")]
    pub max_planner_calls: u32,
    #[serde(default = "Budget::default_max_wall_minutes")]
    pub max_wall_minutes: u32,
    /// Pause when the gateway balance drops below this; `None` = ignore.
    #[serde(default)]
    pub min_balance_usd: Option<f64>,
}

impl Budget {
    fn default_max_nodes() -> usize {
        12
    }
    fn default_max_planner_calls() -> u32 {
        20
    }
    fn default_max_wall_minutes() -> u32 {
        120
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_nodes: Self::default_max_nodes(),
            max_planner_calls: Self::default_max_planner_calls(),
            max_wall_minutes: Self::default_max_wall_minutes(),
            min_balance_usd: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Spent {
    #[serde(default)]
    pub nodes_run: usize,
    #[serde(default)]
    pub planner_calls: u32,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetStop {
    Nodes,
    PlannerCalls,
    WallClock,
    Balance,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RunCreated,
    Planned,
    Approved,
    NodeStarted,
    NodeSettled,
    PlannerCalled,
    PlannerFailed,
    BudgetHit,
    Paused,
    Resumed,
    Stopped,
    Replanned,
    ModelFallback,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TimelineEvent {
    pub at_ms: i64,
    pub kind: EventKind,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub detail: String,
}

// --- the run -------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorkflowRun {
    pub id: String,
    pub title: String,
    pub goal: String,
    /// The session's project, as a Uuid string.
    pub project_id: String,
    /// Shared worktree; filled in once the first writing node has made it.
    #[serde(default)]
    pub workspace: Option<WorkspaceRef>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub status: RunStatus,
    /// Bumped on every re-plan or retry; async results carrying an older
    /// value are dropped.
    #[serde(default)]
    pub run_seq: u64,
    /// The planner may add nodes on its own during the run.
    #[serde(default)]
    pub planner_enabled: bool,
    /// The planner may pick agents for the nodes it adds, rather than only
    /// reusing the ones the user assigned to roles.
    #[serde(default)]
    pub planner_chooses_agents: bool,
    /// The planner said the goal is met; the run finishes when the last
    /// node settles instead of asking again.
    #[serde(default)]
    pub planner_done: bool,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub spent: Spent,
    #[serde(default)]
    pub timeline: Vec<TimelineEvent>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// What a node ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeOutcome {
    Done {
        summary: Option<String>,
        numstat: Option<String>,
    },
    Failed {
        reason: String,
    },
    Canceled,
}

/// Nodes and edges to add, from the planner or the form.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Patch {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedPatch {
    pub node_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    DuplicateId(String),
    UnknownEdgeTarget(String),
    Cycle,
    TooManyNodes { limit: usize },
    EmptyPrompt(String),
    UnknownProvider(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "node id `{id}` is already used"),
            Self::UnknownEdgeTarget(id) => write!(f, "edge refers to unknown node `{id}`"),
            Self::Cycle => write!(f, "the dependencies form a cycle"),
            Self::TooManyNodes { limit } => write!(f, "more than {limit} nodes"),
            Self::EmptyPrompt(id) => write!(f, "node `{id}` has no prompt"),
            Self::UnknownProvider(id) => write!(f, "agent `{id}` is not available"),
        }
    }
}

impl std::error::Error for PatchError {}

impl WorkflowRun {
    pub fn new(goal: &str, project_id: &str) -> Self {
        let now = now_ms();
        let title = goal
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Workflow")
            .chars()
            .take(80)
            .collect();
        Self {
            id: next_run_id(),
            title,
            goal: goal.to_owned(),
            project_id: project_id.to_owned(),
            workspace: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            status: RunStatus::Draft,
            run_seq: 0,
            planner_enabled: false,
            planner_chooses_agents: false,
            planner_done: false,
            budget: Budget::default(),
            spent: Spent::default(),
            timeline: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// The node a session belongs to.
    pub fn node_for_session(&self, session_id: &str) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|node| node.session_id.as_deref() == Some(session_id))
    }

    /// The next free `n<k>` id.
    pub fn next_node_id(&self) -> String {
        let mut k = self.nodes.len() + 1;
        loop {
            let candidate = format!("n{k}");
            if self.node(&candidate).is_none() {
                return candidate;
            }
            k += 1;
        }
    }

    pub fn dependencies_of(&self, id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|edge| edge.to == id).collect()
    }

    pub fn dependents_of(&self, id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|edge| edge.from == id).collect()
    }

    pub fn push_event(
        &mut self,
        kind: EventKind,
        node_id: Option<&str>,
        detail: impl Into<String>,
    ) {
        let at_ms = now_ms();
        self.timeline.push(TimelineEvent {
            at_ms,
            kind,
            node_id: node_id.map(str::to_owned),
            detail: detail.into(),
        });
        self.updated_at_ms = at_ms;
    }

    /// Every `DependsOn` predecessor is `Done` and every `OnFailure`
    /// predecessor is `Failed`. A node without dependencies is satisfied.
    pub fn deps_satisfied(&self, id: &str) -> bool {
        self.dependencies_of(id).into_iter().all(|edge| {
            let Some(from) = self.node(&edge.from) else {
                return false;
            };
            match edge.kind {
                EdgeKind::DependsOn => from.status == NodeStatus::Done,
                EdgeKind::OnFailure => from.status == NodeStatus::Failed,
            }
        })
    }

    /// A dependency that can no longer be met: a `DependsOn` predecessor
    /// that failed or was skipped, or an `OnFailure` trigger that succeeded.
    fn deps_impossible(&self, id: &str) -> bool {
        self.dependencies_of(id).into_iter().any(|edge| {
            let Some(from) = self.node(&edge.from) else {
                return true;
            };
            match edge.kind {
                EdgeKind::DependsOn => matches!(
                    from.status,
                    NodeStatus::Failed | NodeStatus::Skipped | NodeStatus::Canceled
                ),
                EdgeKind::OnFailure => matches!(
                    from.status,
                    NodeStatus::Done | NodeStatus::Skipped | NodeStatus::Canceled
                ),
            }
        })
    }

    /// Skip every waiting node whose dependencies can no longer be met.
    /// Cascades until nothing changes.
    fn prune_unreachable(&mut self) -> Vec<String> {
        let mut skipped = Vec::new();
        loop {
            let next = self
                .nodes
                .iter()
                .filter(|node| matches!(node.status, NodeStatus::Pending | NodeStatus::Ready))
                .filter(|node| self.deps_impossible(&node.id))
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            if next.is_empty() {
                return skipped;
            }
            for id in next {
                if let Some(node) = self.node_mut(&id) {
                    node.status = NodeStatus::Skipped;
                }
                skipped.push(id);
            }
        }
    }

    /// Move `Pending` nodes whose dependencies are met to `Ready`, and skip
    /// the ones that can never run. Returns the ids that became ready.
    pub fn promote_ready(&mut self) -> Vec<String> {
        self.prune_unreachable();
        let ready = self
            .nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Pending)
            .filter(|node| self.deps_satisfied(&node.id))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        for id in &ready {
            if let Some(node) = self.node_mut(id) {
                node.status = NodeStatus::Ready;
            }
        }
        ready
    }

    /// The nodes that may start now.
    ///
    /// A writer runs alone: nothing starts while one is live, and one starts
    /// only while nothing else is. Readers fill up to `max_readers` while no
    /// writer is live. Writers take priority over readers when both wait.
    pub fn schedulable(&self, max_readers: usize) -> Vec<String> {
        let writer_live = self
            .nodes
            .iter()
            .any(|node| node.status.is_live() && node.writes);
        if writer_live {
            return Vec::new();
        }
        let readers_live = self
            .nodes
            .iter()
            .filter(|node| node.status.is_live() && !node.writes)
            .count();
        if readers_live == 0
            && let Some(writer) = self
                .nodes
                .iter()
                .find(|node| node.status == NodeStatus::Ready && node.writes)
        {
            return vec![writer.id.clone()];
        }
        let room = max_readers.saturating_sub(readers_live);
        self.nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Ready && !node.writes)
            .take(room)
            .map(|node| node.id.clone())
            .collect()
    }

    /// Record that a node's session has started.
    pub fn mark_started(&mut self, id: &str, session_id: &str, turn_id: Option<&str>, at_ms: i64) {
        if let Some(node) = self.node_mut(id) {
            node.status = NodeStatus::Running;
            node.session_id = Some(session_id.to_owned());
            node.expected_turn_id = turn_id.map(str::to_owned);
            node.started_at_ms = Some(at_ms);
            node.finished_at_ms = None;
            node.attempts = node.attempts.saturating_add(1);
        }
        self.spent.nodes_run += 1;
        if self.spent.started_at_ms.is_none() {
            self.spent.started_at_ms = Some(at_ms);
        }
        self.updated_at_ms = at_ms;
    }

    pub fn mark_awaiting_input(&mut self, id: &str, awaiting: bool) {
        if let Some(node) = self.node_mut(id)
            && node.status.is_live()
        {
            node.status = if awaiting {
                NodeStatus::AwaitingInput
            } else {
                NodeStatus::Running
            };
        }
    }

    /// Settle a node. A failure skips everything downstream that depended
    /// on it succeeding; rework nodes hanging off `OnFailure` edges stay.
    pub fn settle_node(&mut self, id: &str, outcome: NodeOutcome, at_ms: i64) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        node.finished_at_ms = Some(at_ms);
        match outcome {
            NodeOutcome::Done { summary, numstat } => {
                node.status = NodeStatus::Done;
                node.summary = summary.map(|text| truncate(&text, SUMMARY_CAP));
                node.numstat = numstat;
            }
            NodeOutcome::Failed { reason } => {
                node.status = NodeStatus::Failed;
                node.summary = Some(truncate(&reason, SUMMARY_CAP));
            }
            NodeOutcome::Canceled => node.status = NodeStatus::Canceled,
        }
        self.updated_at_ms = at_ms;
        self.prune_unreachable();
    }

    /// Put a settled node back to `Pending` for another attempt, and every
    /// node downstream of it with it. Bumps `run_seq`.
    pub fn reset_from(&mut self, id: &str) -> Vec<String> {
        let mut affected = vec![id.to_owned()];
        let mut queue = VecDeque::from([id.to_owned()]);
        while let Some(current) = queue.pop_front() {
            for edge in self.dependents_of(&current) {
                if !affected.contains(&edge.to) {
                    affected.push(edge.to.clone());
                    queue.push_back(edge.to.clone());
                }
            }
        }
        for node_id in &affected {
            if let Some(node) = self.node_mut(node_id)
                && !node.status.is_live()
            {
                node.status = NodeStatus::Pending;
                node.session_id = None;
                node.expected_turn_id = None;
                node.summary = None;
                node.numstat = None;
                node.started_at_ms = None;
                node.finished_at_ms = None;
            }
        }
        self.run_seq += 1;
        affected
    }

    /// `Some` once every node has settled: `Done` unless a failure went
    /// unhandled — a `Failed` node with no `OnFailure` successor that
    /// itself finished `Done`.
    pub fn terminal_status(&self) -> Option<RunStatus> {
        if self.nodes.is_empty() {
            return None;
        }
        if self.nodes.iter().any(|node| !node.status.is_terminal()) {
            return None;
        }
        let unhandled = self
            .nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Failed)
            .any(|failed| {
                !self.dependents_of(&failed.id).into_iter().any(|edge| {
                    edge.kind == EdgeKind::OnFailure
                        && self
                            .node(&edge.to)
                            .is_some_and(|rework| rework.status == NodeStatus::Done)
                })
            });
        Some(if unhandled {
            RunStatus::Failed
        } else {
            RunStatus::Done
        })
    }

    pub fn has_proposed(&self) -> bool {
        self.nodes
            .iter()
            .any(|node| node.status == NodeStatus::Proposed)
    }

    /// `Proposed` → `Pending`. Returns how many moved.
    pub fn approve_proposed(&mut self) -> usize {
        let mut count = 0;
        for node in &mut self.nodes {
            if node.status == NodeStatus::Proposed {
                node.status = NodeStatus::Pending;
                count += 1;
            }
        }
        if count > 0 {
            self.updated_at_ms = now_ms();
        }
        count
    }

    /// Why the run should pause, if it should.
    pub fn budget_exceeded(&self, now_ms: i64, balance_usd: Option<f64>) -> Option<BudgetStop> {
        if self.spent.nodes_run >= self.budget.max_nodes {
            return Some(BudgetStop::Nodes);
        }
        if self.spent.planner_calls >= self.budget.max_planner_calls {
            return Some(BudgetStop::PlannerCalls);
        }
        if let Some(started) = self.spent.started_at_ms {
            let limit_ms = i64::from(self.budget.max_wall_minutes) * 60_000;
            if now_ms.saturating_sub(started) >= limit_ms {
                return Some(BudgetStop::WallClock);
            }
        }
        if let (Some(floor), Some(balance)) = (self.budget.min_balance_usd, balance_usd)
            && balance < floor
        {
            return Some(BudgetStop::Balance);
        }
        None
    }

    /// Add nodes and edges after validating them against what is already
    /// there. Added nodes enter as `Proposed`; the caller approves them.
    pub fn apply_patch(
        &mut self,
        mut patch: Patch,
        allowed_providers: &[&str],
    ) -> Result<AppliedPatch, PatchError> {
        let limit = self.budget.max_nodes;
        if self.nodes.len() + patch.nodes.len() > limit {
            return Err(PatchError::TooManyNodes { limit });
        }
        let mut ids: BTreeSet<&str> = self.nodes.iter().map(|node| node.id.as_str()).collect();
        for node in &patch.nodes {
            if !ids.insert(node.id.as_str()) {
                return Err(PatchError::DuplicateId(node.id.clone()));
            }
            if node.prompt.trim().is_empty() {
                return Err(PatchError::EmptyPrompt(node.id.clone()));
            }
            if !allowed_providers.contains(&node.provider_id.as_str()) {
                return Err(PatchError::UnknownProvider(node.provider_id.clone()));
            }
        }
        for edge in &patch.edges {
            for endpoint in [&edge.from, &edge.to] {
                if !ids.contains(endpoint.as_str()) {
                    return Err(PatchError::UnknownEdgeTarget(endpoint.clone()));
                }
            }
        }
        let all_edges = self
            .edges
            .iter()
            .chain(patch.edges.iter())
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect::<Vec<_>>();
        if has_cycle(&ids, &all_edges) {
            return Err(PatchError::Cycle);
        }
        let node_ids = patch
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        for node in &mut patch.nodes {
            node.status = NodeStatus::Proposed;
            node.session_id = None;
            node.expected_turn_id = None;
        }
        self.nodes.append(&mut patch.nodes);
        self.edges.append(&mut patch.edges);
        self.updated_at_ms = now_ms();
        Ok(AppliedPatch { node_ids })
    }

    pub fn budget_line(&self, now_ms: i64) -> BudgetLine {
        BudgetLine {
            nodes_run: self.spent.nodes_run,
            max_nodes: self.budget.max_nodes,
            planner_calls: self.spent.planner_calls,
            max_planner_calls: self.budget.max_planner_calls,
            elapsed_ms: self
                .spent
                .started_at_ms
                .map(|started| now_ms.saturating_sub(started))
                .unwrap_or_default(),
            max_wall_ms: i64::from(self.budget.max_wall_minutes) * 60_000,
        }
    }
}

/// Figures for the header's budget readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetLine {
    pub nodes_run: usize,
    pub max_nodes: usize,
    pub planner_calls: u32,
    pub max_planner_calls: u32,
    pub elapsed_ms: i64,
    pub max_wall_ms: i64,
}

fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(cap.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Kahn's algorithm: a cycle leaves nodes with a non-zero in-degree.
fn has_cycle(ids: &BTreeSet<&str>, edges: &[(&str, &str)]) -> bool {
    let mut indegree: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (from, to) in edges {
        if let Some(count) = indegree.get_mut(to) {
            *count += 1;
        }
        outgoing.entry(from).or_default().push(to);
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut seen = 0;
    while let Some(current) = queue.pop_front() {
        seen += 1;
        if let Some(targets) = outgoing.get(current) {
            for target in targets {
                if let Some(count) = indegree.get_mut(target) {
                    *count -= 1;
                    if *count == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }
    seen != ids.len()
}

// --- layout --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    pub col_width: f32,
    pub row_height: f32,
    pub node_w: f32,
    pub node_h: f32,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            col_width: 260.0,
            row_height: 120.0,
            node_w: 200.0,
            node_h: 88.0,
        }
    }
}

/// A node's column: the longest dependency path leading to it.
fn layer_of(
    run: &WorkflowRun,
    id: &str,
    memo: &mut BTreeMap<String, usize>,
    stack: &mut Vec<String>,
) -> usize {
    if let Some(layer) = memo.get(id) {
        return *layer;
    }
    // A cycle cannot exist after `apply_patch`, but never recurse forever
    // on a hand-edited file.
    if stack.iter().any(|entry| entry == id) {
        return 0;
    }
    stack.push(id.to_owned());
    let layer = run
        .dependencies_of(id)
        .into_iter()
        .map(|edge| layer_of(run, &edge.from, memo, stack) + 1)
        .max()
        .unwrap_or(0);
    stack.pop();
    memo.insert(id.to_owned(), layer);
    layer
}

/// Layered layout: one column per dependency depth, nodes of a column
/// stacked in their run order and the columns vertically centred on the
/// tallest one. Pinned nodes keep their position. Returns the moved ids.
pub fn layout(run: &mut WorkflowRun, options: &LayoutOptions) -> Vec<String> {
    let mut memo = BTreeMap::new();
    let mut stack = Vec::new();
    let mut columns: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for node in &run.nodes {
        let layer = layer_of(run, &node.id, &mut memo, &mut stack);
        columns.entry(layer).or_default().push(node.id.clone());
    }
    let tallest = columns.values().map(Vec::len).max().unwrap_or(0);
    let mut moved = Vec::new();
    for (layer, ids) in &columns {
        let offset = (tallest - ids.len()) as f32 * options.row_height / 2.0;
        for (row, id) in ids.iter().enumerate() {
            let Some(node) = run.node_mut(id) else {
                continue;
            };
            if node.pinned {
                continue;
            }
            node.x = *layer as f32 * options.col_width;
            node.y = offset + row as f32 * options.row_height;
            moved.push(id.clone());
        }
    }
    moved
}

/// Control points of the cubic Bézier drawn for one edge, from the right
/// edge of `from` to the left edge of `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeCurve {
    pub start: (f32, f32),
    pub c1: (f32, f32),
    pub c2: (f32, f32),
    pub end: (f32, f32),
}

pub fn edge_curve(from: &Node, to: &Node, options: &LayoutOptions) -> EdgeCurve {
    let start = (from.x + options.node_w, from.y + options.node_h / 2.0);
    let end = (to.x, to.y + options.node_h / 2.0);
    let reach = ((end.0 - start.0) * 0.5).max(40.0);
    EdgeCurve {
        start,
        c1: (start.0 + reach, start.1),
        c2: (end.0 - reach, end.1),
        end,
    }
}

/// The three corners of the arrowhead at the curve's end.
pub fn arrow_head(curve: &EdgeCurve, size: f32) -> [(f32, f32); 3] {
    let (dx, dy) = (curve.end.0 - curve.c2.0, curve.end.1 - curve.c2.1);
    let length = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let (px, py) = (-uy, ux);
    let base = (curve.end.0 - ux * size, curve.end.1 - uy * size);
    let half = size * 0.5;
    [
        curve.end,
        (base.0 + px * half, base.1 + py * half),
        (base.0 - px * half, base.1 - py * half),
    ]
}

// --- templates -----------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TemplateRole {
    pub role: String,
    pub plan_mode: bool,
    pub writes: bool,
    /// Seed for the node's prompt; the goal is appended by the desktop.
    pub prompt_hint: String,
    pub gate: Gate,
    /// Index of the role this one depends on, and how.
    #[serde(default)]
    pub after: Option<(usize, EdgeKind)>,
    #[serde(default)]
    pub check_command: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub roles: Vec<TemplateRole>,
}

impl Template {
    /// Turn the template into nodes and edges, one provider per role.
    /// `providers[i]` is the agent id for `roles[i]`.
    pub fn instantiate(&self, providers: &[String], goal: &str) -> Patch {
        let mut patch = Patch::default();
        for (index, role) in self.roles.iter().enumerate() {
            let id = format!("n{}", index + 1);
            let provider = providers.get(index).cloned().unwrap_or_default();
            let prompt = format!("{}\n\nGoal:\n{}", role.prompt_hint, goal.trim());
            let mut node = Node::new(&id, &role.role, &provider, &prompt)
                .writes(role.writes)
                .plan_mode(role.plan_mode);
            node.gate = role.gate;
            node.check_command = role.check_command.clone();
            patch.nodes.push(node);
            if let Some((after, kind)) = role.after {
                patch.edges.push(Edge {
                    from: format!("n{}", after + 1),
                    to: id,
                    kind,
                });
            }
        }
        patch
    }
}

fn role(
    role: &str,
    plan_mode: bool,
    writes: bool,
    prompt_hint: &str,
    after: Option<(usize, EdgeKind)>,
) -> TemplateRole {
    TemplateRole {
        role: role.to_owned(),
        plan_mode,
        writes,
        prompt_hint: prompt_hint.to_owned(),
        gate: Gate::Auto,
        after,
        check_command: None,
    }
}

pub fn builtin_templates() -> Vec<Template> {
    vec![
        Template {
            id: "prd-implement-review".to_owned(),
            name: "PRD → Implement → Review".to_owned(),
            roles: vec![
                role(
                    "Draft PRD",
                    true,
                    true,
                    "Write a concise product requirements document for the goal below into docs/prd.md: scope, user-facing behaviour, acceptance criteria, and what is explicitly out of scope. Do not write code.",
                    None,
                ),
                role(
                    "Implement",
                    false,
                    true,
                    "Implement the requirements in docs/prd.md. Keep changes focused, follow the project's conventions, and finish with a short summary of what you changed.",
                    Some((0, EdgeKind::DependsOn)),
                ),
                role(
                    "Review",
                    true,
                    false,
                    "Review the uncommitted changes in this worktree against docs/prd.md. Report bugs, missing acceptance criteria, and leftovers. Do not edit files; end with a clear verdict.",
                    Some((1, EdgeKind::DependsOn)),
                ),
            ],
        },
        Template {
            id: "implement-test-fix".to_owned(),
            name: "Implement → Test → Fix".to_owned(),
            roles: vec![
                role(
                    "Implement",
                    false,
                    true,
                    "Implement the goal below. Keep changes focused and finish with a short summary.",
                    None,
                ),
                role(
                    "Test",
                    false,
                    true,
                    "Write or extend tests for the change in this worktree and run the project's test suite. Report exactly what fails.",
                    Some((0, EdgeKind::DependsOn)),
                ),
                role(
                    "Fix",
                    false,
                    true,
                    "Tests failed in the previous step. Read the failures reported below, fix the code (not the tests, unless they are wrong), and re-run the suite.",
                    Some((1, EdgeKind::OnFailure)),
                ),
            ],
        },
        Template {
            id: "dual-review".to_owned(),
            name: "Implement → Two reviews → Summary".to_owned(),
            roles: vec![
                role(
                    "Implement",
                    false,
                    true,
                    "Implement the goal below. Keep changes focused and finish with a short summary.",
                    None,
                ),
                role(
                    "Review A",
                    true,
                    false,
                    "Review the uncommitted changes in this worktree for correctness and edge cases. Do not edit files.",
                    Some((0, EdgeKind::DependsOn)),
                ),
                role(
                    "Review B",
                    true,
                    false,
                    "Review the uncommitted changes in this worktree for readability, naming, and consistency with the codebase. Do not edit files.",
                    Some((0, EdgeKind::DependsOn)),
                ),
                role(
                    "Summarize",
                    true,
                    false,
                    "Combine the two reviews below into one prioritized list of changes to make. Do not edit files.",
                    Some((1, EdgeKind::DependsOn)),
                ),
            ],
        },
    ]
}

// --- store ---------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct WorkflowStore {
    #[serde(default)]
    pub runs: Vec<WorkflowRun>,
}

impl WorkflowStore {
    pub fn get(&self, id: &str) -> Option<&WorkflowRun> {
        self.runs.iter().find(|run| run.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut WorkflowRun> {
        self.runs.iter_mut().find(|run| run.id == id)
    }

    pub fn upsert(&mut self, run: WorkflowRun) {
        match self.runs.iter_mut().find(|existing| existing.id == run.id) {
            Some(existing) => *existing = run,
            None => self.runs.push(run),
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.runs.len();
        self.runs.retain(|run| run.id != id);
        self.runs.len() != before
    }

    /// Runs newest first.
    pub fn sorted(&self) -> Vec<&WorkflowRun> {
        let mut runs: Vec<&WorkflowRun> = self.runs.iter().collect();
        runs.sort_by_key(|run| std::cmp::Reverse(run.updated_at_ms));
        runs
    }

    /// Keep every unfinished run and the `keep` most recent finished ones.
    pub fn trim(&mut self, keep: usize) {
        let mut finished: Vec<(i64, String)> = self
            .runs
            .iter()
            .filter(|run| run.status.is_finished())
            .map(|run| (run.updated_at_ms, run.id.clone()))
            .collect();
        finished.sort_by_key(|(updated, _)| std::cmp::Reverse(*updated));
        let drop: BTreeSet<String> = finished.into_iter().skip(keep).map(|(_, id)| id).collect();
        self.runs.retain(|run| !drop.contains(&run.id));
    }

    /// After a restart no session is still running: mark live nodes as
    /// interrupted failures the user can retry, and pause their runs. A
    /// planner call in flight is lost the same way.
    pub fn reconcile_after_restart(&mut self, now_ms: i64) -> usize {
        let mut interrupted = 0;
        for run in &mut self.runs {
            match run.status {
                RunStatus::Running | RunStatus::Planning => {}
                _ => continue,
            }
            let live: Vec<String> = run
                .nodes
                .iter()
                .filter(|node| node.status.is_live())
                .map(|node| node.id.clone())
                .collect();
            for id in &live {
                run.settle_node(
                    id,
                    NodeOutcome::Failed {
                        reason: "interrupted by restart".to_owned(),
                    },
                    now_ms,
                );
                run.push_event(EventKind::Interrupted, Some(id), "restart");
                interrupted += 1;
            }
            for node in &mut run.nodes {
                if node.status == NodeStatus::Ready {
                    node.status = NodeStatus::Pending;
                }
            }
            run.status = RunStatus::Paused;
            run.updated_at_ms = now_ms;
        }
        interrupted
    }
}

pub fn config_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("workflows.json"))
}

pub fn load() -> WorkflowStore {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(store: &WorkflowStore) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut encoded =
        serde_json::to_string_pretty(store).context("could not encode workflow store")?;
    encoded.push('\n');
    atomic_write_private(&path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENTS: &[&str] = &["claude", "codex"];

    fn run_with(nodes: Vec<Node>, edges: Vec<Edge>) -> WorkflowRun {
        let mut run = WorkflowRun::new("goal", "project");
        run.apply_patch(Patch { nodes, edges }, AGENTS)
            .expect("valid patch");
        run.approve_proposed();
        run
    }

    fn node(id: &str, writes: bool) -> Node {
        Node::new(id, id, "claude", "do it").writes(writes)
    }

    fn set(run: &mut WorkflowRun, id: &str, status: NodeStatus) {
        run.node_mut(id).unwrap().status = status;
    }

    #[test]
    fn deps_satisfied_requires_all_depends_on() {
        let mut run = run_with(
            vec![node("a", true), node("b", true), node("c", false)],
            vec![Edge::depends("a", "c"), Edge::depends("b", "c")],
        );
        assert!(run.deps_satisfied("a"), "a source node is satisfied");
        assert!(!run.deps_satisfied("c"));
        set(&mut run, "a", NodeStatus::Done);
        assert!(!run.deps_satisfied("c"));
        set(&mut run, "b", NodeStatus::Done);
        assert!(run.deps_satisfied("c"));
    }

    #[test]
    fn on_failure_edge_fires_only_on_failure() {
        let mut run = run_with(
            vec![node("test", true), node("fix", true)],
            vec![Edge::on_failure("test", "fix")],
        );
        set(&mut run, "test", NodeStatus::Done);
        assert!(!run.deps_satisfied("fix"));
        set(&mut run, "test", NodeStatus::Failed);
        assert!(run.deps_satisfied("fix"));
    }

    #[test]
    fn promote_ready_moves_only_satisfied() {
        let mut run = run_with(
            vec![node("a", true), node("b", true)],
            vec![Edge::depends("a", "b")],
        );
        assert_eq!(run.promote_ready(), vec!["a".to_owned()]);
        assert_eq!(run.node("b").unwrap().status, NodeStatus::Pending);
        set(&mut run, "a", NodeStatus::Done);
        assert_eq!(run.promote_ready(), vec!["b".to_owned()]);
    }

    #[test]
    fn promote_ready_skips_rework_node_when_trigger_succeeded() {
        let mut run = run_with(
            vec![node("test", true), node("fix", true), node("ship", true)],
            vec![
                Edge::on_failure("test", "fix"),
                Edge::depends("test", "ship"),
            ],
        );
        set(&mut run, "test", NodeStatus::Done);
        run.promote_ready();
        assert_eq!(run.node("fix").unwrap().status, NodeStatus::Skipped);
        assert_eq!(run.node("ship").unwrap().status, NodeStatus::Ready);
    }

    #[test]
    fn schedulable_allows_one_writer() {
        let mut run = run_with(vec![node("a", true), node("b", true)], vec![]);
        run.promote_ready();
        assert_eq!(run.schedulable(3), vec!["a".to_owned()]);
    }

    #[test]
    fn schedulable_blocks_everything_while_writer_live() {
        let mut run = run_with(vec![node("w", true), node("r", false)], vec![]);
        run.promote_ready();
        set(&mut run, "w", NodeStatus::Running);
        assert!(run.schedulable(3).is_empty());
    }

    #[test]
    fn schedulable_blocks_writer_while_readers_live() {
        let mut run = run_with(vec![node("r", false), node("w", true)], vec![]);
        run.promote_ready();
        set(&mut run, "r", NodeStatus::Running);
        assert!(
            run.schedulable(3).is_empty(),
            "a writer must not join live readers"
        );
    }

    #[test]
    fn schedulable_prefers_writer_over_waiting_readers() {
        let mut run = run_with(vec![node("r", false), node("w", true)], vec![]);
        run.promote_ready();
        assert_eq!(run.schedulable(3), vec!["w".to_owned()]);
    }

    #[test]
    fn schedulable_caps_readers() {
        let mut run = run_with(
            (1..=5).map(|i| node(&format!("r{i}"), false)).collect(),
            vec![],
        );
        run.promote_ready();
        assert_eq!(run.schedulable(3).len(), 3);
        set(&mut run, "r1", NodeStatus::Running);
        set(&mut run, "r2", NodeStatus::AwaitingInput);
        assert_eq!(run.schedulable(3), vec!["r3".to_owned()]);
    }

    #[test]
    fn settle_failed_skips_downstream_but_not_on_failure_targets() {
        let mut run = run_with(
            vec![
                node("test", true),
                node("ship", true),
                node("fix", true),
                node("after", false),
            ],
            vec![
                Edge::depends("test", "ship"),
                Edge::on_failure("test", "fix"),
                Edge::depends("ship", "after"),
            ],
        );
        run.promote_ready();
        run.settle_node(
            "test",
            NodeOutcome::Failed {
                reason: "boom".into(),
            },
            1,
        );
        assert_eq!(run.node("ship").unwrap().status, NodeStatus::Skipped);
        assert_eq!(
            run.node("after").unwrap().status,
            NodeStatus::Skipped,
            "cascades"
        );
        assert_eq!(run.node("fix").unwrap().status, NodeStatus::Pending);
        assert_eq!(run.promote_ready(), vec!["fix".to_owned()]);
        assert_eq!(run.node("test").unwrap().summary.as_deref(), Some("boom"));
    }

    #[test]
    fn terminal_status_done_when_all_terminal() {
        let mut run = run_with(
            vec![node("a", true), node("b", false)],
            vec![Edge::depends("a", "b")],
        );
        assert_eq!(run.terminal_status(), None);
        set(&mut run, "a", NodeStatus::Done);
        set(&mut run, "b", NodeStatus::Done);
        assert_eq!(run.terminal_status(), Some(RunStatus::Done));
    }

    #[test]
    fn terminal_status_failed_when_failure_unhandled() {
        let mut run = run_with(
            vec![node("a", true), node("b", false)],
            vec![Edge::depends("a", "b")],
        );
        run.promote_ready();
        run.settle_node("a", NodeOutcome::Failed { reason: "x".into() }, 1);
        assert_eq!(run.terminal_status(), Some(RunStatus::Failed));
    }

    #[test]
    fn terminal_status_done_when_rework_handled_failure() {
        let mut run = run_with(
            vec![node("test", true), node("fix", true)],
            vec![Edge::on_failure("test", "fix")],
        );
        run.promote_ready();
        run.settle_node("test", NodeOutcome::Failed { reason: "x".into() }, 1);
        run.promote_ready();
        run.settle_node(
            "fix",
            NodeOutcome::Done {
                summary: None,
                numstat: None,
            },
            2,
        );
        assert_eq!(run.terminal_status(), Some(RunStatus::Done));
    }

    #[test]
    fn budget_stops_on_each_dimension() {
        let mut run = run_with(vec![node("a", true)], vec![]);
        run.budget = Budget {
            max_nodes: 2,
            max_planner_calls: 1,
            max_wall_minutes: 1,
            min_balance_usd: Some(5.0),
        };
        assert_eq!(run.budget_exceeded(0, None), None);
        run.spent.nodes_run = 2;
        assert_eq!(run.budget_exceeded(0, None), Some(BudgetStop::Nodes));
        run.spent.nodes_run = 0;
        run.spent.planner_calls = 1;
        assert_eq!(run.budget_exceeded(0, None), Some(BudgetStop::PlannerCalls));
        run.spent.planner_calls = 0;
        run.spent.started_at_ms = Some(0);
        assert_eq!(
            run.budget_exceeded(60_000, None),
            Some(BudgetStop::WallClock)
        );
        assert_eq!(
            run.budget_exceeded(10, Some(4.0)),
            Some(BudgetStop::Balance)
        );
        assert_eq!(run.budget_exceeded(10, Some(6.0)), None);
        assert_eq!(
            run.budget_exceeded(10, None),
            None,
            "unknown balance never stops"
        );
    }

    #[test]
    fn apply_patch_rejects_cycle() {
        let mut run = WorkflowRun::new("g", "p");
        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![node("a", true), node("b", true)],
                    edges: vec![Edge::depends("a", "b"), Edge::depends("b", "a")],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::Cycle);
        assert!(
            run.nodes.is_empty(),
            "a rejected patch leaves nothing behind"
        );
    }

    #[test]
    fn apply_patch_rejects_cycle_through_existing_nodes() {
        let mut run = run_with(
            vec![node("a", true), node("b", true)],
            vec![Edge::depends("a", "b")],
        );
        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![node("c", true)],
                    edges: vec![Edge::depends("b", "c"), Edge::depends("c", "a")],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::Cycle);
        assert_eq!(run.nodes.len(), 2);
    }

    #[test]
    fn apply_patch_rejects_duplicate_id() {
        let mut run = run_with(vec![node("a", true)], vec![]);
        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![node("a", false)],
                    edges: vec![],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::DuplicateId("a".into()));
    }

    #[test]
    fn apply_patch_rejects_unknown_edge_target() {
        let mut run = WorkflowRun::new("g", "p");
        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![node("a", true)],
                    edges: vec![Edge::depends("a", "zzz")],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::UnknownEdgeTarget("zzz".into()));
    }

    #[test]
    fn apply_patch_rejects_over_max_nodes_and_bad_provider_and_empty_prompt() {
        let mut run = WorkflowRun::new("g", "p");
        run.budget.max_nodes = 1;
        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![node("a", true), node("b", true)],
                    edges: vec![],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::TooManyNodes { limit: 1 });

        let mut run = WorkflowRun::new("g", "p");
        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![Node::new("a", "a", "gemini", "x")],
                    edges: vec![],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::UnknownProvider("gemini".into()));

        let error = run
            .apply_patch(
                Patch {
                    nodes: vec![Node::new("a", "a", "claude", "  ")],
                    edges: vec![],
                },
                AGENTS,
            )
            .unwrap_err();
        assert_eq!(error, PatchError::EmptyPrompt("a".into()));
    }

    #[test]
    fn apply_patch_forces_proposed_and_approve_promotes_all() {
        let mut run = WorkflowRun::new("g", "p");
        let applied = run
            .apply_patch(
                Patch {
                    nodes: vec![node("a", true).status(NodeStatus::Done), node("b", false)],
                    edges: vec![],
                },
                AGENTS,
            )
            .unwrap();
        assert_eq!(applied.node_ids, vec!["a".to_owned(), "b".to_owned()]);
        assert!(
            run.nodes
                .iter()
                .all(|node| node.status == NodeStatus::Proposed)
        );
        assert!(run.has_proposed());
        assert_eq!(run.approve_proposed(), 2);
        assert!(!run.has_proposed());
        assert!(
            run.nodes
                .iter()
                .all(|node| node.status == NodeStatus::Pending)
        );
    }

    #[test]
    fn mark_started_counts_and_records_turn() {
        let mut run = run_with(vec![node("a", true)], vec![]);
        run.promote_ready();
        run.mark_started("a", "sess-1", Some("turn-1"), 100);
        let a = run.node("a").unwrap();
        assert_eq!(a.status, NodeStatus::Running);
        assert_eq!(a.session_id.as_deref(), Some("sess-1"));
        assert_eq!(a.expected_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(a.attempts, 1);
        assert_eq!(run.spent.nodes_run, 1);
        assert_eq!(run.spent.started_at_ms, Some(100));
        assert_eq!(
            run.node_for_session("sess-1").map(|n| n.id.as_str()),
            Some("a")
        );
    }

    #[test]
    fn reset_from_reverts_node_and_downstream_and_bumps_seq() {
        let mut run = run_with(
            vec![node("a", true), node("b", true), node("c", false)],
            vec![Edge::depends("a", "b"), Edge::depends("b", "c")],
        );
        for id in ["a", "b", "c"] {
            set(&mut run, id, NodeStatus::Done);
        }
        run.node_mut("b").unwrap().session_id = Some("s".into());
        let seq = run.run_seq;
        let affected = run.reset_from("b");
        assert_eq!(affected, vec!["b".to_owned(), "c".to_owned()]);
        assert_eq!(run.node("a").unwrap().status, NodeStatus::Done);
        assert_eq!(run.node("b").unwrap().status, NodeStatus::Pending);
        assert_eq!(run.node("b").unwrap().session_id, None);
        assert_eq!(run.node("c").unwrap().status, NodeStatus::Pending);
        assert_eq!(run.run_seq, seq + 1);
    }

    #[test]
    fn layout_assigns_layers_by_longest_path() {
        let mut run = run_with(
            vec![
                node("a", true),
                node("b", true),
                node("c", false),
                node("d", false),
            ],
            vec![
                Edge::depends("a", "b"),
                Edge::depends("b", "c"),
                Edge::depends("a", "d"),
                Edge::depends("c", "d"),
            ],
        );
        let options = LayoutOptions::default();
        layout(&mut run, &options);
        let x = |id: &str| run.node(id).unwrap().x;
        assert_eq!(x("a"), 0.0);
        assert_eq!(x("b"), options.col_width);
        assert_eq!(x("c"), options.col_width * 2.0);
        assert_eq!(
            x("d"),
            options.col_width * 3.0,
            "d waits for the longer path"
        );
    }

    #[test]
    fn layout_centres_columns_and_skips_pinned() {
        let mut run = run_with(
            vec![
                node("a", true),
                node("b", false),
                node("c", false),
                node("d", false),
            ],
            vec![
                Edge::depends("a", "b"),
                Edge::depends("a", "c"),
                Edge::depends("a", "d"),
            ],
        );
        let options = LayoutOptions::default();
        run.node_mut("c").unwrap().pinned = true;
        run.node_mut("c").unwrap().x = 999.0;
        let moved = layout(&mut run, &options);
        assert!(!moved.contains(&"c".to_owned()));
        assert_eq!(run.node("c").unwrap().x, 999.0);
        // Column 0 holds one node against a tallest column of three: centred.
        assert_eq!(run.node("a").unwrap().y, options.row_height);
        assert_eq!(run.node("b").unwrap().y, 0.0);
        assert_eq!(run.node("d").unwrap().y, options.row_height * 2.0);
    }

    #[test]
    fn edge_curve_and_arrow_point_forward() {
        let options = LayoutOptions::default();
        let mut from = node("a", true);
        let mut to = node("b", true);
        to.x = options.col_width;
        from.y = 0.0;
        to.y = 0.0;
        let curve = edge_curve(&from, &to, &options);
        assert_eq!(curve.start.0, options.node_w);
        assert_eq!(curve.end.0, options.col_width);
        assert_eq!(curve.start.1, curve.end.1);
        assert!(curve.c1.0 > curve.start.0 && curve.c2.0 < curve.end.0);
        let head = arrow_head(&curve, 8.0);
        assert_eq!(head[0], curve.end);
        assert!(
            head[1].0 < curve.end.0 && head[2].0 < curve.end.0,
            "base is behind the tip"
        );
        assert!((head[1].1 - head[2].1).abs() > 0.0, "base has width");
    }

    #[test]
    fn template_instantiates_with_edges_and_providers() {
        let template = &builtin_templates()[1];
        let patch = template.instantiate(
            &["codex".into(), "claude".into(), "codex".into()],
            "add search",
        );
        assert_eq!(patch.nodes.len(), 3);
        assert_eq!(patch.nodes[0].provider_id, "codex");
        assert_eq!(patch.nodes[1].provider_id, "claude");
        assert!(patch.nodes[0].prompt.contains("add search"));
        assert_eq!(patch.edges.len(), 2);
        assert_eq!(patch.edges[1].kind, EdgeKind::OnFailure);
        assert_eq!(patch.edges[1].from, "n2");
        assert_eq!(patch.edges[1].to, "n3");
        let mut run = WorkflowRun::new("add search", "p");
        run.apply_patch(patch, AGENTS).expect("templates are valid");
    }

    #[test]
    fn store_round_trips_and_upserts() {
        let mut store = WorkflowStore::default();
        let run = run_with(vec![node("a", true)], vec![]);
        let id = run.id.clone();
        store.upsert(run.clone());
        store.upsert(run.clone());
        assert_eq!(store.runs.len(), 1);
        let encoded = serde_json::to_string(&store).unwrap();
        let decoded: WorkflowStore = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, store);
        assert!(store.remove(&id));
        assert!(!store.remove(&id));
    }

    #[test]
    fn store_reads_older_shape_with_defaults() {
        let raw = r#"{"runs":[{"id":"w1","title":"t","goal":"g","project_id":"p",
            "nodes":[{"id":"n1","role":"r","provider_id":"claude","prompt":"x"}],
            "created_at_ms":1,"updated_at_ms":1}]}"#;
        let store: WorkflowStore = serde_json::from_str(raw).unwrap();
        let run = &store.runs[0];
        assert_eq!(run.status, RunStatus::Draft);
        assert_eq!(run.budget, Budget::default());
        assert_eq!(run.nodes[0].status, NodeStatus::Proposed);
        assert_eq!(run.nodes[0].gate, Gate::Auto);
    }

    #[test]
    fn reconcile_after_restart_marks_live_as_failed_and_pauses() {
        let mut store = WorkflowStore::default();
        let mut run = run_with(
            vec![node("a", true), node("b", false), node("c", false)],
            vec![],
        );
        run.status = RunStatus::Running;
        set(&mut run, "a", NodeStatus::Running);
        set(&mut run, "b", NodeStatus::Ready);
        set(&mut run, "c", NodeStatus::Done);
        let mut untouched = run_with(vec![node("z", true)], vec![]);
        untouched.status = RunStatus::AwaitingApproval;
        set(&mut untouched, "z", NodeStatus::Running);
        store.upsert(run.clone());
        store.upsert(untouched.clone());
        assert_eq!(store.reconcile_after_restart(5), 1);
        let run = store.get(&run.id).unwrap();
        assert_eq!(run.status, RunStatus::Paused);
        assert_eq!(run.node("a").unwrap().status, NodeStatus::Failed);
        assert_eq!(run.node("b").unwrap().status, NodeStatus::Pending);
        assert_eq!(run.node("c").unwrap().status, NodeStatus::Done);
        assert_eq!(run.timeline.last().unwrap().kind, EventKind::Interrupted);
        let untouched = store.get(&untouched.id).unwrap();
        assert_eq!(
            untouched.status,
            RunStatus::AwaitingApproval,
            "only running runs are touched"
        );
    }

    #[test]
    fn trim_keeps_unfinished_and_recent_finished() {
        let mut store = WorkflowStore::default();
        for i in 0..5 {
            let mut run = run_with(vec![node("a", true)], vec![]);
            run.status = RunStatus::Done;
            run.updated_at_ms = i;
            store.upsert(run);
        }
        let mut live = run_with(vec![node("a", true)], vec![]);
        live.status = RunStatus::Running;
        live.updated_at_ms = -100;
        let live_id = live.id.clone();
        store.upsert(live);
        store.trim(2);
        assert_eq!(store.runs.len(), 3);
        assert!(store.get(&live_id).is_some());
        let kept: Vec<i64> = store
            .runs
            .iter()
            .filter(|run| run.status.is_finished())
            .map(|run| run.updated_at_ms)
            .collect();
        assert_eq!(kept, vec![3, 4]);
    }

    #[test]
    fn new_run_titles_from_first_non_empty_line() {
        let run = WorkflowRun::new("\n\n  Add search to settings  \nmore", "p");
        assert_eq!(run.title, "Add search to settings");
        assert_eq!(run.next_node_id(), "n1");
    }

    #[test]
    fn summary_is_capped() {
        let mut run = run_with(vec![node("a", true)], vec![]);
        let long = "x".repeat(SUMMARY_CAP + 50);
        run.settle_node(
            "a",
            NodeOutcome::Done {
                summary: Some(long),
                numstat: None,
            },
            1,
        );
        assert_eq!(
            run.node("a")
                .unwrap()
                .summary
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            SUMMARY_CAP
        );
    }
}
