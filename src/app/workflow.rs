//! Settings → Workflow: staged multi-agent runs.
//!
//! Fork addition. A run is a small graph of nodes, each of which becomes one
//! agent session sharing a worktree with the others; the model, scheduler
//! and store live in the `workflow-engine` crate, and this file is the view plus the
//! glue that turns nodes into sessions. Upstream reaches it through three
//! seams: the settings-page dispatch arm, the turn-settlement seam (which
//! records that a session's turn closed), and the event pump (which acts on
//! those records with a `Context` in hand).
//!
//! The graph view draws nodes as ordinary positioned `div`s — so hover,
//! focus, tooltips and clicks come for free — and the edges on one canvas
//! layer beneath them. Pan is an offset added to every coordinate.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use gpui::{ClickEvent, PathBuilder, ScrollWheelEvent};
use waku_protocol::model::{InteractionMode, SessionWorkspace, TurnStatus};
use workflow_engine::check as workflow_check;
use workflow_engine::model::{self as wf, NodeStatus, RunStatus, WorkflowRun, WorkflowStore};
use workflow_engine::planner::{self as orchestrator, Action};

use super::composer::picker_rail_shows_provider;
use super::providers_page::card_button;
use super::*;
use crate::ui::ActivationExt as _;
use crate::ui::text_field::TextField;

/// Where the graph starts when a run is opened, so the first column is not
/// glued to the edge.
const CANVAS_MARGIN: (f32, f32) = (24.0, 24.0);
/// Pointer travel before a press counts as a drag rather than a click.
const DRAG_THRESHOLD: f32 = 3.0;
const INSPECTOR_WIDTH: f32 = 340.0;
/// A stage's check command may run a whole test suite.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Which of the page's three views is up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum WorkflowView {
    #[default]
    List,
    New,
    Detail(String),
}

/// One row of the form's role table.
#[derive(Clone, Debug)]
pub(super) struct RoleRow {
    pub role: String,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub plan_mode: bool,
    pub writes: bool,
    pub prompt_hint: String,
    pub after: Option<(usize, wf::EdgeKind)>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct NewRunForm {
    pub project_id: Option<Uuid>,
    /// `None` = custom roles.
    pub template_id: Option<String>,
    pub roles: Vec<RoleRow>,
    pub planner_enabled: bool,
    pub error: Option<String>,
}

/// A press on the canvas or on a node, until the button is released.
#[derive(Clone, Debug)]
pub(super) struct CanvasDrag {
    start_mouse: (f32, f32),
    start_pan: (f32, f32),
    /// `Some` when a node is being moved rather than the view panned.
    node: Option<String>,
    node_start: (f32, f32),
    moved: bool,
}

#[derive(Default)]
pub(super) struct WorkflowState {
    pub store: WorkflowStore,
    pub loaded: bool,
    /// Render schedules the load; this keeps it from scheduling twice.
    load_scheduled: Cell<bool>,
    pub view: WorkflowView,
    pub form: NewRunForm,
    /// Created on first use — a text input needs the window.
    pub goal_input: Option<Entity<TextInput>>,
    pub check_input: Option<Entity<TextInput>>,
    pub error: Option<String>,
    /// Turns that settled, recorded at the settlement seam (which has no
    /// `Context`) and acted on by the event pump: `(session, turn, status)`.
    pub pending_settles: Vec<(Uuid, Uuid, TurnStatus)>,
    /// Live stage sessions → `(run id, node id)`.
    pub node_of_session: HashMap<Uuid, (String, String)>,
    /// Check commands running after a stage's turn, keyed `(run, node)`.
    pub checks_in_flight: HashSet<(String, String)>,
    // --- canvas ---
    pub selected_node: Option<String>,
    pub canvas_pan: (f32, f32),
    pub canvas_drag: Option<CanvasDrag>,
    // --- planner ---
    pub planner_busy: bool,
    /// Bumped per request; a reply to an older request is dropped.
    pub planner_generation: u64,
}

impl WorkflowState {
    /// The detail view pans its own canvas, so its page fills the viewport
    /// instead of riding the shared settings scroll.
    pub fn fills_viewport(&self) -> bool {
        matches!(self.view, WorkflowView::Detail(_))
    }

    fn current_run_id(&self) -> Option<&str> {
        match &self.view {
            WorkflowView::Detail(id) => Some(id.as_str()),
            _ => None,
        }
    }
}

pub(super) fn provider_from_id(id: &str) -> Option<ProviderKind> {
    ProviderKind::ALL.into_iter().find(|kind| kind.id() == id)
}

fn run_status_label(status: RunStatus) -> String {
    match status {
        RunStatus::Draft => tr!("workflow.status_draft"),
        RunStatus::Planning => tr!("workflow.status_planning"),
        RunStatus::AwaitingApproval => tr!("workflow.status_awaiting"),
        RunStatus::Running => tr!("workflow.status_running"),
        RunStatus::Paused => tr!("workflow.status_paused"),
        RunStatus::Done => tr!("workflow.status_done"),
        RunStatus::Failed => tr!("workflow.status_failed"),
    }
}

fn run_status_style(theme: &Theme, status: RunStatus) -> (&'static str, gpui::Hsla) {
    match status {
        RunStatus::Draft => ("icons/pencil.svg", theme.text_tertiary),
        RunStatus::Planning => ("icons/sparkle.svg", theme.accent),
        RunStatus::AwaitingApproval => ("icons/bell.svg", theme.warning),
        RunStatus::Running => ("icons/loader-circle.svg", theme.accent),
        RunStatus::Paused => ("icons/stop.svg", theme.warning),
        RunStatus::Done => ("icons/check.svg", theme.success),
        RunStatus::Failed => ("icons/circle-x.svg", theme.danger),
    }
}

pub(super) fn node_status_label(status: NodeStatus) -> String {
    match status {
        NodeStatus::Proposed => tr!("workflow.node_proposed"),
        NodeStatus::Pending => tr!("workflow.node_pending"),
        NodeStatus::Ready => tr!("workflow.node_ready"),
        NodeStatus::Running => tr!("workflow.node_running"),
        NodeStatus::AwaitingInput => tr!("workflow.node_awaiting"),
        NodeStatus::Done => tr!("workflow.node_done"),
        NodeStatus::Failed => tr!("workflow.node_failed"),
        NodeStatus::Skipped => tr!("workflow.node_skipped"),
        NodeStatus::Canceled => tr!("workflow.node_canceled"),
    }
}

pub(super) fn node_status_style(theme: &Theme, status: NodeStatus) -> (&'static str, gpui::Hsla) {
    match status {
        NodeStatus::Proposed => ("icons/sparkle.svg", theme.accent),
        NodeStatus::Pending => ("icons/lock.svg", theme.text_tertiary),
        NodeStatus::Ready => ("icons/queue.svg", theme.text_secondary),
        NodeStatus::Running => ("icons/loader-circle.svg", theme.accent),
        NodeStatus::AwaitingInput => ("icons/bell.svg", theme.warning),
        NodeStatus::Done => ("icons/check.svg", theme.success),
        NodeStatus::Failed => ("icons/circle-x.svg", theme.danger),
        NodeStatus::Skipped => ("icons/block.svg", theme.text_ghost),
        NodeStatus::Canceled => ("icons/x.svg", theme.text_ghost),
    }
}

fn budget_stop_label(stop: wf::BudgetStop) -> String {
    match stop {
        wf::BudgetStop::Nodes => tr!("workflow.budget_nodes"),
        wf::BudgetStop::PlannerCalls => tr!("workflow.budget_planner"),
        wf::BudgetStop::WallClock => tr!("workflow.budget_wall"),
        wf::BudgetStop::Balance => tr!("workflow.budget_balance"),
    }
}

/// Icon + text + colour, never colour alone.
pub(super) fn status_chip(icon_path: &'static str, label: String, color: gpui::Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .flex_none()
        .child(icon(icon_path, 11.0, color))
        .child(div().text_size(sp(11.5)).text_color(color).child(label))
}

fn format_elapsed(ms: i64) -> String {
    let seconds = ms / 1000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

/// The prompt a stage's session receives: its role, its own instructions,
/// what the stages it depends on reported, and the worktree rules. Written
/// for the agent, so it is not localized.
fn compose_node_prompt(run: &WorkflowRun, node: &wf::Node) -> String {
    let mut text = format!(
        "You are the \"{}\" stage of a multi-stage workflow. Other stages run as \
         separate agent sessions in the same worktree, one after another.\n\n{}\n",
        node.role,
        node.prompt.trim()
    );
    for edge in run.dependencies_of(&node.id) {
        let Some(previous) = run.node(&edge.from) else {
            continue;
        };
        let Some(summary) = previous.summary.as_deref() else {
            continue;
        };
        match edge.kind {
            wf::EdgeKind::DependsOn => text.push_str(&format!(
                "\n## Output of the previous stage \"{}\"\n{summary}\n",
                previous.role
            )),
            wf::EdgeKind::OnFailure => text.push_str(&format!(
                "\n## The stage \"{}\" failed; its report\n{summary}\n",
                previous.role
            )),
        }
    }
    text.push_str(if node.writes {
        "\nYou may edit files in this worktree. Do not commit, merge, or push."
    } else {
        "\nDo not edit any file; put your findings in your final message."
    });
    text.push_str(
        "\nFinish with a short summary of what you did or found — the next stage reads it.\n",
    );
    text
}

impl Waku {
    // --- loading and saving --------------------------------------------

    fn load_workflows_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.workflow.loaded {
            return;
        }
        cx.spawn(async move |this, cx| {
            let store = cx
                .background_executor()
                .spawn(async move {
                    let mut store = wf::load();
                    // No session survives a restart, so nothing in the file
                    // can still be running.
                    store.reconcile_after_restart(wf::now_ms());
                    store
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.workflow.store = store;
                this.workflow.loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_workflows(&mut self, cx: &mut Context<Self>) {
        let mut store = self.workflow.store.clone();
        store.trim(wf::KEEP_FINISHED_RUNS);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = wf::save(&store) {
                    eprintln!("could not save workflows: {error:#}");
                }
            })
            .detach();
    }

    // --- installed agents ----------------------------------------------

    /// Agents the form may assign: installed and not switched off.
    pub(super) fn workflow_available_providers(&self) -> Vec<ProviderKind> {
        ProviderKind::ALL
            .into_iter()
            .filter(|kind| {
                picker_rail_shows_provider(
                    &self.probes,
                    &self.state.disabled_providers,
                    None,
                    *kind,
                )
            })
            .collect()
    }

    fn workflow_default_provider(&self) -> Option<ProviderKind> {
        let available = self.workflow_available_providers();
        self.selected_session()
            .map(|session| session.provider)
            .filter(|provider| available.contains(provider))
            .or_else(|| available.first().copied())
    }

    // --- form ----------------------------------------------------------

    pub(super) fn open_workflow_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workflow.goal_input.is_none() {
            let input = cx.new(|cx| {
                TextInput::new(window, cx)
                    .multi_line()
                    .auto_height()
                    .placeholder(tr!("workflow.goal_placeholder"))
            });
            self.workflow.goal_input = Some(input);
        }
        if self.workflow.check_input.is_none() {
            let input = cx.new(|cx| {
                TextInput::new(window, cx).placeholder(tr!("workflow.check_placeholder"))
            });
            self.workflow.check_input = Some(input);
        }
        let project_id = self
            .selected_session()
            .map(|session| session.project_id)
            .or(self.state.selected_project)
            .or_else(|| self.state.projects.first().map(|project| project.id));
        self.workflow.form = NewRunForm {
            project_id,
            ..NewRunForm::default()
        };
        let first_template = wf::builtin_templates()
            .first()
            .map(|template| template.id.clone());
        self.set_workflow_template(first_template, cx);
        self.workflow.view = WorkflowView::New;
        cx.notify();
    }

    /// Fill the goal from the selected session: its title and the agent's
    /// last reply, so a plan drafted in chat can be handed to a workflow
    /// without retyping it.
    pub(super) fn seed_workflow_goal_from_session(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let mut text = session.display_title().to_string();
        if let Some(reply) = session
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Assistant)
        {
            let body = reply.display_content.as_deref().unwrap_or(&reply.content);
            let body = body.trim();
            if !body.is_empty() {
                text.push_str("\n\n");
                text.push_str(body);
            }
        }
        if let Some(input) = &self.workflow.goal_input {
            input.update(cx, |input, cx| input.set_content(text, cx));
        }
        cx.notify();
    }

    pub(super) fn set_workflow_template(
        &mut self,
        template_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.workflow_default_provider() else {
            self.workflow.form.roles.clear();
            self.workflow.form.template_id = template_id;
            self.workflow.form.error = Some(tr!("workflow.no_agents"));
            cx.notify();
            return;
        };
        let roles = match template_id
            .as_deref()
            .and_then(|id| wf::builtin_templates().into_iter().find(|t| t.id == id))
        {
            Some(template) => template
                .roles
                .iter()
                .map(|role| RoleRow {
                    role: role.role.clone(),
                    provider,
                    model: None,
                    plan_mode: role.plan_mode,
                    writes: role.writes,
                    prompt_hint: role.prompt_hint.clone(),
                    after: role.after,
                })
                .collect(),
            None => vec![RoleRow {
                role: tr!("workflow.custom_role_name", n = 1),
                provider,
                model: None,
                plan_mode: false,
                writes: true,
                prompt_hint: String::new(),
                after: None,
            }],
        };
        self.workflow.form.template_id = template_id;
        self.workflow.form.roles = roles;
        self.workflow.form.error = None;
        cx.notify();
    }

    pub(super) fn add_workflow_custom_role(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.workflow_default_provider() else {
            return;
        };
        let index = self.workflow.form.roles.len();
        self.workflow.form.roles.push(RoleRow {
            role: tr!("workflow.custom_role_name", n = index + 1),
            provider,
            model: None,
            plan_mode: false,
            writes: true,
            prompt_hint: String::new(),
            after: (index > 0).then_some((index - 1, wf::EdgeKind::DependsOn)),
        });
        cx.notify();
    }

    pub(super) fn remove_workflow_role(&mut self, index: usize, cx: &mut Context<Self>) {
        let roles = &mut self.workflow.form.roles;
        if index >= roles.len() || roles.len() == 1 {
            return;
        }
        roles.remove(index);
        // Re-chain custom roles so nothing points past the end.
        for (i, row) in roles.iter_mut().enumerate() {
            if let Some((after, _)) = row.after
                && after >= i
            {
                row.after = (i > 0).then_some((i - 1, wf::EdgeKind::DependsOn));
            }
        }
        cx.notify();
    }

    pub(super) fn set_workflow_role_provider(
        &mut self,
        index: usize,
        provider: ProviderKind,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.workflow.form.roles.get_mut(index) {
            row.provider = provider;
            row.model = None;
        }
        cx.notify();
    }

    pub(super) fn set_workflow_role_model(
        &mut self,
        index: usize,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.workflow.form.roles.get_mut(index) {
            row.model = model;
        }
        cx.notify();
    }

    pub(super) fn toggle_workflow_role_plan(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.workflow.form.roles.get_mut(index) {
            row.plan_mode = !row.plan_mode;
        }
        cx.notify();
    }

    pub(super) fn toggle_workflow_role_writes(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.workflow.form.roles.get_mut(index) {
            row.writes = !row.writes;
        }
        cx.notify();
    }

    pub(super) fn set_workflow_project(&mut self, project_id: Uuid, cx: &mut Context<Self>) {
        self.workflow.form.project_id = Some(project_id);
        cx.notify();
    }

    pub(super) fn toggle_workflow_planner(&mut self, cx: &mut Context<Self>) {
        self.workflow.form.planner_enabled = !self.workflow.form.planner_enabled;
        cx.notify();
    }

    /// Turn the form into a stored run and open it.
    pub(super) fn create_workflow_run(&mut self, cx: &mut Context<Self>) {
        let goal = self
            .workflow
            .goal_input
            .as_ref()
            .map(|input| input.read(cx).content().trim().to_owned())
            .unwrap_or_default();
        if goal.is_empty() {
            self.workflow.form.error = Some(tr!("workflow.needs_goal"));
            cx.notify();
            return;
        }
        let Some(project_id) = self.workflow.form.project_id else {
            self.workflow.form.error = Some(tr!("workflow.needs_project"));
            cx.notify();
            return;
        };
        if self.workflow.form.roles.is_empty() {
            self.workflow.form.error = Some(tr!("workflow.needs_roles"));
            cx.notify();
            return;
        }

        let mut run = WorkflowRun::new(&goal, &project_id.to_string());
        run.planner_enabled = self.workflow.form.planner_enabled;
        // The project's own check, attached to every stage that writes.
        let check_command = self
            .workflow
            .check_input
            .as_ref()
            .map(|input| input.read(cx).content().trim().to_owned())
            .filter(|command| !command.is_empty());
        let mut patch = wf::Patch::default();
        for (index, row) in self.workflow.form.roles.iter().enumerate() {
            let id = format!("n{}", index + 1);
            let prompt = if row.prompt_hint.trim().is_empty() {
                format!("{}\n\nGoal:\n{goal}", row.role)
            } else {
                format!("{}\n\nGoal:\n{goal}", row.prompt_hint)
            };
            let mut node = wf::Node::new(&id, &row.role, row.provider.id(), &prompt)
                .writes(row.writes)
                .plan_mode(row.plan_mode);
            node.model = row.model.clone();
            if row.writes {
                node.check_command = check_command.clone();
            }
            patch.nodes.push(node);
            if let Some((after, kind)) = row.after {
                patch.edges.push(wf::Edge {
                    from: format!("n{}", after + 1),
                    to: id,
                    kind,
                });
            }
        }
        let available = self.workflow_available_providers();
        let allowed: Vec<&str> = available.iter().map(|kind| kind.id()).collect();
        if let Err(error) = run.apply_patch(patch, &allowed) {
            self.workflow.form.error = Some(error.to_string());
            cx.notify();
            return;
        }
        // The user built this graph; it needs no approval step.
        run.approve_proposed();
        run.push_event(wf::EventKind::RunCreated, None, "form");
        wf::layout(&mut run, &wf::LayoutOptions::default());
        let id = run.id.clone();
        self.workflow.store.upsert(run);
        self.save_workflows(cx);
        self.workflow.form.error = None;
        self.open_workflow_run(&id, cx);
    }

    pub(super) fn open_workflow_run(&mut self, id: &str, cx: &mut Context<Self>) {
        self.workflow.view = WorkflowView::Detail(id.to_owned());
        self.workflow.selected_node = None;
        self.workflow.canvas_pan = CANVAS_MARGIN;
        self.workflow.canvas_drag = None;
        cx.notify();
    }

    pub(super) fn back_to_workflow_list(&mut self, cx: &mut Context<Self>) {
        self.workflow.view = WorkflowView::List;
        self.workflow.selected_node = None;
        self.workflow.canvas_drag = None;
        cx.notify();
    }

    pub(super) fn confirm_delete_workflow_run(&mut self, id: String, cx: &mut Context<Self>) {
        let title = self
            .workflow
            .store
            .get(&id)
            .map(|run| run.title.clone())
            .unwrap_or_default();
        self.request_confirm(
            tr!("workflow.delete_run_confirm", title = title),
            Some(tr!("workflow.delete_run_confirm_detail")),
            tr!("workflow.delete_run"),
            true,
            cx,
            move |this, _, cx| {
                this.workflow.store.remove(&id);
                this.workflow
                    .node_of_session
                    .retain(|_, (run_id, _)| *run_id != id);
                if this.workflow.view == WorkflowView::Detail(id.clone()) {
                    this.workflow.view = WorkflowView::List;
                    this.workflow.selected_node = None;
                }
                this.save_workflows(cx);
                cx.notify();
            },
        );
    }

    // --- running -------------------------------------------------------

    pub(super) fn start_workflow_run(&mut self, run_id: &str, cx: &mut Context<Self>) {
        let title = {
            let Some(run) = self.workflow.store.get_mut(run_id) else {
                return;
            };
            if !matches!(run.status, RunStatus::Draft | RunStatus::Paused) {
                return;
            }
            run.status = RunStatus::Running;
            run.push_event(wf::EventKind::Resumed, None, "start");
            run.title.clone()
        };
        self.show_toast(tr!("workflow.run_started", title = title));
        self.pump_workflow_run(run_id, cx);
    }

    pub(super) fn pause_workflow_run(&mut self, run_id: &str, cx: &mut Context<Self>) {
        let Some(run) = self.workflow.store.get_mut(run_id) else {
            return;
        };
        if run.status != RunStatus::Running {
            return;
        }
        run.status = RunStatus::Paused;
        run.push_event(wf::EventKind::Paused, None, "user");
        self.save_workflows(cx);
        cx.notify();
    }

    // --- planner -------------------------------------------------------

    /// Ask the planner what the run should do next. The call runs off the
    /// UI thread; its reply is applied only if no newer request was made.
    pub(super) fn request_workflow_plan(&mut self, run_id: &str, cx: &mut Context<Self>) {
        if self.workflow.planner_busy {
            return;
        }
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            self.show_toast(tr!("workflow.planner_no_gateway"));
            return;
        };
        let origin = self.cloud_account.gateway_origin.origin();
        let available: Vec<orchestrator::AgentBrief> = self
            .workflow_available_providers()
            .into_iter()
            .map(|kind| orchestrator::AgentBrief {
                id: kind.id().to_owned(),
                name: kind.display_name().to_owned(),
            })
            .collect();
        let allowed: Vec<String> = available.iter().map(|agent| agent.id.clone()).collect();
        let project = self
            .workflow
            .store
            .get(run_id)
            .and_then(|run| Uuid::parse_str(&run.project_id).ok())
            .and_then(|id| self.state.projects.iter().find(|project| project.id == id))
            .map(|project| project.name.clone());
        let input = {
            let Some(run) = self.workflow.store.get_mut(run_id) else {
                return;
            };
            run.status = RunStatus::Planning;
            run.spent.planner_calls += 1;
            run.push_event(wf::EventKind::PlannerCalled, None, "");
            orchestrator::PlannerInput::from_run(run, &available, project)
        };
        self.workflow.planner_busy = true;
        self.workflow.planner_generation += 1;
        let generation = self.workflow.planner_generation;
        let run_id = run_id.to_owned();
        self.save_workflows(cx);
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    sub2api::refresh_if_needed(&mut credentials)?;
                    // Planning uses the gateway key even while CLI routing
                    // is switched off; the key is the account's either way.
                    let config =
                        sub2api::gateway_config_with_origin(&credentials, true, origin.as_deref());
                    let decision =
                        orchestrator::plan(&config, orchestrator::DEFAULT_PLANNER_MODEL, &input)?;
                    anyhow::Ok((credentials, decision))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.workflow.planner_busy = false;
                if this.workflow.planner_generation != generation {
                    return;
                }
                this.apply_workflow_decision(&run_id, result, &allowed, cx);
            });
        })
        .detach();
    }

    fn apply_workflow_decision(
        &mut self,
        run_id: &str,
        result: anyhow::Result<(sub2api::Credentials, orchestrator::Decision)>,
        allowed: &[String],
        cx: &mut Context<Self>,
    ) {
        let (renewed, decision) = match result {
            Err(error) => {
                if sub2api::session_ended(&error) {
                    self.end_cloud_session(cx);
                }
                let message = format!("{error:#}");
                if let Some(run) = self.workflow.store.get_mut(run_id) {
                    run.status = RunStatus::Paused;
                    run.push_event(wf::EventKind::PlannerFailed, None, message.clone());
                }
                self.show_toast(tr!("workflow.planner_failed", error = message));
                self.save_workflows(cx);
                cx.notify();
                return;
            }
            Ok(reply) => reply,
        };
        self.adopt_cloud_tokens(renewed);

        let action = decision.action.clone();
        let message = decision.message.clone().unwrap_or_default();
        let reasoning = decision.reasoning.clone();
        let allowed: Vec<&str> = allowed.iter().map(String::as_str).collect();
        let mut toast = None;
        let mut pump = false;
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            run.push_event(wf::EventKind::Planned, None, reasoning);
            match action {
                Action::AddNodes => match run.apply_patch(decision.into_patch(), &allowed) {
                    Ok(_) => {
                        wf::layout(run, &wf::LayoutOptions::default());
                        run.status = RunStatus::AwaitingApproval;
                    }
                    Err(error) => {
                        run.status = RunStatus::Paused;
                        run.push_event(wf::EventKind::PlannerFailed, None, error.to_string());
                        toast = Some(tr!("workflow.planner_failed", error = error.to_string()));
                    }
                },
                Action::Done => {
                    run.planner_done = true;
                    run.status = RunStatus::Running;
                    pump = true;
                    toast = Some(tr!("workflow.planner_done"));
                }
                Action::AskUser => {
                    run.status = RunStatus::Paused;
                    toast = Some(tr!("workflow.planner_message", message = message));
                }
                Action::Abort => {
                    run.status = RunStatus::Failed;
                    run.push_event(wf::EventKind::Stopped, None, message.clone());
                    toast = Some(tr!("workflow.planner_message", message = message));
                }
            }
        }
        if let Some(toast) = toast {
            self.show_toast(toast);
        }
        if pump {
            self.pump_workflow_run(run_id, cx);
        }
        self.save_workflows(cx);
        cx.notify();
    }

    /// Accept the planner's proposed stages and carry on.
    pub(super) fn approve_workflow_plan(&mut self, run_id: &str, cx: &mut Context<Self>) {
        if let Some(run) = self.workflow.store.get_mut(run_id)
            && run.has_proposed()
        {
            let count = run.approve_proposed();
            run.push_event(wf::EventKind::Approved, None, format!("{count} stages"));
            run.status = RunStatus::Running;
        }
        self.pump_workflow_run(run_id, cx);
    }

    /// Drop the planner's proposal and let the run finish on its own; the
    /// planner is not consulted again unless the user asks.
    pub(super) fn discard_workflow_plan(&mut self, run_id: &str, cx: &mut Context<Self>) {
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            let proposed: Vec<String> = run
                .nodes
                .iter()
                .filter(|node| node.status == NodeStatus::Proposed)
                .map(|node| node.id.clone())
                .collect();
            run.nodes.retain(|node| node.status != NodeStatus::Proposed);
            run.edges
                .retain(|edge| !proposed.contains(&edge.from) && !proposed.contains(&edge.to));
            run.planner_done = true;
            run.status = RunStatus::Running;
            run.push_event(wf::EventKind::Replanned, None, "proposal discarded");
        }
        self.pump_workflow_run(run_id, cx);
    }

    /// Advance a running run: promote nodes whose dependencies are met,
    /// stop on a tripped budget or a finished graph, and start whatever the
    /// scheduler allows.
    fn pump_workflow_run(&mut self, run_id: &str, cx: &mut Context<Self>) {
        enum Next {
            Idle,
            Paused(String),
            Finished(RunStatus, String),
            Start(Vec<String>),
            /// The graph ran out; the planner decides whether the goal is
            /// met or more stages are needed.
            Plan,
        }
        let now = wf::now_ms();
        let balance = self.cloud_account.user.as_ref().map(|user| user.balance);
        let next = {
            let Some(run) = self.workflow.store.get_mut(run_id) else {
                return;
            };
            if run.status != RunStatus::Running {
                Next::Idle
            } else {
                run.promote_ready();
                if let Some(stop) = run.budget_exceeded(now, balance) {
                    let reason = budget_stop_label(stop);
                    run.status = RunStatus::Paused;
                    run.push_event(wf::EventKind::BudgetHit, None, reason.clone());
                    Next::Paused(reason)
                } else if let Some(terminal) = run.terminal_status() {
                    if run.planner_enabled && !run.planner_done {
                        Next::Plan
                    } else {
                        run.status = terminal;
                        run.push_event(wf::EventKind::Stopped, None, format!("{terminal:?}"));
                        Next::Finished(terminal, run.title.clone())
                    }
                } else {
                    Next::Start(run.schedulable(wf::DEFAULT_MAX_READERS))
                }
            }
        };
        match next {
            Next::Idle => {}
            Next::Plan => self.request_workflow_plan(run_id, cx),
            Next::Paused(reason) => self.show_toast(tr!("workflow.budget_hit", reason = reason)),
            Next::Finished(RunStatus::Done, title) => {
                self.show_toast(tr!("workflow.run_finished", title = title))
            }
            Next::Finished(_, title) => self.show_toast(tr!("workflow.run_failed", title = title)),
            Next::Start(ids) => {
                for id in ids {
                    self.start_workflow_node(run_id, &id, cx);
                }
            }
        }
        self.save_workflows(cx);
        cx.notify();
    }

    /// Give a node its own session and send its prompt. The first writing
    /// node materializes the shared worktree; every later node reuses it.
    fn start_workflow_node(&mut self, run_id: &str, node_id: &str, cx: &mut Context<Self>) {
        let prepared = self.workflow.store.get(run_id).and_then(|run| {
            let node = run.node(node_id)?;
            let provider = provider_from_id(&node.provider_id)?;
            let project_id = Uuid::parse_str(&run.project_id).ok()?;
            let workspace = match &run.workspace {
                Some(shared) => SessionWorkspace::Worktree {
                    path: PathBuf::from(&shared.path),
                    branch: shared.branch.clone(),
                },
                None if node.writes => SessionWorkspace::NewWorktree { base_branch: None },
                None => SessionWorkspace::Local,
            };
            Some((
                project_id,
                provider,
                node.model.clone(),
                node.plan_mode,
                compose_node_prompt(run, node),
                workspace,
            ))
        });
        let Some((project_id, provider, model, plan_mode, prompt, workspace)) = prepared else {
            if let Some(run) = self.workflow.store.get_mut(run_id) {
                run.settle_node(
                    node_id,
                    wf::NodeOutcome::Failed {
                        reason: tr!("workflow.agent_unavailable"),
                    },
                    wf::now_ms(),
                );
            }
            return;
        };

        let mut session = self.state.new_session(project_id, provider);
        if model.is_some() {
            session.model = model;
        }
        session.interaction_mode = if plan_mode {
            InteractionMode::Plan
        } else {
            InteractionMode::Build
        };
        session.workspace = workspace;
        // Carry the user's current access mode, as a new task would.
        if let Some(current) = self.selected_session() {
            session.runtime_mode = current.runtime_mode;
        }
        let session_id = session.id;
        self.state.push_session(session);
        self.workflow
            .node_of_session
            .insert(session_id, (run_id.to_owned(), node_id.to_owned()));
        self.submit_submission_for_session(session_id, ComposerSubmission::plain(prompt), cx);

        // The turn is begun synchronously by the submission, so its id is
        // known now; a later turn the user starts in this session must not
        // count as this node settling.
        let turn_id = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(AgentSession::active_turn_id)
            .map(|id| id.to_string());
        let now = wf::now_ms();
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            run.mark_started(node_id, &session_id.to_string(), turn_id.as_deref(), now);
            run.push_event(
                wf::EventKind::NodeStarted,
                Some(node_id),
                format!("session {session_id}"),
            );
        }
    }

    /// Called from the event pump: act on settled turns and keep the
    /// "waiting on the user" status in step with the runtimes.
    pub(super) fn drain_workflow_settles(&mut self, cx: &mut Context<Self>) {
        if self.sync_workflow_awaiting_input() {
            cx.notify();
        }
        if self.workflow.pending_settles.is_empty() {
            return;
        }
        let settles = std::mem::take(&mut self.workflow.pending_settles);
        for (session_id, turn_id, status) in settles {
            self.settle_workflow_session(session_id, turn_id, status, cx);
        }
    }

    /// Mirror pending permission / question requests onto live nodes.
    /// Returns whether anything changed.
    fn sync_workflow_awaiting_input(&mut self) -> bool {
        if self.workflow.node_of_session.is_empty() {
            return false;
        }
        let entries: Vec<(Uuid, String, String)> = self
            .workflow
            .node_of_session
            .iter()
            .map(|(session, (run, node))| (*session, run.clone(), node.clone()))
            .collect();
        let mut changed = false;
        for (session_id, run_id, node_id) in entries {
            let awaiting = self.runtimes.get(&session_id).is_some_and(|runtime| {
                !runtime.pending_permissions.is_empty() || runtime.pending_user_input.is_some()
            });
            if let Some(run) = self.workflow.store.get_mut(&run_id)
                && let Some(node) = run.node(&node_id)
                && node.status.is_live()
                && (node.status == NodeStatus::AwaitingInput) != awaiting
            {
                run.mark_awaiting_input(&node_id, awaiting);
                changed = true;
            }
        }
        changed
    }

    fn settle_workflow_session(
        &mut self,
        session_id: Uuid,
        turn_id: Uuid,
        status: TurnStatus,
        cx: &mut Context<Self>,
    ) {
        let Some((run_id, node_id)) = self.workflow.node_of_session.get(&session_id).cloned()
        else {
            return;
        };
        // Only the turn this node submitted settles it. A follow-up the
        // user typed into the same session is theirs, not the workflow's.
        let expected = self
            .workflow
            .store
            .get(&run_id)
            .and_then(|run| run.node(&node_id))
            .and_then(|node| node.expected_turn_id.clone());
        if let Some(expected) = expected
            && expected != turn_id.to_string()
        {
            return;
        }

        let (summary, worktree, cwd) = {
            let session = self
                .state
                .sessions
                .iter()
                .find(|session| session.id == session_id);
            let summary = session
                .and_then(|session| {
                    session
                        .messages
                        .iter()
                        .rev()
                        .find(|message| {
                            message.role == MessageRole::Assistant
                                && message.turn_id == Some(turn_id)
                        })
                        .or_else(|| {
                            session
                                .messages
                                .iter()
                                .rev()
                                .find(|message| message.role == MessageRole::Assistant)
                        })
                })
                .map(|message| {
                    message
                        .display_content
                        .as_deref()
                        .unwrap_or(&message.content)
                        .trim()
                        .to_owned()
                })
                .filter(|text| !text.is_empty());
            let worktree = session.and_then(|session| match &session.workspace {
                SessionWorkspace::Worktree { path, branch } => Some(wf::WorkspaceRef {
                    path: path.to_string_lossy().into_owned(),
                    branch: branch.clone(),
                }),
                _ => None,
            });
            let cwd = session
                .and_then(|session| self.workspace_path_for_session(session))
                .map(|path| path.to_path_buf());
            (summary, worktree, cwd)
        };
        // The first writing stage created the worktree; every later stage
        // reuses it.
        if let Some(run) = self.workflow.store.get_mut(&run_id)
            && run.workspace.is_none()
            && let Some(worktree) = worktree
        {
            run.workspace = Some(worktree);
        }

        // Objective gate: a stage that reports success still has to pass
        // the project's own check before it counts.
        let check = (status == TurnStatus::Completed)
            .then(|| {
                self.workflow
                    .store
                    .get(&run_id)
                    .and_then(|run| run.node(&node_id))
                    .and_then(|node| node.check_command.clone())
            })
            .flatten();
        if let Some(command) = check {
            let Some(cwd) = cwd else {
                self.finish_workflow_node(
                    &run_id,
                    &node_id,
                    session_id,
                    wf::NodeOutcome::Failed {
                        reason: tr!("workflow.check_no_workspace"),
                    },
                    false,
                    cx,
                );
                return;
            };
            self.workflow
                .checks_in_flight
                .insert((run_id.clone(), node_id.clone()));
            cx.notify();
            cx.spawn(async move |this, cx| {
                let outcome = cx
                    .background_executor()
                    .spawn(async move { workflow_check::run_check(&command, &cwd, CHECK_TIMEOUT) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.workflow
                        .checks_in_flight
                        .remove(&(run_id.clone(), node_id.clone()));
                    let node_outcome = if outcome.passed {
                        wf::NodeOutcome::Done {
                            summary,
                            numstat: None,
                        }
                    } else {
                        let headline = if outcome.timed_out {
                            tr!("workflow.check_timed_out")
                        } else {
                            tr!("workflow.check_failed")
                        };
                        wf::NodeOutcome::Failed {
                            reason: format!("{headline}\n{}", outcome.output_tail),
                        }
                    };
                    this.finish_workflow_node(
                        &run_id,
                        &node_id,
                        session_id,
                        node_outcome,
                        false,
                        cx,
                    );
                });
            })
            .detach();
            return;
        }

        let outcome = match status {
            TurnStatus::Completed => wf::NodeOutcome::Done {
                summary,
                numstat: None,
            },
            TurnStatus::Failed => wf::NodeOutcome::Failed {
                reason: summary.unwrap_or_else(|| tr!("workflow.turn_failed")),
            },
            TurnStatus::Interrupted | TurnStatus::Running => wf::NodeOutcome::Canceled,
        };
        let stopped = matches!(outcome, wf::NodeOutcome::Canceled);
        self.finish_workflow_node(&run_id, &node_id, session_id, outcome, stopped, cx);
    }

    /// Record a node's outcome, drop its session mapping, gather what it
    /// changed, and move the run on.
    fn finish_workflow_node(
        &mut self,
        run_id: &str,
        node_id: &str,
        session_id: Uuid,
        outcome: wf::NodeOutcome,
        stopped: bool,
        cx: &mut Context<Self>,
    ) {
        let done = matches!(outcome, wf::NodeOutcome::Done { .. });
        let now = wf::now_ms();
        let mut keep_going = false;
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            run.settle_node(node_id, outcome, now);
            run.push_event(
                wf::EventKind::NodeSettled,
                Some(node_id),
                if stopped {
                    "stopped"
                } else if done {
                    "done"
                } else {
                    "failed"
                },
            );
            if stopped && run.status == RunStatus::Running {
                // The user stopped a stage by hand; do not start the next.
                run.status = RunStatus::Paused;
                run.push_event(wf::EventKind::Paused, Some(node_id), "stopped by user");
            }
            keep_going = run.status == RunStatus::Running;
        }
        self.workflow.node_of_session.remove(&session_id);
        if done {
            self.collect_workflow_numstat(run_id, node_id, session_id, cx);
        }
        if stopped {
            self.show_toast(tr!("workflow.stopped_by_user"));
        }
        if keep_going {
            self.pump_workflow_run(run_id, cx);
        } else {
            self.save_workflows(cx);
            cx.notify();
        }
    }

    /// What the stage's turn changed, as `git diff --numstat` text — the
    /// cheap "changed files" input the planner and the inspector read.
    fn collect_workflow_numstat(
        &mut self,
        run_id: &str,
        node_id: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let Some((cwd, turn_id, turn_count)) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| {
                let turn = session.turns.last()?;
                let cwd = self.workspace_path_for_session(session)?.to_path_buf();
                Some((cwd, turn.id, turn.turn_count))
            })
        else {
            return;
        };
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        let run_id = run_id.to_owned();
        let node_id = node_id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match workspace.request(waku_client::WorkspaceOperation::CollectReviewDiff {
                        cwd,
                        source: waku_client::workspace::ReviewDiffSource::LastTurn {
                            session_id,
                            turn_id,
                            turn_count,
                        },
                    })? {
                        waku_client::WorkspaceResult::ReviewDiff { data } => {
                            anyhow::Ok(data.numstat)
                        }
                        _ => anyhow::bail!("the daemon returned an invalid diff response"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Ok(numstat) = result else {
                    return;
                };
                if numstat.trim().is_empty() {
                    return;
                }
                let mut changed = false;
                if let Some(run) = this.workflow.store.get_mut(&run_id)
                    && let Some(node) = run.node_mut(&node_id)
                {
                    node.numstat = Some(numstat);
                    changed = true;
                }
                if changed {
                    this.save_workflows(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Run a settled node again, and everything downstream of it.
    pub(super) fn retry_workflow_node(
        &mut self,
        run_id: &str,
        node_id: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            if run.node(node_id).is_some_and(|node| node.status.is_live()) {
                return;
            }
            run.reset_from(node_id);
            run.status = RunStatus::Running;
            run.push_event(wf::EventKind::Resumed, Some(node_id), "retry");
        }
        self.pump_workflow_run(run_id, cx);
    }

    /// Treat a node as done without running it, so the stages after it
    /// can go ahead.
    pub(super) fn skip_workflow_node(
        &mut self,
        run_id: &str,
        node_id: &str,
        cx: &mut Context<Self>,
    ) {
        let now = wf::now_ms();
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            if run.node(node_id).is_none_or(|node| node.status.is_live()) {
                return;
            }
            // Un-skip whatever was skipped because of this node first.
            run.reset_from(node_id);
            run.settle_node(
                node_id,
                wf::NodeOutcome::Done {
                    summary: Some(tr!("workflow.skipped_by_user")),
                    numstat: None,
                },
                now,
            );
            run.push_event(wf::EventKind::NodeSettled, Some(node_id), "skipped by user");
            run.status = RunStatus::Running;
        }
        self.pump_workflow_run(run_id, cx);
    }

    /// The small "workflow · role" mark on a sidebar row whose session is a
    /// workflow stage. Click opens the run.
    pub(super) fn render_workflow_task_chip(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let session_key = session.id.to_string();
        let (run_id, title, role) = self.workflow.store.runs.iter().find_map(|run| {
            run.nodes
                .iter()
                .find(|node| node.session_id.as_deref() == Some(session_key.as_str()))
                .map(|node| (run.id.clone(), run.title.clone(), node.role.clone()))
        })?;
        let theme = Theme::current(cx);
        Some(
            div()
                .id(SharedString::from(format!("task-workflow-{}", session.id)))
                .flex_none()
                .h(px(16.0))
                .max_w(px(96.0))
                .px(px(5.0))
                .rounded(px(4.0))
                .flex()
                .items_center()
                .gap(px(3.0))
                .cursor_default()
                .bg(theme.overlay)
                .hover(|style| style.bg(theme.overlay_strong))
                .tooltip(Tooltip::text(SharedString::from(tr!(
                    "workflow.task_chip_tooltip",
                    title = title
                ))))
                .child(icon("icons/fork.svg", 10.0, theme.text_tertiary))
                .child(
                    div()
                        .min_w_0()
                        .text_size(sp(10.5))
                        .text_color(theme.text_tertiary)
                        .truncate()
                        .child(role),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_settings_page(SettingsPage::Workflow, cx);
                    this.open_workflow_run(&run_id, cx);
                })),
        )
    }

    /// Leave the settings page and select a stage's session.
    pub(super) fn open_workflow_node_session(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        self.settings_page = None;
        self.select_session(session_id, cx);
        cx.notify();
    }

    // --- canvas interaction --------------------------------------------

    pub(super) fn select_workflow_node(&mut self, node_id: Option<String>, cx: &mut Context<Self>) {
        self.workflow.selected_node = node_id;
        cx.notify();
    }

    /// A press on empty canvas: start panning and clear the selection.
    fn workflow_canvas_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        self.workflow.canvas_drag = Some(CanvasDrag {
            start_mouse: (f32::from(event.position.x), f32::from(event.position.y)),
            start_pan: self.workflow.canvas_pan,
            node: None,
            node_start: (0.0, 0.0),
            moved: false,
        });
        self.workflow.selected_node = None;
        cx.notify();
    }

    /// A press on a node: select it and start moving it.
    fn workflow_node_mouse_down(
        &mut self,
        node_id: &str,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        cx.stop_propagation();
        let node_start = self
            .workflow
            .current_run_id()
            .and_then(|run_id| self.workflow.store.get(run_id))
            .and_then(|run| run.node(node_id))
            .map(|node| (node.x, node.y))
            .unwrap_or_default();
        self.workflow.canvas_drag = Some(CanvasDrag {
            start_mouse: (f32::from(event.position.x), f32::from(event.position.y)),
            start_pan: self.workflow.canvas_pan,
            node: Some(node_id.to_owned()),
            node_start,
            moved: false,
        });
        self.workflow.selected_node = Some(node_id.to_owned());
        cx.notify();
    }

    fn workflow_canvas_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.workflow.canvas_drag.clone() else {
            return;
        };
        let dx = f32::from(event.position.x) - drag.start_mouse.0;
        let dy = f32::from(event.position.y) - drag.start_mouse.1;
        match &drag.node {
            None => self.workflow.canvas_pan = (drag.start_pan.0 + dx, drag.start_pan.1 + dy),
            Some(node_id) => {
                if let Some(run_id) = self.workflow.current_run_id().map(str::to_owned)
                    && let Some(run) = self.workflow.store.get_mut(&run_id)
                    && let Some(node) = run.node_mut(node_id)
                {
                    node.x = drag.node_start.0 + dx;
                    node.y = drag.node_start.1 + dy;
                }
            }
        }
        if (dx.abs() > DRAG_THRESHOLD || dy.abs() > DRAG_THRESHOLD)
            && let Some(drag) = self.workflow.canvas_drag.as_mut()
        {
            drag.moved = true;
        }
        cx.notify();
    }

    fn workflow_canvas_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let Some(drag) = self.workflow.canvas_drag.take() else {
            return;
        };
        // A node the user moved stays where they put it through re-layouts.
        if drag.moved
            && let Some(node_id) = drag.node
            && let Some(run_id) = self.workflow.current_run_id().map(str::to_owned)
            && let Some(run) = self.workflow.store.get_mut(&run_id)
            && let Some(node) = run.node_mut(&node_id)
        {
            node.pinned = true;
            self.save_workflows(cx);
        }
        cx.notify();
    }

    fn workflow_canvas_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(20.0));
        self.workflow.canvas_pan.0 += f32::from(delta.x);
        self.workflow.canvas_pan.1 += f32::from(delta.y);
        cx.notify();
    }

    /// Forget manual positions and lay the graph out again.
    pub(super) fn relayout_workflow_run(&mut self, run_id: &str, cx: &mut Context<Self>) {
        if let Some(run) = self.workflow.store.get_mut(run_id) {
            for node in &mut run.nodes {
                node.pinned = false;
            }
            wf::layout(run, &wf::LayoutOptions::default());
        }
        self.workflow.canvas_pan = CANVAS_MARGIN;
        self.save_workflows(cx);
        cx.notify();
    }

    pub(super) fn reset_workflow_view(&mut self, cx: &mut Context<Self>) {
        self.workflow.canvas_pan = CANVAS_MARGIN;
        cx.notify();
    }

    // --- rendering -----------------------------------------------------

    pub(super) fn render_workflow_settings(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.workflow.load_scheduled.replace(true) {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| this.load_workflows_if_needed(cx));
            })
            .detach();
        }
        match self.workflow.view.clone() {
            WorkflowView::List => self.render_workflow_list(cx),
            WorkflowView::New => self.render_workflow_form(cx),
            WorkflowView::Detail(id) => self.render_workflow_detail(&id, cx),
        }
    }

    fn render_workflow_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let now = wf::now_ms();
        let mut page = div().mt(px(15.0)).w_full().flex().flex_col().gap(px(12.0));
        if let Some(error) = &self.workflow.error {
            page = page.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }

        page = page.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("workflow.description")),
                )
                .child(card_button(
                    theme,
                    SharedString::from("workflow-new"),
                    tr!("workflow.new_run"),
                    true,
                    false,
                    cx,
                    |this, window, cx| this.open_workflow_form(window, cx),
                )),
        );

        let runs = self.workflow.store.sorted();
        if runs.is_empty() {
            return page
                .child(
                    div()
                        .w_full()
                        .px(px(20.0))
                        .py(px(28.0))
                        .rounded(px(13.0))
                        .bg(theme.raised)
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon("icons/fork.svg", 22.0, theme.text_tertiary))
                        .child(
                            div()
                                .text_size(sp(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(tr!("workflow.empty_title")),
                        )
                        .child(
                            div()
                                .text_size(sp(12.5))
                                .text_color(theme.text_secondary)
                                .child(tr!("workflow.empty_detail")),
                        ),
                )
                .into_any_element();
        }

        for run in runs {
            let id = run.id.clone();
            let (status_icon, status_color) = run_status_style(&theme, run.status);
            let done = run
                .nodes
                .iter()
                .filter(|node| node.status == NodeStatus::Done)
                .count();
            let chain = run
                .nodes
                .iter()
                .map(|node| {
                    let agent = provider_from_id(&node.provider_id)
                        .map(ProviderKind::short_name)
                        .unwrap_or(node.provider_id.as_str());
                    format!("{agent} {}", node.role)
                })
                .collect::<Vec<_>>()
                .join(" → ");
            let line = run.budget_line(now);
            let open_id = id.clone();
            page = page.child(
                div()
                    .id(SharedString::from(format!("workflow-run-{id}")))
                    .tab_index(0)
                    .focus_visible(|style| style.border_color(theme.accent))
                    .w_full()
                    .px(px(20.0))
                    .py(px(14.0))
                    .rounded(px(13.0))
                    .bg(theme.raised)
                    .border_1()
                    .border_color(theme.raised)
                    .cursor_default()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(status_chip(
                                status_icon,
                                run_status_label(run.status),
                                status_color,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(sp(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .truncate()
                                    .child(run.title.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_tertiary)
                            .child(tr!(
                                "workflow.nodes_line",
                                done = done,
                                total = run.nodes.len(),
                                elapsed = format_elapsed(line.elapsed_ms)
                            )),
                    )
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_secondary)
                            .truncate()
                            .child(chain),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_workflow_run(&open_id, cx);
                    })),
            );
        }
        page.into_any_element()
    }

    fn render_workflow_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let form = &self.workflow.form;
        let mut page = div().mt(px(15.0)).w_full().flex().flex_col().gap(px(14.0));

        // Header: back + title.
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(card_button(
                    theme,
                    SharedString::from("workflow-form-back"),
                    tr!("workflow.back"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.back_to_workflow_list(cx),
                ))
                .child(
                    div()
                        .text_size(sp(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(tr!("workflow.new_run")),
                ),
        );

        // 1. Goal + project.
        let mut goal_card = self.workflow_card(theme, tr!("workflow.goal_label"));
        if let Some(input) = &self.workflow.goal_input {
            goal_card = goal_card.child(
                div()
                    .w_full()
                    .min_h(px(72.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.inset)
                    .text_size(sp(12.5))
                    .child(input.clone()),
            );
        }
        let project_label = form
            .project_id
            .and_then(|id| self.state.projects.iter().find(|project| project.id == id))
            .map(|project| project.name.clone())
            .unwrap_or_else(|| tr!("workflow.no_project"));
        let projects: Vec<(Uuid, String)> = self
            .state
            .projects
            .iter()
            .map(|project| (project.id, project.name.clone()))
            .collect();
        let weak = cx.entity().downgrade();
        let project_handle = self.menu_handle("workflow-project-menu", cx);
        let project_trigger = card_button(
            theme,
            SharedString::from("workflow-project-trigger"),
            format!("{}: {project_label}", tr!("workflow.project_label")),
            false,
            projects.is_empty(),
            cx,
            |_, _, _| {},
        );
        let project_menu = dropdown_menu(
            project_trigger,
            "workflow-project-menu",
            &project_handle,
            MenuAlign::BelowLeft,
            move |_| {
                projects
                    .iter()
                    .map(|(id, name)| {
                        let weak = weak.clone();
                        let id = *id;
                        MenuItem::new(name.clone(), move |_, cx| {
                            let _ = weak.update(cx, |this, cx| this.set_workflow_project(id, cx));
                        })
                    })
                    .collect()
            },
        );
        goal_card = goal_card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(project_menu)
                .when(self.selected_session().is_some(), |row| {
                    row.child(card_button(
                        theme,
                        SharedString::from("workflow-seed-goal"),
                        tr!("workflow.seed_from_session"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.seed_workflow_goal_from_session(cx),
                    ))
                }),
        );
        page = page.child(goal_card);

        // 2. Template chips.
        let mut template_row = div().flex().flex_wrap().gap(px(6.0));
        let custom_selected = form.template_id.is_none();
        template_row = template_row.child(self.workflow_chip(
            theme,
            "workflow-template-custom",
            tr!("workflow.template_custom"),
            custom_selected,
            cx,
            |this, _, cx| this.set_workflow_template(None, cx),
        ));
        for template in wf::builtin_templates() {
            let selected = form.template_id.as_deref() == Some(template.id.as_str());
            let id = template.id.clone();
            template_row = template_row.child(self.workflow_chip(
                theme,
                SharedString::from(format!("workflow-template-{}", template.id)),
                template.name.clone(),
                selected,
                cx,
                move |this, _, cx| this.set_workflow_template(Some(id.clone()), cx),
            ));
        }
        page = page.child(
            self.workflow_card(theme, tr!("workflow.template_label"))
                .child(template_row),
        );

        // 3. Roles table.
        let available = self.workflow_available_providers();
        let mut roles_card = self.workflow_card(theme, tr!("workflow.roles_label"));
        if available.is_empty() {
            roles_card = roles_card.child(
                div()
                    .text_size(sp(12.5))
                    .text_color(theme.warning)
                    .child(tr!("workflow.no_agents")),
            );
        }
        for (index, row) in form.roles.iter().enumerate() {
            roles_card =
                roles_card.child(self.render_workflow_role_row(index, row, &available, theme, cx));
        }
        if custom_selected {
            roles_card = roles_card.child(card_button(
                theme,
                SharedString::from("workflow-add-role"),
                tr!("workflow.add_role"),
                false,
                available.is_empty(),
                cx,
                |this, _, cx| this.add_workflow_custom_role(cx),
            ));
        }
        page = page.child(roles_card);

        // 3b. Objective check.
        if let Some(input) = &self.workflow.check_input {
            page = page.child(
                self.workflow_card(theme, tr!("workflow.check_label"))
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.text_tertiary)
                            .child(tr!("workflow.check_detail")),
                    )
                    .child(
                        TextField::new("workflow-check-input", input.clone())
                            .icon("icons/terminal.svg", 12.0),
                    ),
            );
        }

        // 4. Planner toggle + create.
        let mut footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(toggle_switch(
                        "workflow-planner-toggle",
                        form.planner_enabled,
                        false,
                        theme,
                        cx,
                        |this, _, cx| this.toggle_workflow_planner(cx),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(sp(12.5))
                                    .text_color(theme.text)
                                    .child(tr!("workflow.use_planner")),
                            )
                            .child(
                                div()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child(tr!("workflow.use_planner_detail")),
                            ),
                    ),
            )
            .child(card_button(
                theme,
                SharedString::from("workflow-create"),
                tr!("workflow.create"),
                true,
                available.is_empty(),
                cx,
                |this, _, cx| this.create_workflow_run(cx),
            ));
        if let Some(error) = &form.error {
            footer = footer.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        page = page.child(footer);
        page.into_any_element()
    }

    fn render_workflow_role_row(
        &self,
        index: usize,
        row: &RoleRow,
        available: &[ProviderKind],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let weak = cx.entity().downgrade();

        // Agent picker: every CLI, the uninstalled ones greyed with a hint.
        let agent_handle = self.menu_handle(format!("workflow-role-agent-{index}"), cx);
        let installed: Vec<ProviderKind> = available.to_vec();
        let agent_trigger = card_button(
            theme,
            SharedString::from(format!("workflow-role-agent-trigger-{index}")),
            row.provider.display_name().to_owned(),
            false,
            false,
            cx,
            |_, _, _| {},
        );
        let current_provider = row.provider;
        let agent_weak = weak.clone();
        let agent_menu = dropdown_menu(
            agent_trigger,
            format!("workflow-role-agent-{index}"),
            &agent_handle,
            MenuAlign::BelowLeft,
            move |_| {
                ProviderKind::ALL
                    .into_iter()
                    .map(|kind| {
                        let weak = agent_weak.clone();
                        let usable = installed.contains(&kind);
                        let label = if usable {
                            kind.display_name().to_owned()
                        } else {
                            format!(
                                "{} · {}",
                                kind.display_name(),
                                tr!("workflow.agent_not_installed")
                            )
                        };
                        MenuItem::new(label, move |_, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.set_workflow_role_provider(index, kind, cx)
                            });
                        })
                        .icon(provider_icon(kind))
                        .selected(kind == current_provider)
                        .disabled(!usable)
                    })
                    .collect()
            },
        );

        // Model picker: the provider's discovered catalog, default first.
        let models: Vec<(String, String)> = self
            .provider_probe(row.provider)
            .map(|probe| {
                probe
                    .models
                    .iter()
                    .map(|model| (model.id.clone(), model.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let model_label = row
            .model
            .as_ref()
            .and_then(|id| models.iter().find(|(model_id, _)| model_id == id))
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| tr!("workflow.role_model_default"));
        let model_handle = self.menu_handle(format!("workflow-role-model-{index}"), cx);
        let model_trigger = card_button(
            theme,
            SharedString::from(format!("workflow-role-model-trigger-{index}")),
            model_label,
            false,
            models.is_empty(),
            cx,
            |_, _, _| {},
        );
        let current_model = row.model.clone();
        let model_weak = weak.clone();
        let model_menu = dropdown_menu(
            model_trigger,
            format!("workflow-role-model-{index}"),
            &model_handle,
            MenuAlign::BelowLeft,
            move |_| {
                let mut items = Vec::with_capacity(models.len() + 1);
                let default_weak = model_weak.clone();
                items.push(
                    MenuItem::new(tr!("workflow.role_model_default"), move |_, cx| {
                        let _ = default_weak
                            .update(cx, |this, cx| this.set_workflow_role_model(index, None, cx));
                    })
                    .selected(current_model.is_none()),
                );
                for (id, name) in &models {
                    let weak = model_weak.clone();
                    let selected = current_model.as_deref() == Some(id.as_str());
                    let pick = id.clone();
                    items.push(
                        MenuItem::new(name.clone(), move |_, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.set_workflow_role_model(index, Some(pick.clone()), cx)
                            });
                        })
                        .selected(selected),
                    );
                }
                items
            },
        );

        let mode_label = if row.plan_mode {
            tr!("workflow.mode_plan")
        } else {
            tr!("workflow.mode_build")
        };

        div()
            .w_full()
            .py(px(8.0))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .w(px(120.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(icon(
                        provider_icon(row.provider),
                        13.0,
                        provider_color(&theme, row.provider),
                    ))
                    .child(
                        div()
                            .text_size(sp(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .truncate()
                            .child(row.role.clone()),
                    ),
            )
            .child(agent_menu)
            .child(model_menu)
            .child(card_button(
                theme,
                SharedString::from(format!("workflow-role-mode-{index}")),
                mode_label,
                false,
                false,
                cx,
                move |this, _, cx| this.toggle_workflow_role_plan(index, cx),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_tertiary)
                            .child(tr!("workflow.role_writes")),
                    )
                    .child(toggle_switch(
                        SharedString::from(format!("workflow-role-writes-{index}")),
                        row.writes,
                        false,
                        theme,
                        cx,
                        move |this, _, cx| this.toggle_workflow_role_writes(index, cx),
                    )),
            )
            .when(
                self.workflow.form.template_id.is_none() && self.workflow.form.roles.len() > 1,
                |row| {
                    row.child(
                        icon_button(
                            SharedString::from(format!("workflow-role-remove-{index}")),
                            "icons/x.svg",
                            theme,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove_workflow_role(index, cx)),
                        ),
                    )
                },
            )
    }

    fn render_workflow_detail(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(run) = self.workflow.store.get(id) else {
            return div()
                .mt(px(15.0))
                .child(card_button(
                    theme,
                    SharedString::from("workflow-detail-back-missing"),
                    tr!("workflow.back"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.back_to_workflow_list(cx),
                ))
                .into_any_element();
        };
        let now = wf::now_ms();
        let (status_icon, status_color) = run_status_style(&theme, run.status);
        let line = run.budget_line(now);
        let run_id = run.id.clone();

        let mut actions = div().flex().items_center().gap(px(8.0));
        match run.status {
            RunStatus::Draft | RunStatus::Paused => {
                let start_id = run_id.clone();
                actions = actions.child(card_button(
                    theme,
                    SharedString::from("workflow-detail-start"),
                    if run.status == RunStatus::Draft {
                        tr!("workflow.start")
                    } else {
                        tr!("workflow.resume")
                    },
                    true,
                    false,
                    cx,
                    move |this, _, cx| this.start_workflow_run(&start_id, cx),
                ));
            }
            RunStatus::Running => {
                let pause_id = run_id.clone();
                actions = actions.child(card_button(
                    theme,
                    SharedString::from("workflow-detail-pause"),
                    tr!("workflow.pause"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.pause_workflow_run(&pause_id, cx),
                ));
            }
            _ => {}
        }
        if run.planner_enabled {
            if self.workflow.planner_busy || run.status == RunStatus::Planning {
                actions = actions.child(status_chip(
                    "icons/sparkle.svg",
                    tr!("workflow.planning"),
                    theme.accent,
                ));
            } else if run.status != RunStatus::AwaitingApproval {
                let plan_id = run_id.clone();
                actions = actions.child(card_button(
                    theme,
                    SharedString::from("workflow-detail-replan"),
                    tr!("workflow.replan"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.request_workflow_plan(&plan_id, cx),
                ));
            }
        }
        let relayout_id = run_id.clone();
        actions = actions.child(card_button(
            theme,
            SharedString::from("workflow-detail-relayout"),
            tr!("workflow.relayout"),
            false,
            false,
            cx,
            move |this, _, cx| this.relayout_workflow_run(&relayout_id, cx),
        ));
        let delete_id = run_id.clone();
        actions = actions.child(card_button(
            theme,
            SharedString::from("workflow-detail-delete"),
            tr!("workflow.delete_run"),
            false,
            false,
            cx,
            move |this, _, cx| this.confirm_delete_workflow_run(delete_id.clone(), cx),
        ));

        let header = div()
            .flex_none()
            .pt(px(15.0))
            .pb(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(card_button(
                        theme,
                        SharedString::from("workflow-detail-back"),
                        tr!("workflow.back"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.back_to_workflow_list(cx),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .truncate()
                            .child(run.title.clone()),
                    )
                    .child(status_chip(
                        status_icon,
                        run_status_label(run.status),
                        status_color,
                    ))
                    .child(actions),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_tertiary)
                            .child(tr!(
                                "workflow.budget_line",
                                nodes = line.nodes_run,
                                max_nodes = line.max_nodes,
                                calls = line.planner_calls,
                                max_calls = line.max_planner_calls,
                                elapsed = format_elapsed(line.elapsed_ms),
                                max_elapsed = format_elapsed(line.max_wall_ms)
                            )),
                    )
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.text_ghost)
                            .child(tr!("workflow.canvas_hint")),
                    ),
            );

        let selected = self
            .workflow
            .selected_node
            .as_deref()
            .and_then(|node_id| run.node(node_id));
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .gap(px(10.0))
            .pb(px(12.0))
            .child(self.render_workflow_canvas(run, theme, now, cx))
            .when_some(selected, |body, node| {
                body.child(self.render_workflow_inspector(run, node, theme, now, cx))
            });

        // Proposed stages wait for the user before anything runs.
        let proposed = run
            .nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Proposed)
            .count();
        let gate = (proposed > 0).then(|| {
            let approve_id = run_id.clone();
            let discard_id = run_id.clone();
            div()
                .flex_none()
                .mb(px(10.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(px(10.0))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.accent)
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(icon("icons/sparkle.svg", 13.0, theme.accent))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(sp(12.5))
                        .text_color(theme.text)
                        .child(tr!("workflow.gate_title", count = proposed)),
                )
                .child(card_button(
                    theme,
                    SharedString::from("workflow-gate-approve"),
                    tr!("workflow.gate_approve"),
                    true,
                    false,
                    cx,
                    move |this, _, cx| this.approve_workflow_plan(&approve_id, cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from("workflow-gate-discard"),
                    tr!("workflow.gate_discard"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.discard_workflow_plan(&discard_id, cx),
                ))
        });

        div()
            .w_full()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .children(gate)
            .child(body)
            .into_any_element()
    }

    /// The graph: one canvas layer for the edges, positioned `div`s for
    /// the nodes, and a reset button. Every coordinate carries the pan.
    fn render_workflow_canvas(
        &self,
        run: &WorkflowRun,
        theme: Theme,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let pan = self.workflow.canvas_pan;
        let options = wf::LayoutOptions::default();

        // Snapshot the edges for the paint closure, which must own its data.
        let edges: Vec<(wf::EdgeCurve, bool, bool)> = run
            .edges
            .iter()
            .filter_map(|edge| {
                let from = run.node(&edge.from)?;
                let to = run.node(&edge.to)?;
                let satisfied = match edge.kind {
                    wf::EdgeKind::DependsOn => from.status == NodeStatus::Done,
                    wf::EdgeKind::OnFailure => from.status == NodeStatus::Failed,
                };
                Some((
                    wf::edge_curve(from, to, &options),
                    satisfied,
                    edge.kind == wf::EdgeKind::OnFailure,
                ))
            })
            .collect();
        let (edge_color, waiting_color, failure_color) =
            (theme.border_strong, theme.text_ghost, theme.danger);
        let edge_layer = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let ox = f32::from(bounds.origin.x) + pan.0;
                let oy = f32::from(bounds.origin.y) + pan.1;
                let at = |p: (f32, f32)| point(px(ox + p.0), px(oy + p.1));
                for (curve, satisfied, on_failure) in &edges {
                    let color = if *on_failure {
                        failure_color
                    } else if *satisfied {
                        edge_color
                    } else {
                        waiting_color
                    };
                    let mut line = PathBuilder::stroke(px(1.5));
                    if !*satisfied {
                        line = line.dash_array(&[px(4.0), px(4.0)]);
                    }
                    line.move_to(at(curve.start));
                    line.cubic_bezier_to(at(curve.end), at(curve.c1), at(curve.c2));
                    if let Ok(path) = line.build() {
                        window.paint_path(path, color);
                    }
                    let head = wf::arrow_head(curve, 8.0);
                    let mut tip = PathBuilder::fill();
                    tip.move_to(at(head[0]));
                    tip.line_to(at(head[1]));
                    tip.line_to(at(head[2]));
                    tip.close();
                    if let Ok(path) = tip.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size_full();

        let mut area =
            div()
                .id("workflow-canvas")
                .flex_1()
                .min_w_0()
                .h_full()
                .relative()
                .overflow_hidden()
                .rounded(px(13.0))
                .bg(theme.inset)
                .border_1()
                .border_color(theme.border)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event, window, cx| {
                        this.workflow_canvas_mouse_down(event, window, cx)
                    }),
                )
                .on_mouse_move(cx.listener(|this, event, window, cx| {
                    this.workflow_canvas_mouse_move(event, window, cx)
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event, window, cx| {
                        this.workflow_canvas_mouse_up(event, window, cx)
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                    this.workflow_canvas_scroll(event, window, cx)
                }))
                .child(edge_layer);

        for node in &run.nodes {
            area = area.child(self.render_workflow_node(node, pan, &options, theme, now, cx));
        }

        area.child(
            div()
                .absolute()
                .bottom(px(10.0))
                .right(px(10.0))
                .child(card_button(
                    theme,
                    SharedString::from("workflow-canvas-reset"),
                    tr!("workflow.reset_view"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.reset_workflow_view(cx),
                )),
        )
    }

    fn render_workflow_node(
        &self,
        node: &wf::Node,
        pan: (f32, f32),
        options: &wf::LayoutOptions,
        theme: Theme,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (status_icon, status_color) = node_status_style(&theme, node.status);
        let provider = provider_from_id(&node.provider_id);
        let selected = self.workflow.selected_node.as_deref() == Some(node.id.as_str());
        let border_color = if selected {
            theme.accent
        } else if node.status.is_live() {
            status_color
        } else {
            theme.border_strong
        };
        let elapsed = node.elapsed_ms(now).map(format_elapsed);
        let detail = format!(
            "{}{}",
            node.model.as_deref().unwrap_or(""),
            if node.plan_mode {
                format!(
                    "{}{}",
                    if node.model.is_some() { " · " } else { "" },
                    tr!("workflow.mode_plan")
                )
            } else {
                String::new()
            }
        );
        let press_id = node.id.clone();
        let activate_id = node.id.clone();
        let session_id = node
            .session_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok());

        div()
            .id(SharedString::from(format!("workflow-node-{}", node.id)))
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .absolute()
            .left(px(node.x + pan.0))
            .top(px(node.y + pan.1))
            .w(px(options.node_w))
            .h(px(options.node_h))
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .bg(theme.raised)
            .border_1()
            .border_color(border_color)
            .when(node.status == NodeStatus::Proposed, |card| {
                card.border_dashed()
            })
            .cursor_default()
            .flex()
            .flex_col()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .truncate()
                            .child(node.role.clone()),
                    )
                    .when_some(provider, |row, provider| {
                        row.child(icon(
                            provider_icon(provider),
                            13.0,
                            provider_color(&theme, provider),
                        ))
                    }),
            )
            .child(
                div()
                    .text_size(sp(11.0))
                    .text_color(theme.text_tertiary)
                    .truncate()
                    .child(if detail.is_empty() {
                        provider
                            .map(|kind| kind.short_name().to_owned())
                            .unwrap_or_default()
                    } else {
                        detail
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(status_chip(
                        status_icon,
                        node_status_label(node.status),
                        status_color,
                    ))
                    .when_some(elapsed, |row, elapsed| {
                        row.child(
                            div()
                                .text_size(sp(11.0))
                                .text_color(theme.text_tertiary)
                                .child(elapsed),
                        )
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| {
                    this.workflow_node_mouse_down(&press_id, event, cx)
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2
                    && let Some(session_id) = session_id
                {
                    this.open_workflow_node_session(session_id, cx);
                }
            }))
            .on_activation(cx, move |this, _, cx| {
                this.select_workflow_node(Some(activate_id.clone()), cx)
            })
    }

    /// The side column for the selected node.
    fn render_workflow_inspector(
        &self,
        run: &WorkflowRun,
        node: &wf::Node,
        theme: Theme,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (status_icon, status_color) = node_status_style(&theme, node.status);
        let provider = provider_from_id(&node.provider_id);
        let session_id = node
            .session_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok());
        let deps = run
            .dependencies_of(&node.id)
            .into_iter()
            .map(|edge| {
                let name = run
                    .node(&edge.from)
                    .map(|n| n.role.clone())
                    .unwrap_or_default();
                match edge.kind {
                    wf::EdgeKind::DependsOn => tr!("workflow.after_node", name = name),
                    wf::EdgeKind::OnFailure => tr!("workflow.on_failure_of", name = name),
                }
            })
            .collect::<Vec<_>>();
        let config = format!(
            "{} · {}{} · {}",
            provider
                .map(ProviderKind::display_name)
                .unwrap_or(&node.provider_id),
            node.model
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| tr!("workflow.role_model_default")),
            if node.plan_mode {
                format!(" · {}", tr!("workflow.mode_plan"))
            } else {
                format!(" · {}", tr!("workflow.mode_build"))
            },
            if node.writes {
                tr!("workflow.role_writes")
            } else {
                String::new()
            }
        );
        let section = |title: String| {
            div()
                .text_size(sp(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_tertiary)
                .child(title)
        };
        let body_text = |text: String, color: gpui::Hsla| {
            div()
                .text_size(sp(12.0))
                .line_height(sp(17.0))
                .text_color(color)
                .child(text)
        };

        div()
            .id("workflow-inspector")
            .w(px(INSPECTOR_WIDTH))
            .flex_none()
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when_some(provider, |row, provider| {
                        row.child(icon(
                            provider_icon(provider),
                            14.0,
                            provider_color(&theme, provider),
                        ))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .truncate()
                            .child(node.role.clone()),
                    )
                    .child(
                        icon_button("workflow-inspector-close", "icons/x.svg", theme).on_click(
                            cx.listener(|this, _, _, cx| this.select_workflow_node(None, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(status_chip(
                        status_icon,
                        node_status_label(node.status),
                        status_color,
                    ))
                    .when_some(node.elapsed_ms(now), |row, elapsed| {
                        row.child(
                            div()
                                .text_size(sp(11.5))
                                .text_color(theme.text_tertiary)
                                .child(format_elapsed(elapsed)),
                        )
                    }),
            )
            .when_some(session_id, |column, session_id| {
                column.child(card_button(
                    theme,
                    SharedString::from("workflow-inspector-open"),
                    tr!("workflow.open_session"),
                    true,
                    false,
                    cx,
                    move |this, _, cx| this.open_workflow_node_session(session_id, cx),
                ))
            })
            .when(!node.status.is_live(), |column| {
                let retry_run = run.id.clone();
                let retry_node = node.id.clone();
                let skip_run = run.id.clone();
                let skip_node = node.id.clone();
                column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(card_button(
                            theme,
                            SharedString::from("workflow-inspector-retry"),
                            tr!("workflow.retry_node"),
                            false,
                            false,
                            cx,
                            move |this, _, cx| {
                                this.retry_workflow_node(&retry_run, &retry_node, cx)
                            },
                        ))
                        .when(node.status != NodeStatus::Done, |row| {
                            row.child(card_button(
                                theme,
                                SharedString::from("workflow-inspector-skip"),
                                tr!("workflow.skip_node"),
                                false,
                                false,
                                cx,
                                move |this, _, cx| {
                                    this.skip_workflow_node(&skip_run, &skip_node, cx)
                                },
                            ))
                        }),
                )
            })
            .child(section(tr!("workflow.inspector_config")))
            .child(body_text(config, theme.text_secondary))
            .when_some(node.numstat.clone(), |column, numstat| {
                column
                    .child(section(tr!("workflow.inspector_changes")))
                    .child(body_text(numstat, theme.text_secondary))
            })
            .when(!deps.is_empty(), |column| {
                column
                    .child(section(tr!("workflow.inspector_deps")))
                    .child(body_text(deps.join("\n"), theme.text_secondary))
            })
            .child(section(tr!("workflow.inspector_summary")))
            .child(body_text(
                node.summary
                    .clone()
                    .unwrap_or_else(|| tr!("workflow.no_summary_yet")),
                if node.summary.is_some() {
                    theme.text
                } else {
                    theme.text_ghost
                },
            ))
            .child(section(tr!("workflow.inspector_prompt")))
            .child(body_text(node.prompt.clone(), theme.text_secondary))
    }

    fn workflow_card(&self, theme: Theme, title: String) -> Div {
        div()
            .w_full()
            .px(px(20.0))
            .py(px(14.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(sp(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(title),
            )
    }

    fn workflow_chip(
        &self,
        theme: Theme,
        id: impl Into<SharedString>,
        label: String,
        selected: bool,
        cx: &mut Context<Self>,
        activate: impl Fn(&mut Waku, &mut Window, &mut Context<Waku>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id.into())
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(26.0))
            .px(px(10.0))
            .rounded_full()
            .border_1()
            .border_color(if selected {
                theme.inverse
            } else {
                theme.border_strong
            })
            .bg(if selected { theme.inverse } else { theme.inset })
            .flex()
            .items_center()
            .cursor_default()
            .text_size(sp(12.0))
            .text_color(if selected {
                theme.on_inverse
            } else {
                theme.text_secondary
            })
            .child(label)
            .on_activation(cx, activate)
    }
}
