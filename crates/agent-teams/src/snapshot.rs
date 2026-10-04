// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! What the Team panel draws: one team's record, projected for display, and
//! the compact left-to-right task graph.
//!
//! Assembled from the durable record (the truth) plus the live activity the
//! app already knows, so the panel shows the real state even when a model
//! skipped a step of the protocol.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::types::{MemberStatus, TaskStatus, TeamState, TeamTask};

/// A task's state as the panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VisualTaskState {
    /// An upstream task is not finished.
    Blocked,
    /// Ready, or claimed and not started.
    Open,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl VisualTaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            VisualTaskState::Blocked => "blocked",
            VisualTaskState::Open => "open",
            VisualTaskState::Running => "running",
            VisualTaskState::Completed => "completed",
            VisualTaskState::Failed => "failed",
            VisualTaskState::Cancelled => "cancelled",
        }
    }
}

/// The visual state of one task.
pub fn task_visual_state(task: &TeamTask, tasks: &[TeamTask]) -> VisualTaskState {
    match task.status {
        TaskStatus::Completed => VisualTaskState::Completed,
        TaskStatus::Failed => VisualTaskState::Failed,
        TaskStatus::Cancelled => VisualTaskState::Cancelled,
        TaskStatus::InProgress => VisualTaskState::Running,
        TaskStatus::Pending | TaskStatus::Claimed => {
            let blocked = task.dependencies.iter().any(|id| {
                tasks
                    .iter()
                    .find(|candidate| &candidate.id == id)
                    .is_some_and(|dependency| dependency.status != TaskStatus::Completed)
            });
            if blocked {
                VisualTaskState::Blocked
            } else {
                VisualTaskState::Open
            }
        }
    }
}

/// Longest dependency path per task: its column in the graph.
pub fn task_depths(tasks: &[TeamTask]) -> HashMap<String, usize> {
    fn depth_of(
        id: &str,
        tasks: &[TeamTask],
        depths: &mut HashMap<String, usize>,
        visiting: &mut HashSet<String>,
    ) -> usize {
        if let Some(depth) = depths.get(id) {
            return *depth;
        }
        if visiting.contains(id) {
            return 0;
        }
        let Some(task) = tasks.iter().find(|task| task.id == id) else {
            return 0;
        };
        visiting.insert(id.to_owned());
        let mut dependencies: Vec<&String> = task
            .dependencies
            .iter()
            .filter(|dependency| tasks.iter().any(|task| &task.id == *dependency))
            .collect();
        dependencies.sort();
        let depth = dependencies
            .into_iter()
            .map(|dependency| depth_of(dependency, tasks, depths, visiting) + 1)
            .max()
            .unwrap_or(0);
        visiting.remove(id);
        depths.insert(id.to_owned(), depth);
        depth
    }
    let mut depths = HashMap::new();
    let mut visiting = HashSet::new();
    for task in tasks {
        depth_of(&task.id, tasks, &mut depths, &mut visiting);
    }
    depths
}

/// A member's live activity as the app sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemberLiveActivity {
    Working,
    Idle,
    Unknown,
}

/// One member row.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberRow {
    pub id: String,
    pub name: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: String,
    pub status: MemberStatus,
    pub activity: MemberLiveActivity,
    pub done: usize,
    pub total: usize,
    /// The task the member is working on now, if any.
    pub current_task: Option<String>,
    pub unread: usize,
    pub spawn_error: Option<String>,
}

impl MemberRow {
    /// `provider/model`, or just the model.
    pub fn route_label(&self) -> String {
        match (self.provider.is_empty(), self.model.is_empty()) {
            (false, false) => format!("{}/{}", self.provider, self.model),
            _ => self.model.clone(),
        }
    }

    pub fn is_working(&self) -> bool {
        self.activity == MemberLiveActivity::Working || self.status == MemberStatus::Working
    }
}

/// One task row.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    pub state: VisualTaskState,
    pub assignee: String,
    /// The assignee's `provider/model`.
    pub model: String,
    pub dependencies: Vec<String>,
    pub depth: usize,
    pub kind: Option<String>,
    pub round: Option<u32>,
    pub verdict: Option<String>,
    pub output: Option<String>,
    pub open_findings: usize,
    pub updated_at: u64,
}

/// One message preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePreview {
    pub from: String,
    pub content: String,
}

/// Everything the panel shows for one team.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamSnapshot {
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub captain_session_id: String,
    pub staged: bool,
    pub awaiting_feedback: bool,
    pub halted: bool,
    pub escalated: bool,
    /// Archived (ended or discarded): kept for review only.
    pub archived: bool,
    pub profile: Option<String>,
    pub members: Vec<MemberRow>,
    pub tasks: Vec<TaskRow>,
    pub message_count: usize,
    pub captain_inbox: Vec<MessagePreview>,
}

/// The inputs beyond the record: unread counts by mailbox key and the live
/// activity of member sessions by id.
#[derive(Clone, Debug, Default)]
pub struct LiveFacts {
    pub unread_by_member: HashMap<String, usize>,
    pub activity_by_member_id: HashMap<String, MemberLiveActivity>,
    pub captain_unread: Vec<MessagePreview>,
    pub message_count: usize,
}

/// Assemble one team's snapshot.
pub fn assemble(team: &TeamState, facts: &LiveFacts, archived: bool) -> TeamSnapshot {
    let depths = task_depths(&team.tasks);
    let roster = team
        .members
        .iter()
        .filter(|member| archived || !member.is_removed());
    let members: Vec<MemberRow> = roster
        .map(|member| {
            let owned: Vec<&TeamTask> = team
                .tasks
                .iter()
                .filter(|task| task.assignee.as_deref() == Some(member.name.as_str()))
                .collect();
            let activity = if archived {
                MemberLiveActivity::Idle
            } else if member.is_spawned() {
                facts
                    .activity_by_member_id
                    .get(&member.id)
                    .copied()
                    .unwrap_or(MemberLiveActivity::Idle)
            } else {
                MemberLiveActivity::Unknown
            };
            MemberRow {
                id: member.id.clone(),
                name: member.name.clone(),
                role: member.role.clone().unwrap_or_default(),
                provider: member
                    .provider
                    .clone()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
                model: member.model.clone().unwrap_or_default().trim().to_owned(),
                reasoning_effort: member.reasoning_effort.clone().unwrap_or_default(),
                status: member.status,
                activity,
                done: owned
                    .iter()
                    .filter(|task| task.status == TaskStatus::Completed)
                    .count(),
                total: owned.len(),
                current_task: owned
                    .iter()
                    .find(|task| task.status == TaskStatus::InProgress)
                    .map(|task| task.id.clone()),
                unread: facts
                    .unread_by_member
                    .get(&member.name)
                    .copied()
                    .unwrap_or(0),
                spawn_error: member.spawn_error.clone(),
            }
        })
        .collect();
    let route_of = |name: &str| {
        members
            .iter()
            .find(|member| member.name == name)
            .map(MemberRow::route_label)
            .unwrap_or_default()
    };
    let tasks = team
        .tasks
        .iter()
        .map(|task| {
            let assignee = task.assignee.clone().unwrap_or_default();
            TaskRow {
                id: task.id.clone(),
                subject: task.subject.clone(),
                description: task.description.clone().unwrap_or_default(),
                status: task.status,
                state: task_visual_state(task, &team.tasks),
                model: route_of(&assignee),
                assignee,
                dependencies: task.dependencies.clone(),
                depth: depths.get(&task.id).copied().unwrap_or(0),
                kind: task.kind.map(|kind| kind.as_str().to_owned()),
                round: task.round,
                verdict: task.verdict.map(|verdict| verdict.as_str().to_owned()),
                output: task.output.clone(),
                open_findings: task
                    .findings
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|finding| finding.resolved != Some(true))
                    .count(),
                updated_at: task.updated_at,
            }
        })
        .collect();
    TeamSnapshot {
        team_id: team.id.clone(),
        name: team.name.clone(),
        description: team.description.clone(),
        captain_session_id: team.captain_session_id.clone(),
        staged: team.is_staged(),
        awaiting_feedback: team.review_state()
            == Some(crate::types::PlanReviewState::AwaitingFeedback),
        halted: team.is_halted(),
        escalated: team.is_escalated(),
        archived,
        profile: team.profile.as_ref().map(|profile| profile.name.clone()),
        members,
        tasks,
        message_count: facts.message_count,
        captain_inbox: facts
            .captain_unread
            .iter()
            .rev()
            .take(5)
            .rev()
            .cloned()
            .collect(),
    }
}

impl TeamSnapshot {
    /// The team still has work in flight.
    pub fn is_active(&self) -> bool {
        if self.halted || self.staged || self.archived {
            return false;
        }
        if self.members.iter().any(MemberRow::is_working) {
            return true;
        }
        if self.tasks.iter().any(|task| !task.status.is_terminal()) {
            return true;
        }
        !self.members.is_empty() && self.tasks.is_empty()
    }

    /// Counts per visual state.
    pub fn state_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for task in &self.tasks {
            *counts.entry(task.state.as_str()).or_insert(0) += 1;
        }
        counts
    }

    pub fn completed(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Completed)
            .count()
    }

    /// Members in display order: working first, then finished ones newest
    /// first, then the rest in roster order.
    pub fn ordered_members(&self) -> Vec<&MemberRow> {
        let open = |name: &str| {
            self.tasks
                .iter()
                .any(|task| task.assignee == name && !task.status.is_terminal())
        };
        let finished_at = |name: &str| {
            self.tasks
                .iter()
                .filter(|task| task.assignee == name && task.status.is_terminal())
                .map(|task| task.updated_at)
                .max()
        };
        let mut running = Vec::new();
        let mut stamped = Vec::new();
        let mut unstamped = Vec::new();
        for (index, member) in self.members.iter().enumerate() {
            if member.is_working() || open(&member.name) {
                running.push(member);
            } else if let Some(at) = finished_at(&member.name) {
                stamped.push((member, index, at));
            } else {
                unstamped.push(member);
            }
        }
        stamped.sort_by(|left, right| right.2.cmp(&left.2).then(left.1.cmp(&right.1)));
        running
            .into_iter()
            .chain(stamped.into_iter().map(|(member, _, _)| member))
            .chain(unstamped)
            .collect()
    }
}

/// Node and gap sizes of the compact graph, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DagDims {
    pub node_width: f32,
    pub node_height: f32,
    pub column_gap: f32,
    pub row_gap: f32,
}

impl Default for DagDims {
    fn default() -> Self {
        Self {
            node_width: 132.0,
            node_height: 40.0,
            column_gap: 32.0,
            row_gap: 10.0,
        }
    }
}

/// One placed node: the index of its task in the snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct DagNode {
    pub task: usize,
    pub x: f32,
    pub y: f32,
}

/// One dependency edge as a cubic curve.
#[derive(Clone, Debug, PartialEq)]
pub struct DagEdge {
    pub from: String,
    pub to: String,
    pub start: (f32, f32),
    pub control1: (f32, f32),
    pub control2: (f32, f32),
    pub end: (f32, f32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DagLayout {
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<DagNode>,
    pub edges: Vec<DagEdge>,
}

/// Natural order for ids like `t2` < `t10`.
fn natural_key(id: &str) -> (String, u64, String) {
    let digits_at = id.find(|ch: char| ch.is_ascii_digit()).unwrap_or(id.len());
    let (head, rest) = id.split_at(digits_at);
    let digits_end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    let number = rest[..digits_end].parse().unwrap_or(0);
    (head.to_owned(), number, rest[digits_end..].to_owned())
}

/// Columns are dependency depths; rows are id order within a column; edges
/// are cubic curves leaving a node's right edge for the next's left.
pub fn compact_dag_layout(tasks: &[TaskRow], dims: DagDims) -> DagLayout {
    let mut stages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, task) in tasks.iter().enumerate() {
        stages.entry(task.depth).or_default().push(index);
    }
    for indices in stages.values_mut() {
        indices.sort_by_key(|index| natural_key(&tasks[*index].id));
    }
    let mut positions: HashMap<&str, (f32, f32)> = HashMap::new();
    let mut nodes = Vec::new();
    for (column, indices) in stages.values().enumerate() {
        for (row, index) in indices.iter().enumerate() {
            let x = column as f32 * (dims.node_width + dims.column_gap);
            let y = row as f32 * (dims.node_height + dims.row_gap);
            positions.insert(&tasks[*index].id, (x, y));
            nodes.push(DagNode { task: *index, x, y });
        }
    }
    let mut edges = Vec::new();
    for task in tasks {
        let Some(&(tx, ty)) = positions.get(task.id.as_str()) else {
            continue;
        };
        for dependency in &task.dependencies {
            let Some(&(sx, sy)) = positions.get(dependency.as_str()) else {
                continue;
            };
            let start = (sx + dims.node_width, sy + dims.node_height / 2.0);
            let end = (tx, ty + dims.node_height / 2.0);
            let bend = (dims.column_gap * 0.55).max(8.0);
            edges.push(DagEdge {
                from: dependency.clone(),
                to: task.id.clone(),
                start,
                control1: (start.0 + bend, start.1),
                control2: (end.0 - bend, end.1),
                end,
            });
        }
    }
    let columns = stages.len();
    let rows = stages.values().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return DagLayout::default();
    }
    DagLayout {
        width: columns as f32 * dims.node_width + (columns - 1) as f32 * dims.column_gap,
        height: rows as f32 * dims.node_height + (rows.saturating_sub(1)) as f32 * dims.row_gap,
        nodes,
        edges,
    }
}

/// No task depends on another: a wrapping grid reads better than a graph.
pub fn uses_parallel_task_grid(tasks: &[TaskRow]) -> bool {
    if tasks.is_empty() {
        return false;
    }
    let ids: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
    tasks.iter().all(|task| {
        task.dependencies
            .iter()
            .all(|dependency| !ids.contains(dependency.as_str()))
    })
}

/// The whole upstream and downstream chain around one task, cycle-safe.
pub fn related_task_ids(task_id: &str, tasks: &[TaskRow]) -> HashSet<String> {
    if !tasks.iter().any(|task| task.id == task_id) {
        return HashSet::new();
    }
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in tasks {
        for dependency in &task.dependencies {
            dependents
                .entry(dependency.as_str())
                .or_default()
                .push(task.id.as_str());
        }
    }
    let mut related = HashSet::new();
    let mut stack = vec![task_id.to_owned()];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        related.insert(id.clone());
        if let Some(task) = tasks.iter().find(|task| task.id == id) {
            stack.extend(task.dependencies.iter().cloned());
        }
    }
    let mut stack = vec![task_id.to_owned()];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        related.insert(id.clone());
        if let Some(next) = dependents.get(id.as_str()) {
            stack.extend(next.iter().map(|id| (*id).to_owned()));
        }
    }
    related
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TeamMember, TeamTask};

    fn team() -> TeamState {
        let mut team = TeamState::new("Alpha", "alpha", "cap", 1);
        let mut dev = TeamMember::new("dev", 1);
        dev.id = "m-dev".into();
        dev.provider = Some("openai".into());
        dev.model = Some("gpt-6-sol".into());
        team.members.push(dev);
        team.members.push(TeamMember::new("qa", 1));
        let mut first = TeamTask::new("t1", "survey", 1);
        first.assignee = Some("dev".into());
        first.status = TaskStatus::Completed;
        let mut second = TeamTask::new("t2", "check", 1);
        second.dependencies = vec!["t1".into()];
        second.assignee = Some("qa".into());
        second.status = TaskStatus::InProgress;
        let mut third = TeamTask::new("t10", "ship", 1);
        third.dependencies = vec!["t2".into()];
        let mut fourth = TeamTask::new("t3", "docs", 1);
        fourth.dependencies = vec!["t1".into()];
        team.tasks = vec![first, second, third, fourth];
        team
    }

    #[test]
    fn rows_carry_states_depths_and_routes() {
        let mut facts = LiveFacts::default();
        facts
            .activity_by_member_id
            .insert("m-dev".into(), MemberLiveActivity::Working);
        let snapshot = assemble(&team(), &facts, false);
        let states: Vec<_> = snapshot.tasks.iter().map(|task| task.state).collect();
        assert_eq!(
            states,
            [
                VisualTaskState::Completed,
                VisualTaskState::Running,
                VisualTaskState::Blocked,
                VisualTaskState::Open
            ]
        );
        let depths: Vec<_> = snapshot.tasks.iter().map(|task| task.depth).collect();
        assert_eq!(depths, [0, 1, 2, 1]);
        assert_eq!(snapshot.tasks[0].model, "openai/gpt-6-sol");
        assert_eq!(snapshot.members[0].done, 1);
        assert_eq!(snapshot.members[1].activity, MemberLiveActivity::Unknown);
        assert!(snapshot.is_active());
    }

    #[test]
    fn the_layout_places_columns_by_depth_and_rows_by_natural_id() {
        let snapshot = assemble(&team(), &LiveFacts::default(), false);
        let dims = DagDims {
            node_width: 100.0,
            node_height: 30.0,
            column_gap: 20.0,
            row_gap: 10.0,
        };
        let layout = compact_dag_layout(&snapshot.tasks, dims);
        assert_eq!(layout.width, 3.0 * 100.0 + 2.0 * 20.0);
        assert_eq!(layout.height, 2.0 * 30.0 + 10.0);
        let position = |id: &str| {
            let node = layout
                .nodes
                .iter()
                .find(|node| snapshot.tasks[node.task].id == id)
                .unwrap();
            (node.x, node.y)
        };
        assert_eq!(position("t1"), (0.0, 0.0));
        assert_eq!(position("t2"), (120.0, 0.0));
        assert_eq!(position("t3"), (120.0, 40.0));
        assert_eq!(position("t10"), (240.0, 0.0));
        assert_eq!(layout.edges.len(), 3);
        let edge = layout.edges.iter().find(|edge| edge.to == "t2").unwrap();
        assert_eq!(edge.start, (100.0, 15.0));
        assert_eq!(edge.end, (120.0, 15.0));
    }

    #[test]
    fn related_ids_follow_both_directions() {
        let snapshot = assemble(&team(), &LiveFacts::default(), false);
        let related = related_task_ids("t2", &snapshot.tasks);
        assert!(related.contains("t1") && related.contains("t10") && related.contains("t2"));
        assert!(!related.contains("t3"));
        assert!(!uses_parallel_task_grid(&snapshot.tasks));
    }

    #[test]
    fn finished_members_follow_working_ones_newest_first() {
        let mut team = team();
        team.members.push(TeamMember::new("docs", 1));
        team.tasks[1].status = TaskStatus::Completed;
        team.tasks[1].updated_at = 50;
        team.tasks[0].updated_at = 10;
        team.tasks[2].assignee = Some("docs".into());
        let snapshot = assemble(&team, &LiveFacts::default(), false);
        let order: Vec<&str> = snapshot
            .ordered_members()
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        assert_eq!(order, ["docs", "qa", "dev"]);
    }
}
