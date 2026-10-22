//! The session rail: what a task is working with, beside its transcript.
//!
//! Fork addition, after Claude Desktop's session details rail. A column of
//! cards to the right of the conversation — Environment (where the task
//! runs: this computer, its folder, its branch), Context (how full the
//! window is, and how the last request split), Changes (what the working
//! tree holds against the last commit, with Review and Commit one click
//! away) and Background tasks (what is running, with its time and a stop
//! button, and what finished). The right panel takes the same column: when
//! a tool opens there — the browser, the terminal, Review — it covers the
//! rail, which comes back when the panel goes away. A header button shows or
//! hides the rail; it also steps aside on a window too narrow for both.
//!
//! The todo list moves into the stream the same way: while a turn runs, the
//! session's list stays at the bottom of the conversation, above the
//! composer, instead of only under the call that last wrote it.
//!
//! Frames read only stored state; the Changes card's figures are read by a
//! daemon request off the UI thread, throttled and dropped when superseded.

use std::time::{Duration, Instant};

use agent_trajectory::metrics::format_tokens;

use crate::ui::ActivationExt as _;

use super::background_work::{rendered_work_status_icon, work_elapsed, work_kind_icon, work_status_color};
use super::providers_page::card_button;
use super::todo_list::render_todo_list;
use super::*;

/// The rail's width.
pub(super) const RAIL_WIDTH: f32 = 260.0;
/// The conversation keeps at least this much beside it. Low enough that a
/// default window with the sidebar open (252 px) shows the rail from about
/// 910 px wide.
const MIN_CHAT_WIDTH: f32 = 400.0;
/// How often a running task's changes are read again.
const BUSY_REFRESH: Duration = Duration::from_secs(5);
/// The shortest gap between two reads.
const MIN_REFRESH_GAP: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Changes {
    snapshot: Option<waku_client::git_commit::Snapshot>,
    /// The folder is not a Git repository, or the read failed.
    unavailable: bool,
    loading: bool,
    last_read: Option<Instant>,
    /// `updated_at` of the session when it was last read.
    read_for: u64,
    generation: u64,
}

pub(super) struct SessionRailState {
    /// The person's choice; the rail still steps aside for the right panel
    /// and for a narrow window.
    pub(super) enabled: bool,
    /// The width it takes this frame (0 when not shown).
    pub(super) rendered_width: f32,
    changes: HashMap<Uuid, Changes>,
    context_open: bool,
    finished_open: bool,
    /// Sessions whose docked todo list is folded to its header.
    todo_folded: HashSet<Uuid>,
}

impl Default for SessionRailState {
    fn default() -> Self {
        Self {
            enabled: true,
            rendered_width: 0.0,
            changes: HashMap::new(),
            context_open: false,
            finished_open: false,
            todo_folded: HashSet::new(),
        }
    }
}

/// Whether the rail fits beside the conversation in `viewport` with the
/// sidebar taking `sidebar`.
pub(super) fn rail_fits(viewport: f32, sidebar: f32) -> bool {
    viewport - sidebar - RAIL_WIDTH >= MIN_CHAT_WIDTH
}

fn card(theme: &Theme) -> Div {
    div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
}

fn card_title(theme: &Theme, title: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(sp(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(title)
}

fn fact_row(theme: &Theme, icon_path: &'static str, text: String, detail: Option<String>) -> Div {
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(sp(12.0))
        .text_color(theme.text_secondary)
        .child(icon(icon_path, 12.0, theme.text_tertiary))
        .child(div().flex_none().child(text))
        .children(detail.map(|detail| {
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.text_tertiary)
                .child(detail)
        }))
}

/// `~/…` for a path under the home folder.
fn shorten_home(path: &Path) -> String {
    let text = path.display().to_string();
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => text,
    }
}

impl Waku {
    /// Whether the rail is shown this frame: wanted, on a task, with the
    /// right panel away and room for both.
    fn session_rail_shown(&self, window: &Window) -> bool {
        self.session_rail.enabled
            && self.settings_page.is_none()
            && !self.main_page_open()
            && !self.right_panel_visible
            && self.right_panel_rendered_width <= 0.0
            && self.selected_session().is_some_and(|session| session.has_started())
            && rail_fits(f32::from(window.viewport_size().width), self.sidebar_rendered_width)
    }

    /// The rail's width this frame, for the layout and the chat column.
    pub(super) fn session_rail_width(&self, window: &Window) -> f32 {
        if self.session_rail_shown(window) {
            RAIL_WIDTH
        } else {
            0.0
        }
    }

    /// Whether the rail is on screen (the status capsule then stands down:
    /// the rail and the docked todo list say what it said).
    pub(super) fn session_rail_on_screen(&self) -> bool {
        self.session_rail.rendered_width > 0.0
    }

    /// The header button: show or hide the rail. With the right panel in
    /// front, it puts the panel away and shows the rail.
    pub(super) fn render_session_rail_button(&self, theme: Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        self.selected_session()?;
        let shown = self.session_rail_on_screen();
        let tooltip: SharedString = if shown {
            tr!("session_rail.hide")
        } else {
            tr!("session_rail.show")
        }
        .into();
        Some(
            div()
                .id("surface-bar-session-rail")
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
                    "icons/panel-right.svg",
                    14.0,
                    if shown { theme.accent } else { theme.text_tertiary },
                ))
                .tooltip(Tooltip::text(tooltip))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_activation(cx, move |this, _, cx| {
                    if this.right_panel_visible {
                        this.set_right_panel_visible(false, cx);
                        this.session_rail.enabled = true;
                    } else {
                        this.session_rail.enabled = !shown;
                    }
                    cx.notify();
                }),
        )
    }

    /// Read the working tree's changes again when the task moved since the
    /// last read, or every few seconds while it runs.
    fn maybe_refresh_rail_changes(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(session) = self.state.sessions.iter().find(|session| session.id == session_id) else {
            return;
        };
        let busy = session.status.is_busy();
        let updated_at = session.updated_at;
        let Some(cwd) = self.workspace_path_for_session(session).map(Path::to_path_buf) else {
            return;
        };
        let entry = self.session_rail.changes.entry(session_id).or_default();
        if entry.loading {
            return;
        }
        let elapsed = entry.last_read.map(|read| read.elapsed());
        let due = match elapsed {
            None => true,
            Some(elapsed) if elapsed < MIN_REFRESH_GAP => false,
            Some(elapsed) => entry.read_for != updated_at || (busy && elapsed >= BUSY_REFRESH),
        };
        if !due {
            return;
        }
        entry.loading = true;
        entry.last_read = Some(Instant::now());
        entry.read_for = updated_at;
        entry.generation += 1;
        let generation = entry.generation;
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { workspace.request(waku_client::WorkspaceOperation::InspectCommit { cwd }) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(entry) = this.session_rail.changes.get_mut(&session_id) else {
                    return;
                };
                entry.loading = false;
                if entry.generation != generation {
                    return;
                }
                match read {
                    Ok(waku_client::WorkspaceResult::CommitSnapshot { snapshot }) => {
                        entry.snapshot = Some(snapshot);
                        entry.unavailable = false;
                    }
                    _ => entry.unavailable = true,
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_session_rail(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.session_rail.rendered_width <= 0.0 {
            return None;
        }
        let session_id = self.state.selected_session?;
        self.maybe_refresh_rail_changes(session_id, cx);
        let theme = Theme::current(cx);
        let environment = self.render_rail_environment(&theme)?;
        let context = self.render_rail_context(&theme, cx);
        let changes = self.render_rail_changes(session_id, &theme, cx);
        let background = self.render_rail_background(session_id, &theme, cx);
        Some(
            div()
                .id("session-rail")
                .h_full()
                .flex_none()
                .w(px(RAIL_WIDTH))
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.canvas)
                .overflow_y_scroll()
                .child(
                    div()
                        .p(px(10.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(environment)
                        .children(context)
                        .child(changes)
                        .child(background),
                )
                .into_any_element(),
        )
    }

    fn render_rail_environment(&self, theme: &Theme) -> Option<Div> {
        let session = self.selected_session()?;
        let project = self.state.projects.iter().find(|project| project.id == session.project_id);
        let folder_name = project
            .filter(|project| !project.is_projectless())
            .map(Project::display_name)
            .unwrap_or_else(|| tr!("project.no_project_name"));
        let folder_path = self.workspace_path_for_session(session).map(shorten_home);
        let branch = self
            .session_rail
            .changes
            .get(&session.id)
            .and_then(|changes| changes.snapshot.as_ref())
            .map(|snapshot| snapshot.branch.clone())
            .filter(|branch| !branch.is_empty());
        let worktree = session.workspace.is_worktree();
        Some(
            card(theme)
                .child(card_title(theme, tr!("session_rail.environment")))
                .child(fact_row(theme, "icons/laptop.svg", tr!("session_rail.this_computer"), None))
                .child(fact_row(theme, "icons/folder.svg", folder_name, folder_path))
                .children(branch.map(|branch| {
                    fact_row(
                        theme,
                        "icons/git-branch.svg",
                        branch,
                        worktree.then(|| tr!("session_rail.worktree")),
                    )
                })),
        )
    }

    fn render_rail_context(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.selected_session()?;
        let usage = self.ring_context_usage(session)?;
        let percent = usage
            .window
            .filter(|window| *window > 0)
            .map(|window| (usage.tokens as f64 * 100.0 / window as f64).clamp(0.0, 100.0));
        let open = self.session_rail.context_open;
        let bar_color = match percent {
            Some(percent) if percent >= 90.0 => theme.danger,
            Some(percent) if percent >= 75.0 => theme.warning,
            _ => theme.accent,
        };
        let header = div()
            .id("session-rail-context")
            .tab_index(0)
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .on_activation(cx, |this, _, cx| {
                this.session_rail.context_open = !this.session_rail.context_open;
                cx.notify();
            })
            .child(card_title(theme, tr!("session_rail.context")))
            .child(icon(
                if open { "icons/chevron-down.svg" } else { "icons/chevron-right.svg" },
                10.0,
                theme.text_tertiary,
            ))
            .child(div().flex_1())
            .child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_secondary)
                    .child(match percent {
                        Some(percent) => format!("{percent:.0}%"),
                        None => format_tokens(usage.tokens),
                    }),
            )
            .children(percent.map(|percent| {
                div()
                    .w(px(44.0))
                    .h(px(5.0))
                    .rounded_full()
                    .bg(theme.overlay)
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(bar_color)
                            .w(px((44.0 * percent / 100.0) as f32)),
                    )
            }));
        let mut body = card(theme).child(header);
        if open {
            let line = |label: String, value: String| {
                div()
                    .flex()
                    .text_size(sp(11.5))
                    .child(div().flex_1().text_color(theme.text_tertiary).child(label))
                    .child(div().text_color(theme.text_secondary).child(value))
            };
            body = body.child(line(
                tr!("session_rail.context_used"),
                match usage.window {
                    Some(window) => format!("{} / {}", format_tokens(usage.tokens), format_tokens(window)),
                    None => format_tokens(usage.tokens),
                },
            ));
            if let Some(last) = usage.last {
                body = body
                    .child(line(tr!("session_rail.last_input"), format_tokens(last.uncached_input)))
                    .child(line(tr!("session_rail.last_cached"), format_tokens(last.cached_input)))
                    .child(line(tr!("session_rail.last_output"), format_tokens(last.output)));
            }
            if let Some(total) = usage.session {
                body = body.child(line(tr!("session_rail.session_total"), format_tokens(total.total())));
            }
        }
        Some(body)
    }

    fn render_rail_changes(&self, session_id: Uuid, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let changes = self.session_rail.changes.get(&session_id);
        let summary: AnyElement = match changes {
            Some(changes) if changes.unavailable => div()
                .text_size(sp(12.0))
                .text_color(theme.text_tertiary)
                .child(tr!("session_rail.no_git"))
                .into_any_element(),
            Some(Changes {
                snapshot: Some(snapshot),
                ..
            }) if snapshot.additions + snapshot.deletions > 0 => div()
                .id("session-rail-changes-summary")
                .tab_index(0)
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(sp(12.5))
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .hover(|style| style.bg(theme.overlay))
                .tooltip(Tooltip::text(tr!("session_rail.open_review")))
                .on_activation(cx, |this, _, cx| {
                    this.open_right_panel_surface(RightPanelSurface::Diff, cx);
                })
                .child(icon("icons/file-diff.svg", 12.0, theme.text_tertiary))
                .child(div().text_color(theme.success).child(format!("+{}", snapshot.additions)))
                .child(div().text_color(theme.danger).child(format!("−{}", snapshot.deletions)))
                .into_any_element(),
            Some(Changes {
                snapshot: Some(_), ..
            }) => div()
                .text_size(sp(12.0))
                .text_color(theme.text_tertiary)
                .child(tr!("session_rail.no_changes"))
                .into_any_element(),
            _ => div()
                .text_size(sp(12.0))
                .text_color(theme.text_tertiary)
                .child(tr!("common.loading"))
                .into_any_element(),
        };
        let has_changes = changes
            .and_then(|changes| changes.snapshot.as_ref())
            .is_some_and(|snapshot| snapshot.additions + snapshot.deletions > 0);
        card(theme)
            .child(card_title(theme, tr!("session_rail.changes")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().flex_1().min_w_0().child(summary))
                    .when(has_changes, |row| {
                        row.child(card_button(
                            *theme,
                            SharedString::from("session-rail-commit"),
                            tr!("session_rail.commit"),
                            false,
                            false,
                            cx,
                            |this, window, cx| this.open_commit_dialog(window, cx),
                        ))
                    }),
            )
    }

    fn render_rail_background(&self, session_id: Uuid, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let all = self.session_background_work(session_id);
        let (live, finished): (Vec<&BackgroundWorkItem>, Vec<&BackgroundWorkItem>) =
            all.into_iter().partition(|item| item.status.is_live());
        let open = self.session_rail.finished_open;
        let row = |item: &BackgroundWorkItem, cx: &mut Context<Self>| {
            let key = item.key.clone();
            let stop_key = item.key.clone();
            let stoppable = item.status.is_stoppable() && item.can_stop;
            let id = SharedString::from(format!("session-rail-work-{}", item.key.provider_id));
            div()
                .id(id.clone())
                .tab_index(0)
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .py(px(2.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(sp(12.0))
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .hover(|style| style.bg(theme.overlay))
                .on_activation(cx, move |this, _, cx| {
                    this.open_background_work_surface(session_id, key.clone(), cx);
                })
                .child(icon(work_kind_icon(item.key.kind), 12.0, theme.text_tertiary))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text_secondary)
                        .child(item.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(sp(11.0))
                        .text_color(theme.text_tertiary)
                        .child(work_elapsed(item)),
                )
                .child(rendered_work_status_icon(item.status, 11.0, work_status_color(item.status, *theme)))
                .when(stoppable, |row| {
                    row.child(
                        div()
                            .id(SharedString::from(format!("{id}-stop")))
                            .tab_index(0)
                            .size(px(18.0))
                            .flex_none()
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|style| style.bg(theme.overlay_strong))
                            .focus_visible(|style| style.border_1().border_color(theme.accent))
                            .tooltip(Tooltip::text(tr!("session_rail.stop")))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_activation(cx, move |this, _, cx| {
                                cx.stop_propagation();
                                this.stop_background_work(session_id, stop_key.clone(), cx);
                            })
                            .child(icon("icons/stop.svg", 10.0, theme.text_secondary)),
                    )
                })
        };
        let mut body = card(theme).child(card_title(theme, tr!("session_rail.background")));
        if live.is_empty() && finished.is_empty() {
            return body.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("session_rail.no_background")),
            );
        }
        for item in &live {
            body = body.child(row(item, cx));
        }
        if !finished.is_empty() {
            body = body.child(
                div()
                    .id("session-rail-finished")
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .on_activation(cx, |this, _, cx| {
                        this.session_rail.finished_open = !this.session_rail.finished_open;
                        cx.notify();
                    })
                    .child(icon(
                        if open { "icons/chevron-down.svg" } else { "icons/chevron-right.svg" },
                        10.0,
                        theme.text_tertiary,
                    ))
                    .child(tr!("session_rail.finished", count = finished.len())),
            );
            if open {
                for item in &finished {
                    body = body.child(row(item, cx));
                }
            }
        }
        body
    }

    /// The todo list, kept at the bottom of the conversation while a turn
    /// runs. Folds to its header on a click; the fold is per task.
    pub(super) fn render_todo_dock(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.selected_session()?;
        if !session.status.is_busy() || session.todos.is_empty() {
            return None;
        }
        let session_id = session.id;
        let theme = Theme::current(cx);
        let done = session
            .todos
            .iter()
            .filter(|todo| matches!(todo.status, waku_protocol::todo::TodoStatus::Completed))
            .count();
        let folded = self.session_rail.todo_folded.contains(&session_id);
        let header = div()
            .id("todo-dock-header")
            .tab_index(0)
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(sp(12.0))
            .text_color(theme.text_secondary)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .on_activation(cx, move |this, _, cx| {
                if !this.session_rail.todo_folded.remove(&session_id) {
                    this.session_rail.todo_folded.insert(session_id);
                }
                cx.notify();
            })
            .child(icon("icons/list.svg", 12.0, theme.text_tertiary))
            .child(tr!("session_rail.todos", done = done, total = session.todos.len()))
            .child(div().flex_1())
            .child(icon(
                if folded { "icons/chevron-up.svg" } else { "icons/chevron-down.svg" },
                10.0,
                theme.text_tertiary,
            ));
        Some(
            div()
                .flex_none()
                .w_full()
                .px(px(16.0))
                .pb(px(6.0))
                .child(
                    div()
                        .max_w(px(760.0))
                        .mx_auto()
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface)
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(header)
                        .when(!folded, |card| {
                            card.child(
                                div()
                                    .id("todo-dock-items")
                                    .max_h(px(168.0))
                                    .overflow_y_scroll()
                                    .child(render_todo_list(&session.todos, 12.5, &theme)),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_steps_aside_on_a_narrow_window() {
        assert!(rail_fits(1440.0, 252.0));
        assert!(rail_fits(1000.0, 252.0));
        assert!(!rail_fits(880.0, 252.0));
        assert!(rail_fits(880.0, 0.0));
    }

    #[test]
    fn home_paths_are_shortened() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(shorten_home(&home.join("code/app")), "~/code/app");
        }
        assert_eq!(shorten_home(Path::new("/opt/x")), "/opt/x");
    }
}
