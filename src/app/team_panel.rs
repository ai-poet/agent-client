//! The right panel's Team surface: the team the selected session leads.
//!
//! Fork addition (AgentTeams), in place of the former workflow graph. The
//! team's record lives in the workspace (`<cwd>/.agent-teams/`) and is the
//! truth; this surface reads it through the daemon — the same path the Files
//! panel uses, so it works wherever the session runs — together with the live
//! state the app already has (which members are working, from their
//! background-work records).
//!
//! Plan decisions go back as controls: a sentinel prompt the built-in agent
//! recognises and handles without a turn (`agent_teams::command`). Nothing
//! here writes the team's files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_teams::command::{Control, ControlAction, encode_control};
use agent_teams::key::{CAPTAIN_KEY, sanitize_key};
use agent_teams::snapshot::{
    DagDims, DagLayout, LiveFacts, MemberLiveActivity, MessagePreview, TeamSnapshot,
    VisualTaskState, compact_dag_layout, related_task_ids, uses_parallel_task_grid,
};
use agent_teams::types::{MemberStatus, TeamState};
use gpui::PathBuilder;

use super::providers_page::card_button;
use super::*;
use crate::ui::ActivationExt as _;

/// How often an open Team surface looks again while its team works.
const ACTIVE_POLL: Duration = Duration::from_secs(2);
/// …and while nothing runs.
const IDLE_POLL: Duration = Duration::from_secs(15);
/// The least time between two looks for a team in a session that has none.
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
/// How long the Stop button waits for its second press.
const STOP_CONFIRM_WINDOW: Duration = Duration::from_secs(4);

fn dag_dims() -> DagDims {
    DagDims::default()
}

/// One session's team, as last read.
pub(super) struct TeamView {
    pub snapshot: TeamSnapshot,
    pub layout: DagLayout,
}

#[derive(Default)]
pub(super) struct TeamPanelState {
    views: HashMap<Uuid, TeamView>,
    /// Sessions known to lead no team (as of the last look).
    none: HashSet<Uuid>,
    loading: HashSet<Uuid>,
    generation: HashMap<Uuid, u64>,
    last_look: HashMap<Uuid, Instant>,
    /// Sessions whose team should be read again at the next chance.
    pub stale: HashSet<Uuid>,
    /// Controls waiting for the session's runtime.
    pending_controls: HashMap<Uuid, Vec<String>>,
    /// Teams already shown once on their own, by `session/team`.
    auto_opened: HashSet<String>,
    stop_armed: Option<(Uuid, Instant)>,
    pinned_task: Option<String>,
    error: Option<String>,
    poll_running: bool,
}

/// Read the team a captain leads, or its newest archived one, through the
/// daemon. Blocking: runs on the background executor.
fn fetch_team(
    workspace: &waku_client::WorkspaceClient,
    cwd: &Path,
    state_dir: &str,
    captain: &str,
    live: &HashMap<String, MemberLiveActivity>,
) -> Result<Option<TeamSnapshot>, String> {
    let root = cwd.join(state_dir);
    let read = |relative: String| -> Option<String> {
        match workspace.request(waku_client::WorkspaceOperation::ReadTextFile {
            root: root.clone(),
            relative_path: PathBuf::from(relative),
        }) {
            Ok(waku_client::WorkspaceResult::TextFile { content }) => Some(content),
            _ => None,
        }
    };
    let directories = |path: PathBuf| -> Vec<String> {
        match workspace
            .request(waku_client::WorkspaceOperation::BrowseDirectory { path: Some(path) })
        {
            Ok(waku_client::WorkspaceResult::Directory { entries, .. }) => entries
                .into_iter()
                .filter(|entry| entry.is_dir && !entry.name.starts_with('.'))
                .map(|entry| entry.name)
                .collect(),
            _ => Vec::new(),
        }
    };
    let find = |prefix: &str, ids: Vec<String>| -> Option<TeamState> {
        ids.into_iter()
            .filter_map(|id| {
                let text = read(format!("{prefix}{id}/team.json"))?;
                agent_teams::validate::parse_team_state(&text, &id).ok()
            })
            .filter(|team| team.captain_session_id == captain)
            .max_by_key(|team| team.created_at)
    };
    let live_ids: Vec<String> = directories(root.clone())
        .into_iter()
        .filter(|name| name != agent_teams::store::ARCHIVE_DIR)
        .collect();
    let (team, archived, prefix) = match find("", live_ids) {
        Some(team) => (team, false, String::new()),
        None => match find(
            "archive/",
            directories(root.join(agent_teams::store::ARCHIVE_DIR)),
        ) {
            Some(team) => (team, true, "archive/".to_owned()),
            None => return Ok(None),
        },
    };
    let mailbox = |agent: &str| -> Vec<agent_teams::types::TeamMessage> {
        read(format!(
            "{prefix}{}/inbox/{}.jsonl",
            team.id,
            sanitize_key(agent)
        ))
        .map(|text| {
            text.lines()
                .filter_map(agent_teams::validate::parse_message_line)
                .collect()
        })
        .unwrap_or_default()
    };
    let mut facts = LiveFacts::default();
    let captain_mail = mailbox(CAPTAIN_KEY);
    facts.message_count += captain_mail.len();
    facts.captain_unread = captain_mail
        .iter()
        .filter(|message| message.is_unread())
        .map(|message| MessagePreview {
            from: message.from.clone(),
            content: agent_teams::mailbox::mailbox_content(message),
        })
        .collect();
    for member in &team.members {
        let mail = mailbox(&member.name);
        facts.message_count += mail.len();
        facts.unread_by_member.insert(
            member.name.clone(),
            mail.iter().filter(|message| message.is_unread()).count(),
        );
    }
    facts.activity_by_member_id = live.clone();
    Ok(Some(agent_teams::snapshot::assemble(
        &team, &facts, archived,
    )))
}

fn state_dir() -> String {
    sub2api::global_config::native::config_dir()
        .map(|dir| dir.join(agent_teams::config::CONFIG_FILE))
        .and_then(|path| agent_teams::config::load(&path).ok())
        .map(|config| config.state_dir().to_owned())
        .unwrap_or_else(|| agent_teams::store::DEFAULT_STATE_DIR.to_owned())
}

impl Waku {
    /// Whether `session_id` is the built-in agent's — the only kind that can
    /// lead a team.
    fn team_capable(&self, session_id: Uuid) -> bool {
        self.state
            .sessions
            .iter()
            .any(|session| session.id == session_id && session.provider == ProviderKind::Native)
    }

    /// The team the selected session leads, as last read.
    pub(super) fn selected_team_view(&self) -> Option<&TeamView> {
        self.team_panel.views.get(&self.state.selected_session?)
    }

    /// Whether the selected session leads a team (live or archived).
    pub(super) fn selected_session_has_team(&self) -> bool {
        self.selected_team_view().is_some()
    }

    /// Which of the session's members are working, from their records.
    fn team_member_activity(&self, session_id: Uuid) -> HashMap<String, MemberLiveActivity> {
        self.live_background_work(session_id)
            .into_iter()
            .filter(|item| {
                item.key.kind == BackgroundWorkKind::Subagent
                    && item.key.provider_id.contains("::team:")
            })
            .map(|item| (item.key.provider_id.clone(), MemberLiveActivity::Working))
            .collect()
    }

    /// Read the session's team again, off the UI thread.
    pub(super) fn refresh_team_view(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if !self.team_capable(session_id) || self.team_panel.loading.contains(&session_id) {
            return;
        }
        let Some(session) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        let Some(captain) = session.provider_native_id().map(str::to_owned) else {
            return;
        };
        let Some(cwd) = self
            .workspace_path_for_session(session)
            .map(Path::to_path_buf)
        else {
            return;
        };
        let live = self.team_member_activity(session_id);
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        let generation = {
            let entry = self.team_panel.generation.entry(session_id).or_default();
            *entry += 1;
            *entry
        };
        self.team_panel.loading.insert(session_id);
        self.team_panel.stale.remove(&session_id);
        self.team_panel.last_look.insert(session_id, Instant::now());
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move { fetch_team(&workspace, &cwd, &state_dir(), &captain, &live) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.team_panel;
                state.loading.remove(&session_id);
                if state.generation.get(&session_id) != Some(&generation) {
                    return;
                }
                match fetched {
                    Ok(Some(snapshot)) => {
                        let first_staged = snapshot.staged
                            && state
                                .auto_opened
                                .insert(format!("{session_id}/{}", snapshot.team_id));
                        let layout = compact_dag_layout(&snapshot.tasks, dag_dims());
                        state.none.remove(&session_id);
                        state
                            .views
                            .insert(session_id, TeamView { snapshot, layout });
                        state.error = None;
                        if first_staged && this.state.selected_session == Some(session_id) {
                            this.open_right_panel_surface(RightPanelSurface::Team, cx);
                        }
                    }
                    Ok(None) => {
                        state.views.remove(&session_id);
                        state.none.insert(session_id);
                    }
                    Err(error) => state.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The event pump's turn: send controls whose runtime is now up, and
    /// look again where something changed.
    pub(super) fn drain_team_panel(&mut self, cx: &mut Context<Self>) {
        let ready: Vec<Uuid> = self
            .team_panel
            .pending_controls
            .keys()
            .copied()
            .filter(|session_id| self.runtimes.contains_key(session_id))
            .collect();
        for session_id in ready {
            if let Some(controls) = self.team_panel.pending_controls.remove(&session_id)
                && let Some(runtime) = self.runtimes.get(&session_id)
            {
                for control in controls {
                    runtime.driver.prompt(control);
                }
                self.team_panel.stale.insert(session_id);
            }
        }
        let stale: Vec<Uuid> = self.team_panel.stale.iter().copied().collect();
        for session_id in stale {
            let known = self.team_panel.views.contains_key(&session_id);
            let recently = self
                .team_panel
                .last_look
                .get(&session_id)
                .is_some_and(|at| at.elapsed() < DISCOVERY_INTERVAL);
            if known || !recently {
                self.refresh_team_view(session_id, cx);
            } else {
                self.team_panel.stale.remove(&session_id);
            }
        }
    }

    /// Note that a session's team may have changed (a turn settled, its
    /// background work moved).
    pub(super) fn mark_team_stale(&mut self, session_id: Uuid) {
        self.team_panel.stale.insert(session_id);
    }

    /// Keep an open surface current while it is shown.
    fn ensure_team_poll(&mut self, cx: &mut Context<Self>) {
        if self.team_panel.poll_running {
            return;
        }
        self.team_panel.poll_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                let wait = this
                    .update(cx, |this, _| {
                        let active = this
                            .selected_team_view()
                            .is_some_and(|view| view.snapshot.is_active());
                        if active { ACTIVE_POLL } else { IDLE_POLL }
                    })
                    .unwrap_or(IDLE_POLL);
                cx.background_executor().timer(wait).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let showing = matches!(
                            this.active_right_panel_surface(),
                            Some(RightPanelSurface::Team)
                        );
                        if !showing {
                            this.team_panel.poll_running = false;
                            return false;
                        }
                        if let Some(session_id) = this.state.selected_session {
                            this.refresh_team_view(session_id, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Send a plan decision to the session's captain, starting its runtime
    /// first when the session is not running.
    fn dispatch_team_control(
        &mut self,
        session_id: Uuid,
        action: ControlAction,
        team_id: String,
        cx: &mut Context<Self>,
    ) {
        let text = encode_control(&Control { action, team_id });
        if let Some(runtime) = self.runtimes.get(&session_id) {
            runtime.driver.prompt(text);
        } else {
            self.team_panel
                .pending_controls
                .entry(session_id)
                .or_default()
                .push(text);
            self.start_goal_runtime(session_id, cx);
        }
        self.team_panel.stale.insert(session_id);
        cx.notify();
    }

    /// The Stop button: the first press arms it, the second stops the team.
    fn press_team_stop(&mut self, session_id: Uuid, team_id: String, cx: &mut Context<Self>) {
        let armed = self
            .team_panel
            .stop_armed
            .is_some_and(|(armed, at)| armed == session_id && at.elapsed() < STOP_CONFIRM_WINDOW);
        if armed {
            self.team_panel.stop_armed = None;
            self.dispatch_team_control(session_id, ControlAction::Stop, team_id, cx);
            return;
        }
        self.team_panel.stop_armed = Some((session_id, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STOP_CONFIRM_WINDOW).await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .team_panel
                    .stop_armed
                    .is_some_and(|(_, at)| at.elapsed() >= STOP_CONFIRM_WINDOW)
                {
                    this.team_panel.stop_armed = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Open a member's live record, when it has one.
    fn open_team_member(&mut self, session_id: Uuid, member_id: String, cx: &mut Context<Self>) {
        let key = BackgroundWorkKey::new(BackgroundWorkKind::Subagent, member_id);
        self.open_background_work_surface(session_id, key, cx);
    }

    /// Put `/agent-teams ` in the composer, ready for a goal.
    fn start_team_in_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |composer, cx| {
            composer.set_content("/agent-teams ".to_owned(), cx);
        });
        window.focus(&self.composer.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    // ---- render -------------------------------------------------------------

    pub(super) fn render_team_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(session_id) = self.state.selected_session else {
            return self.render_team_empty(theme, tr!("team.empty.no_session"), false, cx);
        };
        if !self.team_capable(session_id) {
            return self.render_team_empty(theme, tr!("team.empty.not_native"), false, cx);
        }
        self.ensure_team_poll(cx);
        if !self.team_panel.views.contains_key(&session_id)
            && !self.team_panel.none.contains(&session_id)
            && !self.team_panel.loading.contains(&session_id)
        {
            self.refresh_team_view(session_id, cx);
        }
        let _ = window;
        let Some(view) = self.team_panel.views.get(&session_id) else {
            let message = if self.team_panel.loading.contains(&session_id) {
                tr!("common.loading")
            } else {
                tr!("team.empty.no_team")
            };
            return self.render_team_empty(theme, message, true, cx);
        };
        let snapshot = view.snapshot.clone();
        let layout = view.layout.clone();
        let has_runtime = self.runtimes.contains_key(&session_id);

        div()
            .id("team-surface")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(16.0))
            .py(px(14.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(self.render_team_header(&snapshot, theme))
            .child(render_team_progress(&snapshot, theme))
            .child(self.render_team_actions(session_id, &snapshot, has_runtime, theme, cx))
            .child(self.render_team_members(session_id, &snapshot, theme, cx))
            .child(self.render_team_graph(&snapshot, &layout, theme, cx))
            .when_some(
                self.render_team_task_detail(&snapshot, theme),
                |element, detail| element.child(detail),
            )
            .when(!snapshot.captain_inbox.is_empty(), |element| {
                element.child(render_team_inbox(&snapshot, theme))
            })
            .when_some(self.team_panel.error.clone(), |element, error| {
                element.child(
                    div()
                        .text_size(sp(12.0))
                        .text_color(theme.warning)
                        .child(error),
                )
            })
            .into_any_element()
    }

    fn render_team_empty(
        &self,
        theme: Theme,
        message: String,
        offer_start: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("team-surface-empty")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .px(px(24.0))
            .child(icon("icons/users.svg", 22.0, theme.text_tertiary))
            .child(
                div()
                    .max_w(px(360.0))
                    .text_center()
                    .text_size(sp(12.5))
                    .line_height(sp(18.0))
                    .text_color(theme.text_secondary)
                    .child(message),
            )
            .when(offer_start, |element| {
                element.child(card_button(
                    theme,
                    SharedString::from("team-start-in-composer"),
                    tr!("team.action.start"),
                    true,
                    false,
                    cx,
                    |this, window, cx| this.start_team_in_composer(window, cx),
                ))
            })
            .into_any_element()
    }

    fn render_team_header(&self, snapshot: &TeamSnapshot, theme: Theme) -> Div {
        let (phase_icon, phase_label, phase_color) = phase_badge(snapshot, theme);
        let working = snapshot
            .members
            .iter()
            .filter(|member| member.is_working())
            .count();
        let stats = tr!(
            "team.stats",
            working = working,
            members = snapshot.members.len(),
            done = snapshot.completed(),
            tasks = snapshot.tasks.len(),
            messages = snapshot.message_count
        );
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(snapshot.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(phase_color)
                            .text_size(sp(11.5))
                            .text_color(phase_color)
                            .child(icon(phase_icon, 11.0, phase_color))
                            .child(phase_label),
                    ),
            )
            .when_some(
                snapshot
                    .description
                    .clone()
                    .filter(|text| !text.trim().is_empty()),
                |element, text| {
                    element.child(
                        div()
                            .text_size(sp(12.5))
                            .line_height(sp(18.0))
                            .text_color(theme.text_secondary)
                            .child(text),
                    )
                },
            )
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(stats),
            )
            .when_some(snapshot.profile.clone(), |element, profile| {
                element.child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_tertiary)
                        .child(tr!("team.profile", name = profile)),
                )
            })
    }

    fn render_team_actions(
        &self,
        session_id: Uuid,
        snapshot: &TeamSnapshot,
        has_runtime: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let team_id = snapshot.team_id.clone();
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.0));
        if snapshot.archived {
            return row.child(note(theme, tr!("team.notice.archived")));
        }
        if snapshot.staged {
            if snapshot.awaiting_feedback {
                row = row.child(note(theme, tr!("team.notice.awaiting_feedback")));
            }
            let approve = team_id.clone();
            let revise = team_id.clone();
            let discard = team_id;
            return row
                .child(card_button(
                    theme,
                    SharedString::from("team-approve"),
                    tr!("team.action.approve"),
                    true,
                    snapshot.tasks.is_empty(),
                    cx,
                    move |this, _, cx| {
                        this.dispatch_team_control(
                            session_id,
                            ControlAction::Approve,
                            approve.clone(),
                            cx,
                        )
                    },
                ))
                .child(card_button(
                    theme,
                    SharedString::from("team-revise"),
                    tr!("team.action.revise"),
                    false,
                    snapshot.awaiting_feedback,
                    cx,
                    move |this, _, cx| {
                        this.dispatch_team_control(
                            session_id,
                            ControlAction::Revise,
                            revise.clone(),
                            cx,
                        )
                    },
                ))
                .child(card_button(
                    theme,
                    SharedString::from("team-discard"),
                    tr!("team.action.discard"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| {
                        this.dispatch_team_control(
                            session_id,
                            ControlAction::Discard,
                            discard.clone(),
                            cx,
                        )
                    },
                ));
        }
        if snapshot.halted {
            return row.child(note(theme, tr!("team.notice.halted")));
        }
        if snapshot.escalated {
            row = row.child(note(theme, tr!("team.notice.escalated")));
        }
        let armed = self
            .team_panel
            .stop_armed
            .is_some_and(|(armed, at)| armed == session_id && at.elapsed() < STOP_CONFIRM_WINDOW);
        let stop_id = team_id.clone();
        row = row.child(
            div()
                .id("team-stop")
                .tab_index(0)
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(if armed {
                    theme.danger
                } else {
                    theme.border_strong
                })
                .bg(if armed {
                    theme.danger.opacity(0.12)
                } else {
                    theme.raised
                })
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_default()
                .text_size(sp(12.0))
                .text_color(if armed { theme.danger } else { theme.text })
                .focus_visible(|style| style.border_color(theme.accent))
                .hover(|style| style.bg(theme.overlay))
                .child(icon(
                    "icons/stop.svg",
                    12.0,
                    if armed {
                        theme.danger
                    } else {
                        theme.text_secondary
                    },
                ))
                .child(if armed {
                    tr!("team.action.stop_confirm")
                } else {
                    tr!("team.action.stop")
                })
                .on_activation(cx, move |this, _, cx| {
                    this.press_team_stop(session_id, stop_id.clone(), cx)
                }),
        );
        if !has_runtime && snapshot.is_active() {
            row = row.child(card_button(
                theme,
                SharedString::from("team-continue"),
                tr!("team.action.continue"),
                false,
                false,
                cx,
                move |this, _, cx| {
                    this.dispatch_team_control(session_id, ControlAction::Kick, team_id.clone(), cx)
                },
            ));
        }
        row
    }

    fn render_team_members(
        &self,
        session_id: Uuid,
        snapshot: &TeamSnapshot,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().flex().flex_col().gap(px(2.0));
        if snapshot.members.is_empty() {
            list = list.child(note(theme, tr!("team.members_empty")));
        }
        for member in snapshot.ordered_members() {
            let (status_icon, status_label, status_color) = member_status(member, theme);
            let has_record = !member.id.is_empty()
                && self.has_background_work(
                    session_id,
                    &BackgroundWorkKey::new(BackgroundWorkKind::Subagent, member.id.clone()),
                );
            let member_id = member.id.clone();
            let detail = {
                let mut parts = Vec::new();
                if !member.role.is_empty() {
                    parts.push(member.role.clone());
                }
                let route = member.route_label();
                if !route.is_empty() {
                    parts.push(route);
                }
                parts.push(tr!(
                    "team.member_tasks",
                    done = member.done,
                    total = member.total
                ));
                if member.unread > 0 {
                    parts.push(tr!("team.member_unread", count = member.unread));
                }
                parts.join(" · ")
            };
            let row = div()
                .id(SharedString::from(format!("team-member-{}", member.name)))
                .tab_index(0)
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(7.0))
                .flex()
                .items_start()
                .gap(px(8.0))
                .cursor_default()
                .focus_visible(|style| style.bg(theme.overlay))
                .hover(|style| style.bg(theme.overlay))
                .child(
                    div()
                        .pt(px(2.0))
                        .child(icon(status_icon, 13.0, status_color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(sp(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(member.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(sp(11.5))
                                        .text_color(status_color)
                                        .child(status_label),
                                ),
                        )
                        .child(
                            div()
                                .text_size(sp(11.5))
                                .text_color(theme.text_tertiary)
                                .child(detail),
                        )
                        .when_some(member.current_task.clone(), |element, task| {
                            element.child(
                                div()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_secondary)
                                    .child(tr!("team.member_current", task = task)),
                            )
                        })
                        .when_some(member.spawn_error.clone(), |element, error| {
                            element.child(
                                div()
                                    .text_size(sp(11.5))
                                    .text_color(theme.warning)
                                    .child(tr!(
                                        "team.member_start_failed",
                                        error = error.lines().next().unwrap_or_default()
                                    )),
                            )
                        }),
                );
            list = list.child(if has_record {
                row.on_activation(cx, move |this, _, cx| {
                    this.open_team_member(session_id, member_id.clone(), cx)
                })
            } else {
                row
            });
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(section_label(theme, tr!("team.members")))
            .child(list)
    }

    fn render_team_graph(
        &self,
        snapshot: &TeamSnapshot,
        layout: &DagLayout,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let block = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(section_label(theme, tr!("team.tasks")));
        if snapshot.tasks.is_empty() {
            return block.child(note(theme, tr!("team.tasks_empty")));
        }
        let dims = dag_dims();
        let pinned = self.team_panel.pinned_task.clone();
        let related = pinned
            .as_deref()
            .map(|id| related_task_ids(id, &snapshot.tasks))
            .unwrap_or_default();

        if uses_parallel_task_grid(&snapshot.tasks) {
            let mut grid = div().flex().flex_wrap().gap(px(8.0));
            for task in &snapshot.tasks {
                grid = grid.child(self.render_team_node(
                    task,
                    None,
                    &related,
                    pinned.as_deref(),
                    dims,
                    theme,
                    cx,
                ));
            }
            return block.child(grid);
        }

        let edges: Vec<(agent_teams::snapshot::DagEdge, bool, bool)> = layout
            .edges
            .iter()
            .map(|edge| {
                let done = snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == edge.from)
                    .is_some_and(|task| task.state == VisualTaskState::Completed);
                let lit = related.contains(&edge.from) && related.contains(&edge.to);
                (edge.clone(), done, lit)
            })
            .collect();
        let (edge_color, waiting_color, lit_color) =
            (theme.border_strong, theme.text_ghost, theme.accent);
        let edge_layer = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let ox = f32::from(bounds.origin.x);
                let oy = f32::from(bounds.origin.y);
                let at = |p: (f32, f32)| point(px(ox + p.0), px(oy + p.1));
                for (edge, done, lit) in &edges {
                    let color = if *lit {
                        lit_color
                    } else if *done {
                        edge_color
                    } else {
                        waiting_color
                    };
                    let mut line = PathBuilder::stroke(px(1.5));
                    if !*done {
                        line = line.dash_array(&[px(4.0), px(4.0)]);
                    }
                    line.move_to(at(edge.start));
                    line.cubic_bezier_to(at(edge.end), at(edge.control1), at(edge.control2));
                    if let Ok(path) = line.build() {
                        window.paint_path(path, color);
                    }
                    let head = arrow_head(edge.end, edge.control2, 7.0);
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
        .absolute()
        .top_0()
        .left_0()
        .w(px(layout.width))
        .h(px(layout.height));

        let mut area = div()
            .relative()
            .w(px(layout.width))
            .h(px(layout.height))
            .child(edge_layer);
        for node in &layout.nodes {
            let task = &snapshot.tasks[node.task];
            area = area.child(self.render_team_node(
                task,
                Some((node.x, node.y)),
                &related,
                pinned.as_deref(),
                dims,
                theme,
                cx,
            ));
        }
        block.child(
            div()
                .id("team-graph")
                .w_full()
                .overflow_x_scroll()
                .p(px(10.0))
                .rounded(px(10.0))
                .bg(theme.inset)
                .border_1()
                .border_color(theme.border)
                .child(area),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_team_node(
        &self,
        task: &agent_teams::snapshot::TaskRow,
        position: Option<(f32, f32)>,
        related: &HashSet<String>,
        pinned: Option<&str>,
        dims: DagDims,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (state_icon, state_label, state_color) = task_state(task.state, theme);
        let is_pinned = pinned == Some(task.id.as_str());
        let dimmed = pinned.is_some() && !related.contains(&task.id);
        let id = task.id.clone();
        let owner = if task.assignee.is_empty() {
            tr!("team.task_shared")
        } else {
            task.assignee.clone()
        };
        let node = div()
            .id(SharedString::from(format!("team-task-{}", task.id)))
            .tab_index(0)
            .w(px(dims.node_width))
            .h(px(dims.node_height))
            .px(px(7.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(if is_pinned { theme.accent } else { state_color })
            .bg(theme.raised)
            .opacity(if dimmed { 0.45 } else { 1.0 })
            .flex()
            .flex_col()
            .justify_center()
            .cursor_default()
            .focus_visible(|style| style.border_color(theme.accent))
            .hover(|style| style.bg(theme.overlay))
            .tooltip(Tooltip::text(SharedString::from(format!(
                "{} · {} · {}",
                task.subject, state_label, owner
            ))))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(icon(state_icon, 10.0, state_color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(sp(11.5))
                            .text_color(theme.text)
                            .child(format!("{} {}", task.id, task.subject)),
                    ),
            )
            .child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(sp(10.5))
                    .text_color(theme.text_tertiary)
                    .child(owner),
            )
            .on_activation(cx, move |this, _, cx| {
                this.team_panel.pinned_task =
                    if this.team_panel.pinned_task.as_deref() == Some(id.as_str()) {
                        None
                    } else {
                        Some(id.clone())
                    };
                cx.notify();
            });
        match position {
            Some((x, y)) => node.absolute().left(px(x)).top(px(y)),
            None => node,
        }
    }

    fn render_team_task_detail(&self, snapshot: &TeamSnapshot, theme: Theme) -> Option<Div> {
        let pinned = self.team_panel.pinned_task.as_deref()?;
        let task = snapshot.tasks.iter().find(|task| task.id == pinned)?;
        let (_, state_label, state_color) = task_state(task.state, theme);
        let mut facts = vec![state_label];
        if let Some(kind) = &task.kind {
            facts.push(kind.clone());
        }
        if let Some(round) = task.round {
            facts.push(tr!("team.task_round", round = round));
        }
        if let Some(verdict) = &task.verdict {
            facts.push(tr!("team.task_verdict", verdict = verdict.clone()));
        }
        if task.open_findings > 0 {
            facts.push(tr!("team.task_findings", count = task.open_findings));
        }
        let waiting: Vec<&str> = task
            .dependencies
            .iter()
            .filter(|id| {
                snapshot
                    .tasks
                    .iter()
                    .any(|other| &other.id == *id && other.state != VisualTaskState::Completed)
            })
            .map(String::as_str)
            .collect();
        Some(
            div()
                .p(px(12.0))
                .rounded(px(9.0))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(sp(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(format!("{} — {}", task.id, task.subject)),
                )
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(state_color)
                        .child(facts.join(" · ")),
                )
                .when(!task.description.trim().is_empty(), |element| {
                    element.child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text_secondary)
                            .child(task.description.clone()),
                    )
                })
                .when(!waiting.is_empty(), |element| {
                    element.child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.text_tertiary)
                            .child(tr!("team.task_waiting", tasks = waiting.join(", "))),
                    )
                })
                .when_some(task.output.clone(), |element, output| {
                    let output: String = output.chars().take(600).collect();
                    element.child(
                        div()
                            .mt(px(4.0))
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text)
                            .child(output),
                    )
                }),
        )
    }
}

fn section_label(theme: Theme, text: String) -> Div {
    div()
        .text_size(sp(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text_secondary)
        .child(text)
}

fn note(theme: Theme, text: String) -> Div {
    div()
        .text_size(sp(12.0))
        .line_height(sp(17.0))
        .text_color(theme.text_tertiary)
        .child(text)
}

fn phase_badge(snapshot: &TeamSnapshot, theme: Theme) -> (&'static str, String, Hsla) {
    if snapshot.archived {
        (
            "icons/package.svg",
            tr!("team.phase.archived"),
            theme.text_tertiary,
        )
    } else if snapshot.staged {
        if snapshot.awaiting_feedback {
            (
                "icons/pencil.svg",
                tr!("team.phase.revising"),
                theme.warning,
            )
        } else {
            ("icons/list.svg", tr!("team.phase.review"), theme.accent)
        }
    } else if snapshot.halted {
        ("icons/stop.svg", tr!("team.phase.halted"), theme.danger)
    } else if snapshot.escalated {
        (
            "icons/alert.svg",
            tr!("team.phase.escalated"),
            theme.warning,
        )
    } else if snapshot.is_active() {
        (
            "icons/loader-circle.svg",
            tr!("team.phase.running"),
            theme.accent,
        )
    } else {
        ("icons/check.svg", tr!("team.phase.done"), theme.success)
    }
}

fn member_status(
    member: &agent_teams::snapshot::MemberRow,
    theme: Theme,
) -> (&'static str, String, Hsla) {
    if member.status == MemberStatus::Removed {
        ("icons/x.svg", tr!("team.member.removed"), theme.text_ghost)
    } else if member.spawn_error.is_some() && member.id.is_empty() {
        ("icons/alert.svg", tr!("team.member.failed"), theme.warning)
    } else if member.is_working() {
        (
            "icons/loader-circle.svg",
            tr!("team.member.working"),
            theme.accent,
        )
    } else if member.activity == MemberLiveActivity::Unknown {
        (
            "icons/bot.svg",
            tr!("team.member.waiting"),
            theme.text_tertiary,
        )
    } else {
        (
            "icons/check.svg",
            tr!("team.member.idle"),
            theme.text_secondary,
        )
    }
}

fn task_state(state: VisualTaskState, theme: Theme) -> (&'static str, String, Hsla) {
    match state {
        VisualTaskState::Blocked => ("icons/lock.svg", tr!("team.task.blocked"), theme.text_ghost),
        VisualTaskState::Open => (
            "icons/queue.svg",
            tr!("team.task.open"),
            theme.text_secondary,
        ),
        VisualTaskState::Running => (
            "icons/loader-circle.svg",
            tr!("team.task.running"),
            theme.accent,
        ),
        VisualTaskState::Completed => {
            ("icons/check.svg", tr!("team.task.completed"), theme.success)
        }
        VisualTaskState::Failed => ("icons/circle-x.svg", tr!("team.task.failed"), theme.danger),
        VisualTaskState::Cancelled => ("icons/x.svg", tr!("team.task.cancelled"), theme.text_ghost),
    }
}

/// One segment per task, coloured by state, with the counts in words beside
/// it — the colours alone never carry the meaning.
fn render_team_progress(snapshot: &TeamSnapshot, theme: Theme) -> Div {
    let mut bar = div().h(px(6.0)).w_full().flex().gap(px(2.0));
    for task in &snapshot.tasks {
        let (_, _, color) = task_state(task.state, theme);
        bar = bar.child(div().flex_1().h_full().rounded(px(2.0)).bg(color));
    }
    let counts = snapshot.state_counts();
    let words: Vec<String> = [
        (VisualTaskState::Running, "running"),
        (VisualTaskState::Open, "open"),
        (VisualTaskState::Blocked, "blocked"),
        (VisualTaskState::Completed, "completed"),
        (VisualTaskState::Failed, "failed"),
        (VisualTaskState::Cancelled, "cancelled"),
    ]
    .into_iter()
    .filter_map(|(state, key)| {
        let count = counts.get(key).copied().unwrap_or(0);
        (count > 0).then(|| format!("{} {count}", task_state(state, theme).1))
    })
    .collect();
    div()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .when(!snapshot.tasks.is_empty(), |element| element.child(bar))
        .child(
            div()
                .text_size(sp(11.5))
                .text_color(theme.text_tertiary)
                .child(words.join(" · ")),
        )
}

fn render_team_inbox(snapshot: &TeamSnapshot, theme: Theme) -> Div {
    let mut list = div().flex().flex_col().gap(px(6.0));
    for message in &snapshot.captain_inbox {
        let content: String = message.content.chars().take(280).collect();
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_secondary)
                        .child(message.from.clone()),
                )
                .child(
                    div()
                        .text_size(sp(12.0))
                        .line_height(sp(17.0))
                        .text_color(theme.text)
                        .child(content),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(section_label(theme, tr!("team.inbox")))
        .child(list)
}

/// The arrowhead at `end`, pointing along the curve's last tangent.
fn arrow_head(end: (f32, f32), control: (f32, f32), size: f32) -> [(f32, f32); 3] {
    let (dx, dy) = (end.0 - control.0, end.1 - control.1);
    let length = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let base = (end.0 - ux * size, end.1 - uy * size);
    let (px_, py_) = (-uy * size * 0.5, ux * size * 0.5);
    [
        end,
        (base.0 + px_, base.1 + py_),
        (base.0 - px_, base.1 - py_),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrowhead_points_along_the_curve() {
        let head = arrow_head((10.0, 0.0), (0.0, 0.0), 4.0);
        assert_eq!(head[0], (10.0, 0.0));
        assert!((head[1].0 - 6.0).abs() < 1e-4 && (head[2].0 - 6.0).abs() < 1e-4);
        assert!((head[1].1 + head[2].1).abs() < 1e-4);
    }
}

impl Waku {
    /// The header's Team button: there while the selected session leads a
    /// team, it opens the surface or puts it away.
    pub(super) fn render_team_surface_button(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if !self.selected_session_has_team() {
            return None;
        }
        let shown = self.right_panel_visible
            && matches!(
                self.active_right_panel_surface(),
                Some(RightPanelSurface::Team)
            );
        let tooltip: SharedString = if shown {
            tr!("team.surface_hide")
        } else {
            tr!("team.surface_open")
        }
        .into();
        Some(
            div()
                .id("surface-bar-team")
                .tab_index(0)
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .w(px(26.0))
                .h(px(26.0))
                .flex_none()
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .when(shown, |element| element.bg(theme.overlay))
                .hover(|element| element.bg(theme.overlay))
                .child(icon(
                    "icons/users.svg",
                    14.0,
                    if shown {
                        theme.accent
                    } else {
                        theme.text_tertiary
                    },
                ))
                .tooltip(Tooltip::text(tooltip))
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_activation(cx, move |this, _, cx| {
                    if shown {
                        this.set_right_panel_visible(false, cx);
                    } else {
                        this.open_right_panel_surface(RightPanelSurface::Team, cx);
                    }
                }),
        )
    }
}
