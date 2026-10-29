//! The Trajectory surface: what the built-in agent did, request by request.
//!
//! Fork addition, a port of DeepSeek Harness's Trajectory view — MIT
//! License, Copyright (c) 2026 DeepSeek; see NOTICE.md. Three parts, as in
//! dsh: an overview timeline on three lanes (input, model, tools) where a
//! drag picks an interval and a click picks a record; the ledger, a turn at a
//! time, one row per prompt, request, tool call, compaction or prompt
//! change; and an inspector whose tabs depend on what is picked — a request's
//! timing, usage and options, a tool's payload, result and schema, a prompt
//! change's diff.
//!
//! The record is written by the bridge beside the session's transcript
//! (`waku-agent-bridge/src/trajectory.rs`) and read here through the daemon,
//! as the Team panel reads its files: no wire change. Reading and building
//! run off the UI thread; frames read only what the last read left. Only
//! the built-in agent's sessions have one.

use std::cell::Cell as StdCell;
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use agent_trajectory::metrics::{self, Timing};
use agent_trajectory::prompt_diff::{LineTag, line_diff, tool_changes};
use agent_trajectory::record::{CompactionPhase, Purpose, Record, RequestStatus};
use agent_trajectory::snapshot::{CellDetail, CellKind, PromptChange, Snapshot};
use agent_trajectory::timeline::{self, Mode};
use gpui::{ContentMask, size};

use crate::model::RetryCause;
use crate::ui::ActivationExt as _;

use super::providers_page::card_button;
use super::*;

/// The `key_context` of the surface.
pub(super) const TRAJECTORY_CONTEXT: &str = "Trajectory";
/// How often a live session's trajectory is read again.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const LANE_HEIGHT: f32 = 12.0;
const LANE_GAP: f32 = 4.0;
const TIMELINE_PADDING: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Tab {
    #[default]
    Summary,
    Raw,
    Payload,
    Result,
    Schema,
    Timing,
    Options,
    Usage,
    Diff,
    SystemPrompt,
    Tools,
}

impl Tab {
    fn label(self) -> String {
        match self {
            Self::Summary => tr!("trajectory.tab.summary"),
            Self::Raw => tr!("trajectory.tab.raw"),
            Self::Payload => tr!("trajectory.tab.payload"),
            Self::Result => tr!("trajectory.tab.result"),
            Self::Schema => tr!("trajectory.tab.schema"),
            Self::Timing => tr!("trajectory.tab.timing"),
            Self::Options => tr!("trajectory.tab.options"),
            Self::Usage => tr!("trajectory.tab.usage"),
            Self::Diff => tr!("trajectory.tab.diff"),
            Self::SystemPrompt => tr!("trajectory.tab.system_prompt"),
            Self::Tools => tr!("trajectory.tab.tools"),
        }
    }
}

/// The tabs a record of `kind` has, as dsh's inspector lays them out.
fn tabs_for(kind: CellKind, has_previous: bool) -> Vec<Tab> {
    match kind {
        CellKind::System if has_previous => vec![Tab::Diff, Tab::SystemPrompt, Tab::Tools],
        CellKind::System => vec![Tab::SystemPrompt, Tab::Tools],
        CellKind::User => vec![Tab::Summary, Tab::Raw],
        CellKind::Message => vec![
            Tab::Summary,
            Tab::Options,
            Tab::Usage,
            Tab::Timing,
            Tab::Raw,
        ],
        CellKind::Tool | CellKind::Subtool => {
            vec![
                Tab::Summary,
                Tab::Payload,
                Tab::Result,
                Tab::Schema,
                Tab::Timing,
            ]
        }
        CellKind::Compacted => vec![Tab::Summary],
    }
}

fn kind_label(kind: CellKind) -> String {
    match kind {
        CellKind::System => tr!("trajectory.kind.system"),
        CellKind::User => tr!("trajectory.kind.user"),
        CellKind::Message => tr!("trajectory.kind.message"),
        CellKind::Tool => tr!("trajectory.kind.tool"),
        CellKind::Subtool => tr!("trajectory.kind.subtool"),
        CellKind::Compacted => tr!("trajectory.kind.compacted"),
    }
}

fn kind_icon(kind: CellKind) -> &'static str {
    match kind {
        CellKind::System => "icons/settings.svg",
        CellKind::User => "icons/compose.svg",
        CellKind::Message => "icons/sparkle.svg",
        CellKind::Tool => "icons/terminal.svg",
        CellKind::Subtool => "icons/bot.svg",
        CellKind::Compacted => "icons/chevrons-down-up.svg",
    }
}

fn lane_color(lane: u8, theme: &Theme) -> Hsla {
    match lane {
        0 => theme.text_tertiary,
        1 => theme.accent,
        _ => theme.success,
    }
}

fn local_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_default()
}

fn timing_text(timing: Timing<i64>) -> String {
    match timing {
        Timing::Value(ms) => metrics::format_duration(ms),
        Timing::FirstTokenUnavailable => tr!("trajectory.timing.first_token_unavailable"),
        Timing::UsageUnavailable => tr!("trajectory.timing.usage_unavailable"),
        Timing::TooShort => tr!("trajectory.timing.too_short"),
    }
}

fn pretty_json(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_owned())
}

/// One row of the ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Row {
    Turn(usize),
    Cell(usize),
}

/// The ledger's rows: a header per turn, then its cells unless folded, with
/// cells outside any turn (a `/compact`) where they happened. With an
/// interval picked on the timeline, only the cells in it.
fn ledger_rows(
    snapshot: &Snapshot,
    folded: &HashSet<usize>,
    focus: Option<&HashSet<usize>>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut turn_at: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, turn) in snapshot.turns.iter().enumerate() {
        turn_at.insert(turn.first_cell, position);
    }
    let mut current: Option<(usize, usize)> = None;
    for cell in &snapshot.cells {
        if let Some(position) = turn_at.get(&cell.index) {
            let turn = &snapshot.turns[*position];
            rows.push(Row::Turn(*position));
            current = Some((*position, turn.first_cell + turn.cell_count));
        }
        let in_turn = current
            .filter(|(_, end)| cell.index < *end)
            .map(|(position, _)| position);
        if in_turn.is_some_and(|position| folded.contains(&position)) {
            continue;
        }
        if focus.is_some_and(|focus| !focus.contains(&cell.index)) {
            continue;
        }
        rows.push(Row::Cell(cell.index));
    }
    rows
}

struct View {
    consumed: BTreeMap<u32, usize>,
    records: Arc<Vec<Record>>,
    snapshot: Arc<Snapshot>,
    timeline: Option<Arc<timeline::Model>>,
    generation: u64,
    loading: bool,
    loaded: bool,
    poll_scheduled: bool,
    error: Option<String>,
    last_read: Option<Instant>,
    selected: Option<usize>,
    tab: Tab,
    mode: Mode,
    focus: Option<(f64, f64)>,
    folded: HashSet<usize>,
    rows: Rc<Vec<Row>>,
    list: ListState,
    scrollbar: Rc<ScrollbarState>,
    /// A tool call to show once the record has it (Inspect).
    pending_call: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            consumed: BTreeMap::new(),
            records: Arc::new(Vec::new()),
            snapshot: Arc::new(Snapshot::default()),
            timeline: None,
            generation: 0,
            loading: false,
            loaded: false,
            poll_scheduled: false,
            error: None,
            last_read: None,
            selected: None,
            tab: Tab::default(),
            mode: Mode::default(),
            focus: None,
            folded: HashSet::new(),
            rows: Rc::new(Vec::new()),
            list: ListState::new(0, ListAlignment::Top, px(256.0)),
            scrollbar: ScrollbarState::new(),
            pending_call: None,
        }
    }
}

impl View {
    fn rebuild_rows(&mut self) {
        let focus: Option<HashSet<usize>> = match (self.focus, self.timeline.as_ref()) {
            (Some((start, end)), Some(model)) => Some(
                timeline::focus_indexes(model, start, end)
                    .into_iter()
                    .collect(),
            ),
            _ => None,
        };
        let rows = ledger_rows(&self.snapshot, &self.folded, focus.as_ref());
        let follow = self.list.logical_scroll_top().item_ix + 40 >= self.rows.len();
        if rows.len() != self.rows.len() {
            self.list.reset(rows.len());
            if follow && self.selected.is_none() {
                self.list
                    .scroll_to_reveal_item(rows.len().saturating_sub(1));
            }
        } else {
            self.list.remeasure();
        }
        self.rows = Rc::new(rows);
    }

    fn reveal(&mut self, cell: usize) {
        if let Some(position) = self.rows.iter().position(|row| *row == Row::Cell(cell)) {
            self.list.scroll_to_reveal_item(position);
        }
    }
}

#[derive(Default)]
pub(super) struct TrajectoryPanelState {
    views: HashMap<Uuid, View>,
    /// Where the timeline was last painted, for mapping the pointer.
    timeline_bounds: Rc<StdCell<Option<Bounds<Pixels>>>>,
    drag_from: Option<f64>,
    inspector_scroll: ScrollHandle,
}

/// What one read brought back.
struct Read {
    consumed: BTreeMap<u32, usize>,
    records: Arc<Vec<Record>>,
    snapshot: Arc<Snapshot>,
}

/// Read what the segments added since `consumed`, through the daemon.
/// Blocking: runs on the background executor.
fn read_trajectory(
    workspace: &waku_client::WorkspaceClient,
    root: PathBuf,
    dir_name: String,
    mut consumed: BTreeMap<u32, usize>,
    records: Arc<Vec<Record>>,
) -> Result<Read, String> {
    let dir = root.join(&dir_name);
    let segments: Vec<u32> = match workspace
        .request(waku_client::WorkspaceOperation::BrowseDirectory { path: Some(dir) })
    {
        Ok(waku_client::WorkspaceResult::Directory { entries, .. }) => {
            let mut numbers: Vec<u32> = entries
                .into_iter()
                .filter_map(|entry| entry.name.strip_suffix(".jsonl")?.parse().ok())
                .collect();
            numbers.sort_unstable();
            numbers
        }
        // No directory yet: nothing recorded so far.
        _ => Vec::new(),
    };
    let mut records = (*records).clone();
    let last = segments.last().copied();
    for segment in segments {
        let from = consumed.get(&segment).copied();
        // A finished segment read to its end before is not read again.
        if from.is_some() && Some(segment) != last {
            continue;
        }
        let relative =
            PathBuf::from(&dir_name).join(agent_trajectory::writer::segment_name(segment));
        let text = match workspace.request(waku_client::WorkspaceOperation::ReadTextFile {
            root: root.clone(),
            relative_path: relative,
        }) {
            Ok(waku_client::WorkspaceResult::TextFile { content }) => content,
            Ok(_) => return Err("unexpected answer".into()),
            Err(error) => return Err(error.to_string()),
        };
        let (added, next) = agent_trajectory::reader::parse_lines(&text, from.unwrap_or(0));
        records.extend(added);
        consumed.insert(segment, next);
    }
    let snapshot = Arc::new(agent_trajectory::build(&records));
    Ok(Read {
        consumed,
        records: Arc::new(records),
        snapshot,
    })
}

impl Waku {
    pub(super) fn trajectory_capable(&self, session_id: Uuid) -> bool {
        self.state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .is_some_and(|session| session.provider == ProviderKind::Native)
    }

    fn trajectory_showing(&self) -> bool {
        self.right_panel_visible
            && matches!(
                self.active_right_panel_surface(),
                Some(RightPanelSurface::Trajectory)
            )
    }

    /// Read the session's trajectory again, off the UI thread.
    fn refresh_trajectory(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(session) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        let Some(transcript) = session.provider_native_id().map(str::to_owned) else {
            return;
        };
        let view = self.trajectory_panel.views.entry(session_id).or_default();
        if view.loading {
            return;
        }
        view.loading = true;
        view.generation += 1;
        view.last_read = Some(Instant::now());
        let generation = view.generation;
        let consumed = view.consumed.clone();
        let records = view.records.clone();
        let root =
            waku_client::persistence::StateStore::default_path().with_file_name("agent-sessions");
        let dir_name = format!("{transcript}.trajectory");
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(
                    async move { read_trajectory(&workspace, root, dir_name, consumed, records) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                let busy = this
                    .state
                    .sessions
                    .iter()
                    .find(|session| session.id == session_id)
                    .is_some_and(|session| session.status.is_busy());
                let Some(view) = this.trajectory_panel.views.get_mut(&session_id) else {
                    return;
                };
                view.loading = false;
                if view.generation != generation {
                    return;
                }
                match read {
                    Ok(read) => {
                        let changed = read.records.len() != view.records.len();
                        view.consumed = read.consumed;
                        view.records = read.records;
                        view.error = None;
                        view.loaded = true;
                        if changed {
                            view.snapshot = read.snapshot;
                            view.timeline =
                                timeline::derive(&view.snapshot, view.mode).map(Arc::new);
                            view.rebuild_rows();
                        }
                        if let Some(call) = view.pending_call.clone()
                            && let Some(cell) = view.snapshot.cell_for_call(&call)
                        {
                            view.pending_call = None;
                            view.selected = Some(cell);
                            view.tab = Tab::Payload;
                            view.focus = None;
                            view.rebuild_rows();
                            view.reveal(cell);
                        }
                    }
                    Err(error) => view.error = Some(error),
                }
                // A live turn is read again in a moment; so is a turn whose
                // end is not on record yet.
                let open_turn = view
                    .snapshot
                    .turns
                    .last()
                    .is_some_and(|turn| turn.ended_at.is_none());
                if (busy || open_turn || view.pending_call.is_some()) && !view.poll_scheduled {
                    view.poll_scheduled = true;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(POLL_INTERVAL).await;
                        let _ = this.update(cx, |this, cx| {
                            if let Some(view) = this.trajectory_panel.views.get_mut(&session_id) {
                                view.poll_scheduled = false;
                            }
                            if this.trajectory_showing()
                                && this.state.selected_session == Some(session_id)
                            {
                                this.refresh_trajectory(session_id, cx);
                            }
                        });
                    })
                    .detach();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_trajectory(&mut self, cx: &mut Context<Self>) {
        self.open_right_panel_surface(RightPanelSurface::Trajectory, cx);
        if let Some(session_id) = self.state.selected_session {
            self.refresh_trajectory(session_id, cx);
        }
        cx.notify();
    }

    /// Inspect: open the trajectory on the tool call `call_id`.
    pub(super) fn inspect_tool_call(&mut self, call_id: String, cx: &mut Context<Self>) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        let view = self.trajectory_panel.views.entry(session_id).or_default();
        match view.snapshot.cell_for_call(&call_id) {
            Some(cell) => {
                view.selected = Some(cell);
                view.tab = Tab::Payload;
                view.focus = None;
                view.rebuild_rows();
                view.reveal(cell);
            }
            None => view.pending_call = Some(call_id),
        }
        self.open_trajectory(cx);
    }

    fn select_trajectory_cell(&mut self, cell: usize, cx: &mut Context<Self>) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        if let Some(view) = self.trajectory_panel.views.get_mut(&session_id) {
            if view.selected != Some(cell) {
                let kind = view.snapshot.cells.get(cell).map(|cell| cell.kind);
                let has_previous = view.snapshot.cells.get(cell).is_some_and(|cell| {
                    matches!(
                        &cell.detail,
                        CellDetail::System {
                            previous_system_fp: Some(_),
                            ..
                        }
                    )
                });
                if let Some(kind) = kind
                    && !tabs_for(kind, has_previous).contains(&view.tab)
                {
                    view.tab = tabs_for(kind, has_previous)[0];
                }
            }
            view.selected = Some(cell);
            view.reveal(cell);
        }
        cx.notify();
    }

    fn with_trajectory_view(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut View)) {
        if let Some(view) = self
            .state
            .selected_session
            .and_then(|session_id| self.trajectory_panel.views.get_mut(&session_id))
        {
            change(view);
            cx.notify();
        }
    }

    /// Escape: drop the picked interval, then the picked record. Never
    /// further — unclaimed, Escape stops the task's turn.
    pub(super) fn trajectory_escape_action(
        &mut self,
        _: &super::shortcuts::TrajectoryEscape,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_trajectory_view(cx, |view| {
            if view.focus.take().is_some() {
                view.rebuild_rows();
            } else {
                view.selected = None;
            }
        });
    }

    fn trajectory_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let delta: isize = match event.keystroke.key.as_str() {
            "down" | "right" => 1,
            "up" | "left" => -1,
            "home" => isize::MIN / 2,
            "end" => isize::MAX / 2,
            _ => return,
        };
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        let Some(view) = self.trajectory_panel.views.get(&session_id) else {
            return;
        };
        let cells: Vec<usize> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Cell(cell) => Some(*cell),
                Row::Turn(_) => None,
            })
            .collect();
        if cells.is_empty() {
            return;
        }
        let current = view
            .selected
            .and_then(|selected| cells.iter().position(|cell| *cell == selected));
        let target = match current {
            None => {
                if delta < 0 {
                    cells.len() - 1
                } else {
                    0
                }
            }
            Some(position) => (position as isize)
                .saturating_add(delta)
                .clamp(0, cells.len() as isize - 1) as usize,
        };
        self.select_trajectory_cell(cells[target], cx);
        cx.stop_propagation();
    }

    /// The pointer's place on the timeline's axis.
    fn timeline_value(&self, x: Pixels, model: &timeline::Model) -> Option<f64> {
        let bounds = self.trajectory_panel.timeline_bounds.get()?;
        let left = f32::from(bounds.origin.x) + 60.0;
        let width = (f32::from(bounds.size.width) - 60.0 - TIMELINE_PADDING).max(1.0);
        let ratio = ((f32::from(x) - left) / width).clamp(0.0, 1.0) as f64;
        Some(model.start + ratio * (model.end - model.start))
    }

    fn timeline_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(model) = self.current_timeline() else {
            return;
        };
        if event.button == MouseButton::Right {
            self.with_trajectory_view(cx, |view| {
                view.focus = None;
                view.rebuild_rows();
            });
            return;
        }
        self.trajectory_panel.drag_from = self.timeline_value(event.position.x, &model);
    }

    fn timeline_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (Some(from), Some(model)) = (self.trajectory_panel.drag_from, self.current_timeline())
        else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(to) = self.timeline_value(event.position.x, &model) else {
            return;
        };
        let span = (model.end - model.start).max(1e-9);
        if ((to - from) / span).abs() > 0.01 {
            self.with_trajectory_view(cx, |view| {
                view.focus = Some((from.min(to), from.max(to)));
                view.rebuild_rows();
            });
        }
    }

    fn timeline_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(from) = self.trajectory_panel.drag_from.take() else {
            return;
        };
        let Some(model) = self.current_timeline() else {
            return;
        };
        let Some(to) = self.timeline_value(event.position.x, &model) else {
            return;
        };
        let span = (model.end - model.start).max(1e-9);
        if ((to - from) / span).abs() > 0.01 {
            return;
        }
        // A click: the record under the pointer, on the lane it is over.
        let lane = self.trajectory_panel.timeline_bounds.get().map(|bounds| {
            let y = f32::from(event.position.y) - f32::from(bounds.origin.y) - TIMELINE_PADDING;
            (y / (LANE_HEIGHT + LANE_GAP)).floor().clamp(0.0, 2.0) as u8
        });
        let hit = model
            .spans
            .iter()
            .filter(|candidate| Some(candidate.lane) == lane)
            .min_by(|a, b| {
                let distance = |span: &&timeline::Span| {
                    if to < span.start {
                        span.start - to
                    } else if to > span.end {
                        to - span.end
                    } else {
                        0.0
                    }
                };
                distance(a).total_cmp(&distance(b))
            })
            .map(|span| span.index);
        if let Some(cell) = hit {
            self.select_trajectory_cell(cell, cx);
        }
    }

    fn current_timeline(&self) -> Option<Arc<timeline::Model>> {
        self.state
            .selected_session
            .and_then(|session_id| self.trajectory_panel.views.get(&session_id))
            .and_then(|view| view.timeline.clone())
    }

    /// The Inspect pill on a built-in agent's tool row: opens its record in
    /// the trajectory. Shown on hover, and while it has keyboard focus.
    pub(super) fn render_trajectory_inspect_pill(
        &self,
        source_id: Option<&str>,
        hover_group: &SharedString,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let call_id = source_id?.to_owned();
        if !self
            .state
            .selected_session
            .is_some_and(|session_id| self.trajectory_capable(session_id))
        {
            return None;
        }
        let theme = *theme;
        Some(
            div()
                .id(SharedString::from(format!("trajectory-inspect-{call_id}")))
                .tab_index(0)
                .flex_none()
                .h(px(18.0))
                .px(px(6.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .text_size(sp(11.0))
                .text_color(theme.text_tertiary)
                .opacity(0.0)
                .group_hover(hover_group.clone(), |style| style.opacity(1.0))
                .focus_visible(|style| style.opacity(1.0).border_1().border_color(theme.accent))
                .hover(|style| style.bg(theme.overlay))
                .tooltip(Tooltip::text(tr!("trajectory.inspect_tooltip")))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_activation(cx, move |this, _, cx| {
                    cx.stop_propagation();
                    this.inspect_tool_call(call_id.clone(), cx);
                })
                .child(tr!("trajectory.inspect"))
                .into_any_element(),
        )
    }

    pub(super) fn render_trajectory_surface(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(session_id) = self.state.selected_session else {
            return empty_message(theme, tr!("trajectory.no_session"));
        };
        if !self.trajectory_capable(session_id) {
            return empty_message(theme, tr!("trajectory.only_native"));
        }
        let first_look = self
            .trajectory_panel
            .views
            .get(&session_id)
            .is_none_or(|view| !view.loaded && !view.loading && view.last_read.is_none());
        if first_look {
            self.refresh_trajectory(session_id, cx);
        }
        let Some(view) = self.trajectory_panel.views.get(&session_id) else {
            return empty_message(theme, tr!("trajectory.loading"));
        };
        if view.snapshot.cells.is_empty() {
            return empty_message(
                theme,
                if view.loaded {
                    tr!("trajectory.empty")
                } else {
                    tr!("trajectory.loading")
                },
            );
        }
        let snapshot = view.snapshot.clone();
        let header =
            self.render_trajectory_header(&snapshot, view.mode, view.focus.is_some(), theme, cx);
        let overview = self.render_trajectory_timeline(view, theme, cx);
        let ledger = self.render_trajectory_ledger(view, cx);
        let inspector = view
            .selected
            .and_then(|cell| snapshot.cells.get(cell))
            .map(|cell| self.render_trajectory_inspector(&snapshot, cell, view.tab, theme, cx));
        let focus = self.transcript_control_focus("trajectory-ledger", cx);

        div()
            .id("trajectory-surface")
            .key_context(TRAJECTORY_CONTEXT)
            .track_focus(&focus)
            .tab_index(0)
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, _, cx| this.trajectory_key(event, cx)),
            )
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .child(header)
            .children(overview)
            .child(ledger)
            .children(inspector)
            .into_any_element()
    }

    fn render_trajectory_header(
        &self,
        snapshot: &Snapshot,
        mode: Mode,
        focused: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let usage = snapshot.session_usage;
        let summary = tr!(
            "trajectory.summary",
            turns = snapshot.turns.len(),
            requests = snapshot.requests,
            input = metrics::format_tokens(usage.input + usage.cache_read),
            output = metrics::format_tokens(usage.output)
        );
        let mode_button =
            |mode_value: Mode, id: &'static str, label: String, cx: &mut Context<Self>| {
                let selected = mode == mode_value;
                div()
                    .id(id)
                    .tab_index(0)
                    .h(px(22.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .text_size(sp(11.5))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_tertiary
                    })
                    .when(selected, |button| {
                        button.bg(theme.overlay).font_weight(FontWeight::MEDIUM)
                    })
                    .hover(|button| button.bg(theme.overlay))
                    .focus_visible(|button| button.border_1().border_color(theme.accent))
                    .on_activation(cx, move |this, _, cx| {
                        this.with_trajectory_view(cx, |view| {
                            view.mode = mode_value;
                            view.focus = None;
                            view.timeline =
                                timeline::derive(&view.snapshot, mode_value).map(Arc::new);
                            view.rebuild_rows();
                        });
                    })
                    .child(label)
            };
        div()
            .flex_none()
            .px(px(12.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon("icons/chart-column.svg", 13.0, theme.text_secondary))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(12.0))
                            .text_color(theme.text_secondary)
                            .child(summary),
                    )
                    .when(snapshot.dropped > 0, |row| {
                        row.child(
                            div()
                                .text_size(sp(11.0))
                                .text_color(theme.text_tertiary)
                                .child(tr!("trajectory.dropped")),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(mode_button(
                        Mode::Sequence,
                        "trajectory-mode-sequence",
                        tr!("trajectory.mode.sequence"),
                        cx,
                    ))
                    .child(mode_button(
                        Mode::Duration,
                        "trajectory-mode-duration",
                        tr!("trajectory.mode.duration"),
                        cx,
                    ))
                    .child(mode_button(
                        Mode::Actual,
                        "trajectory-mode-actual",
                        tr!("trajectory.mode.actual"),
                        cx,
                    ))
                    .child(div().flex_1())
                    .when(focused, |row| {
                        row.child(card_button(
                            theme,
                            SharedString::from("trajectory-clear-focus"),
                            tr!("trajectory.clear_focus"),
                            false,
                            false,
                            cx,
                            |this, _, cx| {
                                this.with_trajectory_view(cx, |view| {
                                    view.focus = None;
                                    view.rebuild_rows();
                                });
                            },
                        ))
                    }),
            )
    }

    fn render_trajectory_timeline(
        &self,
        view: &View,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let model = view.timeline.clone()?;
        let selected = view.selected;
        let focus = view.focus;
        let bounds_cell = self.trajectory_panel.timeline_bounds.clone();
        let height = TIMELINE_PADDING * 2.0 + LANE_HEIGHT * 3.0 + LANE_GAP * 2.0;
        let lanes = [
            tr!("trajectory.lane.input"),
            tr!("trajectory.lane.model"),
            tr!("trajectory.lane.tools"),
        ];
        let labels = div()
            .absolute()
            .left(px(8.0))
            .top(px(TIMELINE_PADDING - 2.0))
            .flex()
            .flex_col()
            .gap(px(LANE_GAP))
            .children(lanes.into_iter().map(|lane| {
                div()
                    .h(px(LANE_HEIGHT))
                    .text_size(sp(10.0))
                    .line_height(px(LANE_HEIGHT))
                    .text_color(theme.text_tertiary)
                    .child(lane)
            }));
        let painter = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                bounds_cell.set(Some(bounds));
                paint_timeline(window, bounds, &model, selected, focus, &theme);
            },
        )
        .size_full();
        Some(
            div()
                .id("trajectory-timeline")
                .relative()
                .flex_none()
                .h(px(height))
                .mx(px(4.0))
                .border_b_1()
                .border_color(theme.border)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.timeline_mouse_down(event, cx)
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.timeline_mouse_down(event, cx)
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    this.timeline_mouse_move(event, cx)
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseUpEvent, _, cx| {
                        this.timeline_mouse_up(event, cx)
                    }),
                )
                .tooltip(Tooltip::text(tr!("trajectory.timeline_hint")))
                .child(painter)
                .child(labels)
                .into_any_element(),
        )
    }

    fn render_trajectory_ledger(&self, view: &View, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                list(view.list.clone(), move |index, _, cx| {
                    entity
                        .upgrade()
                        .map(|entity| {
                            entity.update(cx, |this, cx| this.render_trajectory_row(index, cx))
                        })
                        .unwrap_or_else(|| div().into_any_element())
                })
                .size_full(),
            )
            .child(scrollbar::vertical(&view.list, &view.scrollbar))
            .into_any_element()
    }

    fn render_trajectory_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(view) = self
            .state
            .selected_session
            .and_then(|session_id| self.trajectory_panel.views.get(&session_id))
        else {
            return div().into_any_element();
        };
        let Some(row) = view.rows.get(index).copied() else {
            return div().into_any_element();
        };
        let snapshot = &view.snapshot;
        match row {
            Row::Turn(position) => {
                let turn = &snapshot.turns[position];
                let folded = view.folded.contains(&position);
                let (status_icon, status_color, status_text) = match turn.success {
                    Some(true) => (
                        "icons/check.svg",
                        theme.success,
                        tr!("trajectory.status.completed"),
                    ),
                    Some(false) => (
                        "icons/circle-x.svg",
                        theme.danger,
                        tr!("trajectory.status.failed"),
                    ),
                    None => (
                        "icons/loader-circle.svg",
                        theme.text_tertiary,
                        tr!("trajectory.status.running"),
                    ),
                };
                let duration = turn
                    .ended_at
                    .map(|end| metrics::format_duration(end - turn.started_at))
                    .unwrap_or_default();
                let mut detail = tr!(
                    "trajectory.turn_detail",
                    steps = turn.steps,
                    tools = turn.tool_calls,
                    tokens = metrics::format_tokens(turn.usage.output)
                );
                // Fork: a failed turn whose goal resumes on its own.
                if let Some(resume) = turn.goal_resume {
                    detail.push_str(" · ");
                    detail.push_str(&tr!(
                        "trajectory.goal_resume",
                        delay = metrics::format_duration(resume.delay_ms as i64),
                        attempt = resume.attempt,
                        max = resume
                            .max_attempts
                            .map(|max| max.to_string())
                            .unwrap_or_else(|| "∞".into())
                    ));
                }
                div()
                    .id(SharedString::from(format!("trajectory-turn-{position}")))
                    .w_full()
                    .h(px(30.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .bg(theme.raised)
                    .border_b_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .tab_index(0)
                    .focus_visible(|row| row.border_1().border_color(theme.accent))
                    .on_activation(cx, move |this, _, cx| {
                        this.with_trajectory_view(cx, |view| {
                            if !view.folded.remove(&position) {
                                view.folded.insert(position);
                            }
                            view.rebuild_rows();
                        });
                    })
                    .child(icon(
                        if folded {
                            "icons/chevron-right.svg"
                        } else {
                            "icons/chevron-down.svg"
                        },
                        11.0,
                        theme.text_tertiary,
                    ))
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("trajectory.turn", turn = turn.turn)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "trajectory-turn-status-{position}"
                            )))
                            .tooltip(Tooltip::text(status_text))
                            .child(icon(status_icon, 11.0, status_color)),
                    )
                    .when(turn.rewound, |row| {
                        row.child(
                            div()
                                .px(px(5.0))
                                .rounded(px(4.0))
                                .bg(theme.overlay)
                                .text_size(sp(10.5))
                                .text_color(theme.text_tertiary)
                                .child(tr!("trajectory.rewound")),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(detail),
                    )
                    .child(
                        div()
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(duration),
                    )
                    .into_any_element()
            }
            Row::Cell(cell_index) => {
                let Some(cell) = snapshot.cells.get(cell_index) else {
                    return div().into_any_element();
                };
                let selected = view.selected == Some(cell_index);
                let (input, output) = match &cell.detail {
                    CellDetail::Request(request) => request
                        .usage
                        .map(|usage| {
                            (
                                metrics::format_tokens(
                                    usage.input + usage.cache_read + usage.cache_write,
                                ),
                                metrics::format_tokens(usage.output),
                            )
                        })
                        .unwrap_or_default(),
                    _ => (String::new(), String::new()),
                };
                let time = cell
                    .duration_ms
                    .map(metrics::format_duration)
                    .unwrap_or_default();
                let mut text = cell.text.clone();
                if let CellDetail::System { change, .. } = &cell.detail {
                    text = match change {
                        PromptChange::Initial => tr!("trajectory.prompt.initial"),
                        PromptChange::System => tr!("trajectory.prompt.system"),
                        PromptChange::Tools => tr!("trajectory.prompt.tools"),
                        PromptChange::SystemAndTools => tr!("trajectory.prompt.both"),
                    };
                }
                if let CellDetail::Request(request) = &cell.detail {
                    let label = tr!("trajectory.request", number = request.number);
                    text = if text.is_empty() {
                        label
                    } else {
                        format!("{label} · {text}")
                    };
                }
                if let CellDetail::Compaction {
                    phase,
                    tokens_before,
                    tokens_after,
                    ..
                } = &cell.detail
                {
                    text = match phase {
                        CompactionPhase::Started => tr!("trajectory.compacting"),
                        CompactionPhase::Failed => tr!("trajectory.compaction_failed"),
                        CompactionPhase::Finished => tr!(
                            "trajectory.compacted",
                            before = metrics::format_tokens(*tokens_before),
                            after = metrics::format_tokens(tokens_after.unwrap_or(0))
                        ),
                    };
                }
                let lane = timeline::lane_for(cell.kind);
                div()
                    .id(SharedString::from(format!("trajectory-cell-{cell_index}")))
                    .w_full()
                    .min_h(px(26.0))
                    .pl(px(if cell.kind == CellKind::Subtool {
                        26.0
                    } else {
                        14.0
                    }))
                    .pr(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .border_l_2()
                    .border_color(if selected {
                        theme.accent
                    } else {
                        gpui::transparent_black()
                    })
                    .when(selected, |row| row.bg(theme.overlay))
                    .hover(|row| row.bg(theme.overlay))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_trajectory_cell(cell_index, cx)
                    }))
                    .child(icon(kind_icon(cell.kind), 11.0, lane_color(lane, &theme)))
                    .child(
                        div()
                            .w(px(56.0))
                            .flex_none()
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(kind_label(cell.kind)),
                    )
                    .when(cell.is_error, |row| {
                        row.child(icon("icons/alert.svg", 11.0, theme.danger))
                    })
                    .when(cell.running, |row| {
                        row.child(icon("icons/loader-circle.svg", 11.0, theme.text_tertiary))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(12.0))
                            .text_color(theme.text)
                            .child(text),
                    )
                    .child(
                        div()
                            .w(px(46.0))
                            .flex_none()
                            .text_right()
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(input),
                    )
                    .child(
                        div()
                            .w(px(40.0))
                            .flex_none()
                            .text_right()
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(output),
                    )
                    .child(
                        div()
                            .w(px(60.0))
                            .flex_none()
                            .text_right()
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(time),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_trajectory_inspector(
        &self,
        snapshot: &Snapshot,
        cell: &agent_trajectory::Cell,
        tab: Tab,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_previous = matches!(
            &cell.detail,
            CellDetail::System {
                previous_system_fp: Some(_),
                ..
            }
        );
        let tabs = tabs_for(cell.kind, has_previous);
        let tab = if tabs.contains(&tab) { tab } else { tabs[0] };
        let tab_bar = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .children(tabs.iter().map(|candidate| {
                let candidate = *candidate;
                let selected = candidate == tab;
                div()
                    .id(SharedString::from(format!("trajectory-tab-{candidate:?}")))
                    .tab_index(0)
                    .h(px(22.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .text_size(sp(11.5))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_tertiary
                    })
                    .when(selected, |button| {
                        button.bg(theme.overlay).font_weight(FontWeight::MEDIUM)
                    })
                    .hover(|button| button.bg(theme.overlay))
                    .focus_visible(|button| button.border_1().border_color(theme.accent))
                    .on_activation(cx, move |this, _, cx| {
                        this.with_trajectory_view(cx, |view| view.tab = candidate);
                    })
                    .child(candidate.label())
            }));
        let body = inspector_body(snapshot, cell, tab, &theme);
        div()
            .flex_none()
            .h(relative_height())
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.raised)
            .child(
                div()
                    .flex_none()
                    .px(px(10.0))
                    .py(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("trajectory.details")),
                    )
                    .child(tab_bar),
            )
            .child(
                div()
                    .id("trajectory-inspector-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.trajectory_panel.inspector_scroll)
                    .px(px(12.0))
                    .pb(px(12.0))
                    .child(body),
            )
            .into_any_element()
    }
}

fn relative_height() -> gpui::DefiniteLength {
    gpui::relative(0.45)
}

fn empty_message(theme: Theme, message: String) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .px(px(24.0))
        .bg(theme.surface)
        .child(icon("icons/chart-column.svg", 18.0, theme.text_tertiary))
        .child(
            div()
                .max_w(px(320.0))
                .text_center()
                .text_size(sp(12.5))
                .line_height(sp(18.0))
                .text_color(theme.text_tertiary)
                .child(message),
        )
        .into_any_element()
}

/// A label and its value, one row.
/// Fork: one retry of a request — its count, why, how long it waited, what
/// failed, whether a half answer was thrown away, and the failed attempt's
/// request id.
fn retry_line(retry: &agent_trajectory::record::RetryInfo) -> String {
    let mut parts = vec![format!(
        "{}/{}",
        retry.attempt,
        retry
            .max_attempts
            .map(|max| max.to_string())
            .unwrap_or_else(|| "∞".into())
    )];
    if !retry.reason.is_empty() {
        parts.push(RetryCause::from_reason(&retry.reason).label());
    }
    parts.push(metrics::format_duration(retry.delay_ms as i64));
    if !retry.error.is_empty() {
        parts.push(retry.error.clone());
    }
    if retry.discarded_output {
        parts.push(tr!("trajectory.retry_discarded"));
    }
    if let Some(id) = &retry.request_id {
        parts.push(id.clone());
    }
    parts.join(" · ")
}

fn fact(theme: &Theme, label: String, value: String) -> Div {
    div()
        .flex()
        .gap(px(10.0))
        .py(px(2.0))
        .text_size(sp(12.0))
        .child(
            div()
                .w(px(128.0))
                .flex_none()
                .text_color(theme.text_tertiary)
                .child(label),
        )
        .child(div().flex_1().min_w_0().text_color(theme.text).child(value))
}

fn code_block(theme: &Theme, text: String) -> Div {
    div()
        .mt(px(4.0))
        .p(px(8.0))
        .rounded(px(6.0))
        .bg(theme.code_wash)
        .font_family(md::render::MONO_FAMILY)
        .text_size(sp(11.5))
        .text_color(theme.text)
        .whitespace_normal()
        .child(if text.is_empty() {
            tr!("trajectory.nothing")
        } else {
            text
        })
}

fn inspector_body(
    snapshot: &Snapshot,
    cell: &agent_trajectory::Cell,
    tab: Tab,
    theme: &Theme,
) -> Div {
    let body = div().flex().flex_col().gap(px(2.0));
    match (&cell.detail, tab) {
        (
            CellDetail::User {
                prompt,
                bytes,
                images,
            },
            Tab::Summary,
        ) => body
            .child(fact(
                theme,
                tr!("trajectory.fact.size"),
                tr!("trajectory.bytes", bytes = bytes),
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.images"),
                images.to_string(),
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.started"),
                cell.started_at.map(local_time).unwrap_or_default(),
            ))
            .child(code_block(theme, prompt.clone())),
        (CellDetail::User { prompt, .. }, _) => body.child(code_block(theme, prompt.clone())),
        (CellDetail::Request(request), Tab::Summary) => {
            let status = match request.status {
                RequestStatus::Complete => tr!("trajectory.status.completed"),
                RequestStatus::Running => tr!("trajectory.status.running"),
                RequestStatus::Error => tr!("trajectory.status.failed"),
                RequestStatus::Cancelled => tr!("trajectory.status.cancelled"),
            };
            let purpose = match request.purpose {
                Purpose::Assistant => tr!("trajectory.purpose.assistant"),
                Purpose::Subagent => tr!("trajectory.purpose.subagent"),
            };
            let mut body = body
                .child(fact(
                    theme,
                    tr!("trajectory.fact.request"),
                    tr!("trajectory.request", number = request.number),
                ))
                .child(fact(theme, tr!("trajectory.fact.status"), status))
                .child(fact(theme, tr!("trajectory.fact.purpose"), purpose))
                .child(fact(
                    theme,
                    tr!("trajectory.fact.provider"),
                    request.provider.clone(),
                ))
                .child(fact(
                    theme,
                    tr!("trajectory.fact.model"),
                    request.model.clone(),
                ))
                .child(fact(
                    theme,
                    tr!("trajectory.fact.wire_format"),
                    request.wire_format.clone(),
                ))
                .child(fact(
                    theme,
                    tr!("trajectory.fact.stop_reason"),
                    request.stop_reason.clone(),
                ))
                .child(fact(
                    theme,
                    tr!("trajectory.fact.tool_calls"),
                    request.tool_calls.to_string(),
                ));
            for retry in &request.retries {
                body = body.child(fact(theme, tr!("trajectory.fact.retry"), retry_line(retry)));
            }
            if let Some(error) = &request.error {
                body = body.child(fact(theme, tr!("trajectory.fact.error"), error.clone()));
            }
            // Fork: the failure the request ended its turn with.
            if let Some(failure) = &request.failure {
                let class = match failure.class.as_str() {
                    "retryable" => tr!("trajectory.failure_class.retryable"),
                    "context_overflow" => tr!("trajectory.failure_class.context_overflow"),
                    "fatal" => tr!("trajectory.failure_class.fatal"),
                    other => other.to_owned(),
                };
                let class = if failure.class == "retryable" && !failure.reason.is_empty() {
                    format!(
                        "{class} · {}",
                        RetryCause::from_reason(&failure.reason).label()
                    )
                } else {
                    class
                };
                body = body.child(fact(theme, tr!("trajectory.fact.failure"), class));
                if let Some(status) = failure.status {
                    body = body.child(fact(
                        theme,
                        tr!("trajectory.fact.http_status"),
                        status.to_string(),
                    ));
                }
                if let Some(code) = &failure.code {
                    body = body.child(fact(theme, tr!("trajectory.fact.error_code"), code.clone()));
                }
                if let Some(id) = &failure.request_id {
                    body = body.child(fact(theme, tr!("trajectory.fact.request_id"), id.clone()));
                }
            }
            body
        }
        (CellDetail::Request(request), Tab::Options) => body
            .child(fact(
                theme,
                tr!("trajectory.fact.api_base"),
                request.api_base.clone(),
            ))
            .child(code_block(
                theme,
                serde_json::to_string_pretty(&request.options).unwrap_or_default(),
            )),
        (CellDetail::Request(request), Tab::Usage) => {
            let row = |label: String, usage: Option<agent_trajectory::record::Usage>| {
                let value = usage
                    .map(|usage| {
                        tr!(
                            "trajectory.usage_line",
                            input = usage.input,
                            cache_read = usage.cache_read,
                            cache_write = usage.cache_write,
                            output = usage.output
                        )
                    })
                    .unwrap_or_else(|| tr!("trajectory.usage_not_reported"));
                fact(theme, label, value)
            };
            body.child(row(tr!("trajectory.usage.this_request"), request.usage))
                .child(row(
                    tr!("trajectory.usage.session"),
                    Some(request.cumulative),
                ))
                .child(fact(
                    theme,
                    tr!("trajectory.usage.thinking"),
                    tr!("trajectory.thinking_chars", chars = request.thinking_chars),
                ))
        }
        (CellDetail::Request(request), Tab::Timing) => {
            let throughput = match metrics::throughput(request) {
                Timing::Value(rate) => {
                    tr!("trajectory.tokens_per_second", rate = format!("{rate:.1}"))
                }
                Timing::FirstTokenUnavailable => tr!("trajectory.timing.first_token_unavailable"),
                Timing::UsageUnavailable => tr!("trajectory.timing.usage_unavailable"),
                Timing::TooShort => tr!("trajectory.timing.too_short"),
            };
            body.child(fact(
                theme,
                tr!("trajectory.fact.started"),
                local_time(request.started_at),
            ))
            .child(fact(
                theme,
                tr!("trajectory.timing.total"),
                metrics::format_duration(metrics::total(request)),
            ))
            .child(fact(
                theme,
                tr!("trajectory.timing.ttft"),
                timing_text(metrics::ttft(request)),
            ))
            .child(fact(
                theme,
                tr!("trajectory.timing.generation"),
                timing_text(metrics::generation(request)),
            ))
            .child(fact(theme, tr!("trajectory.timing.throughput"), throughput))
            .child(
                div()
                    .pt(px(6.0))
                    .text_size(sp(11.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("trajectory.timing.inferred_start")),
            )
        }
        (CellDetail::Request(request), _) => {
            let mut body = body;
            if !request.thinking.is_empty() {
                body = body
                    .child(fact(theme, tr!("trajectory.usage.thinking"), String::new()))
                    .child(code_block(theme, request.thinking.clone()));
            }
            body.child(code_block(theme, request.text.clone()))
        }
        (CellDetail::Tool(tool), Tab::Summary) => body
            .child(fact(theme, tr!("trajectory.fact.tool"), tool.name.clone()))
            .child(fact(
                theme,
                tr!("trajectory.fact.call_id"),
                tool.call_id.clone(),
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.status"),
                if tool.ended_at.is_none() {
                    tr!("trajectory.status.running")
                } else if tool.is_error {
                    tr!("trajectory.status.failed")
                } else {
                    tr!("trajectory.status.completed")
                },
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.size"),
                tr!(
                    "trajectory.payload_sizes",
                    args = tool.args_bytes,
                    result = tool.result_bytes
                ),
            ))
            .when_some(tool.parent_call_id.clone(), |body, parent| {
                body.child(fact(theme, tr!("trajectory.fact.parent"), parent))
            }),
        (CellDetail::Tool(tool), Tab::Payload) => {
            body.child(code_block(theme, pretty_json(&tool.args)))
        }
        (CellDetail::Tool(tool), Tab::Result) => body.child(code_block(
            theme,
            tool.result
                .clone()
                .unwrap_or_else(|| tr!("trajectory.no_result")),
        )),
        (CellDetail::Tool(tool), Tab::Schema) => body.child(code_block(
            theme,
            tool.schema
                .as_ref()
                .and_then(|schema| serde_json::to_string_pretty(schema).ok())
                .unwrap_or_else(|| tr!("trajectory.schema_unavailable")),
        )),
        (CellDetail::Tool(tool), _) => body
            .child(fact(
                theme,
                tr!("trajectory.fact.started"),
                local_time(tool.started_at),
            ))
            .child(fact(
                theme,
                tr!("trajectory.timing.total"),
                tool.ended_at
                    .map(|end| metrics::format_duration(end - tool.started_at))
                    .unwrap_or_else(|| tr!("trajectory.status.running")),
            )),
        (
            CellDetail::Compaction {
                automatic,
                phase,
                tokens_before,
                tokens_after,
            },
            _,
        ) => body
            .child(fact(
                theme,
                tr!("trajectory.fact.trigger"),
                if *automatic {
                    tr!("trajectory.automatic")
                } else {
                    tr!("trajectory.manual")
                },
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.status"),
                format!("{phase:?}"),
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.before"),
                metrics::format_tokens(*tokens_before),
            ))
            .child(fact(
                theme,
                tr!("trajectory.fact.after"),
                tokens_after.map(metrics::format_tokens).unwrap_or_default(),
            ))
            .child(fact(
                theme,
                tr!("trajectory.timing.total"),
                cell.duration_ms
                    .map(metrics::format_duration)
                    .unwrap_or_default(),
            )),
        (CellDetail::System { .. }, _) => {
            let Some((system, previous_system, tools, previous_tools)) = snapshot.prompt_of(cell)
            else {
                return body;
            };
            match tab {
                Tab::Diff => {
                    let mut body = body;
                    let changes = tool_changes(
                        previous_tools.unwrap_or_default(),
                        tools.unwrap_or_default(),
                    );
                    if !changes.is_empty() {
                        body = body.child(fact(
                            theme,
                            tr!("trajectory.tab.tools"),
                            tr!(
                                "trajectory.tool_changes",
                                added = changes.added.join(", "),
                                removed = changes.removed.join(", "),
                                changed = changes.changed.join(", ")
                            ),
                        ));
                    }
                    let lines = line_diff(
                        previous_system.unwrap_or_default(),
                        system.unwrap_or_default(),
                    );
                    let changed: Vec<_> = lines
                        .into_iter()
                        .filter(|line| line.tag != LineTag::Same)
                        .collect();
                    body.child(
                        div()
                            .mt(px(4.0))
                            .font_family(md::render::MONO_FAMILY)
                            .text_size(sp(11.5))
                            .children(changed.into_iter().map(|line| {
                                let (marker, color) = match line.tag {
                                    LineTag::Added => ("+", theme.success),
                                    LineTag::Removed => ("-", theme.danger),
                                    LineTag::Same => (" ", theme.text_tertiary),
                                };
                                div()
                                    .whitespace_normal()
                                    .text_color(color)
                                    .child(format!("{marker} {}", line.text))
                            })),
                    )
                }
                Tab::Tools => body.children(tools.unwrap_or_default().iter().map(|tool| {
                    fact(
                        theme,
                        tool.name.clone(),
                        tool.description
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_owned(),
                    )
                })),
                _ => body
                    .child(
                        div()
                            .pb(px(4.0))
                            .text_size(sp(11.0))
                            .text_color(theme.text_tertiary)
                            .child(tr!("trajectory.system_reconstructed")),
                    )
                    .child(code_block(theme, system.unwrap_or_default().to_owned())),
            }
        }
    }
}

/// The overview: a lane per kind of work, each record a bar, a model bar
/// lighter until its first token, the picked record outlined and the picked
/// interval washed.
fn paint_timeline(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    model: &timeline::Model,
    selected: Option<usize>,
    focus: Option<(f64, f64)>,
    theme: &Theme,
) {
    let left = f32::from(bounds.origin.x) + 60.0;
    let width = (f32::from(bounds.size.width) - 60.0 - TIMELINE_PADDING).max(1.0);
    let top = f32::from(bounds.origin.y) + TIMELINE_PADDING;
    let domain = (model.end - model.start).max(1e-9);
    let x_of = |value: f64| left + ((value - model.start) / domain) as f32 * width;
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for lane in 0..3u8 {
            let y = top + f32::from(lane) * (LANE_HEIGHT + LANE_GAP);
            window.paint_quad(
                fill(
                    Bounds {
                        origin: point(px(left), px(y)),
                        size: size(px(width), px(LANE_HEIGHT)),
                    },
                    theme.overlay,
                )
                .corner_radii(px(3.0)),
            );
        }
        if let Some((start, end)) = focus {
            let x = x_of(start);
            window.paint_quad(fill(
                Bounds {
                    origin: point(px(x), bounds.origin.y),
                    size: size(px((x_of(end) - x).max(1.0)), bounds.size.height),
                },
                theme.accent.opacity(0.12),
            ));
        }
        for boundary in &model.turns {
            window.paint_quad(fill(
                Bounds {
                    origin: point(px(x_of(boundary.at)), bounds.origin.y),
                    size: size(px(1.0), bounds.size.height),
                },
                theme.border,
            ));
        }
        for span in &model.spans {
            let y = top + f32::from(span.lane) * (LANE_HEIGHT + LANE_GAP);
            let x = x_of(span.start);
            let w = (x_of(span.end) - x).max(2.0);
            let color = if span.is_error {
                theme.danger
            } else {
                lane_color(span.lane, theme)
            };
            let bar = Bounds {
                origin: point(px(x), px(y + 1.0)),
                size: size(px(w), px(LANE_HEIGHT - 2.0)),
            };
            match span.first_token {
                // The wait for the first token, lighter than the decoding.
                Some(first) if first > span.start && first < span.end => {
                    let split = x_of(first);
                    window.paint_quad(fill(
                        Bounds {
                            origin: bar.origin,
                            size: size(px((split - x).max(1.0)), bar.size.height),
                        },
                        color.opacity(0.35),
                    ));
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(px(split), bar.origin.y),
                            size: size(px((x + w - split).max(1.0)), bar.size.height),
                        },
                        color.opacity(0.9),
                    ));
                }
                _ => window.paint_quad(fill(bar, color.opacity(0.8)).corner_radii(px(2.0))),
            }
            if selected == Some(span.index) {
                window.paint_quad(
                    fill(bar, gpui::transparent_black())
                        .border_widths(px(1.5))
                        .border_color(theme.text),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_trajectory::record::{RequestOptions, Usage};

    fn records() -> Vec<Record> {
        vec![
            Record::TurnStart {
                turn: 1,
                at: 0,
                prompt_preview: "go".into(),
                prompt_bytes: 2,
                images: 0,
                provider: "anthropic".into(),
                model: "m".into(),
                api_base: String::new(),
                wire_format: String::new(),
                options: RequestOptions::default(),
                system_fp: "s".into(),
                tools_fp: "t".into(),
            },
            Record::Compaction {
                turn: None,
                phase: CompactionPhase::Started,
                automatic: false,
                tokens_before: 1,
                tokens_after: None,
                at: 1,
            },
            Record::TurnEnd {
                turn: 1,
                success: true,
                summary: None,
                session_usage: Usage::default(),
                at: 2,
            },
        ]
    }

    #[test]
    fn the_ledger_folds_a_turn_and_filters_to_an_interval() {
        let snapshot = agent_trajectory::build(&records());
        // System, user, compaction; one turn holding all three.
        let rows = ledger_rows(&snapshot, &HashSet::new(), None);
        assert_eq!(rows[0], Row::Turn(0));
        assert_eq!(rows.len(), 1 + snapshot.cells.len());
        let folded: HashSet<usize> = [0].into_iter().collect();
        let rows = ledger_rows(&snapshot, &folded, None);
        assert!(
            rows.iter().all(
                |row| !matches!(row, Row::Cell(index) if *index < snapshot.turns[0].cell_count)
            )
        );
        let focus: HashSet<usize> = [1].into_iter().collect();
        let rows = ledger_rows(&snapshot, &HashSet::new(), Some(&focus));
        assert_eq!(rows, vec![Row::Turn(0), Row::Cell(1)]);
    }

    #[test]
    fn every_kind_has_tabs_and_a_prompt_change_shows_its_diff_first() {
        assert_eq!(tabs_for(CellKind::System, true)[0], Tab::Diff);
        assert_eq!(tabs_for(CellKind::System, false)[0], Tab::SystemPrompt);
        assert!(tabs_for(CellKind::Tool, false).contains(&Tab::Payload));
        assert!(tabs_for(CellKind::Message, false).contains(&Tab::Timing));
    }
}
