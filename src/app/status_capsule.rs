//! The status capsule: one line over the top right of the transcript saying
//! the most pressing thing about the task — the goal it pursues, the plan
//! step it is on, how far through the plan it is, what runs in the
//! background — opening onto all of them.
//!
//! Fork addition. The facts already live elsewhere (the composer's goal chip,
//! the todo list under the tool that wrote it, the right panel's background
//! work, each turn's changed-files card); this gathers them where the reader
//! of a long task looks. Which line it shows is decided in
//! `waku_client::status_capsule`.

use super::*;
use crate::status_capsule::{CapsuleFact, select_fact, todo_progress};

const PANEL_WIDTH: f32 = 320.0;

impl Waku {
    pub(super) fn render_status_capsule(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.selected_session()?;
        let (processes, agents) = self.background_work_counts(session.id);
        let goal_status = session.thread_goal.as_ref().map(|goal| goal.status);
        let fact = select_fact(goal_status, &session.todos, processes + agents)?;
        let theme = Theme::current(cx);
        let open = self.status_capsule_open;
        let (icon_path, color, label) = match fact {
            CapsuleFact::Goal => {
                let goal = session.thread_goal.as_ref()?;
                (
                    "icons/target.svg",
                    super::goal_dialog::goal_status_color(goal.status, &theme),
                    super::goal_dialog::goal_chip_label(
                        goal,
                        self.goal_live_elapsed_seconds(session),
                    ),
                )
            }
            CapsuleFact::CurrentTodo(step) => ("icons/list.svg", theme.accent, format!("→ {step}")),
            CapsuleFact::Progress { done, total } => (
                "icons/list.svg",
                theme.text_tertiary,
                tr!("status.progress_fact", done = done, total = total),
            ),
            CapsuleFact::Background(count) => (
                "icons/terminal.svg",
                theme.text_tertiary,
                tr!("status.background_fact", count = count),
            ),
            CapsuleFact::GoalComplete => {
                ("icons/target.svg", theme.success, tr!("status.goal_done"))
            }
        };
        let pill = div()
            .id("status-capsule")
            .h(px(28.0))
            .max_w(px(PANEL_WIDTH))
            .px(px(10.0))
            .rounded_full()
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.raised)
            .shadow_md()
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .hover(|pill| pill.text_color(theme.text))
            .child(icon(icon_path, 12.0, color))
            .child(div().min_w_0().truncate().child(label))
            .child(icon(
                if open {
                    "icons/chevron-up.svg"
                } else {
                    "icons/chevron-down.svg"
                },
                10.0,
                theme.text_tertiary,
            ))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.status_capsule_open = !this.status_capsule_open;
                cx.notify();
            }));
        let mut root = div()
            .absolute()
            .top(px(10.0))
            .right(px(16.0))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(6.0))
            .child(pill);
        if open {
            root = root.child(self.render_status_panel(session, &theme, cx));
        }
        Some(root.into_any_element())
    }

    /// Seconds of pursuit the current turn adds to an active goal's recorded
    /// time — the provider accounts only time a turn actually runs.
    fn goal_live_elapsed_seconds(&self, session: &AgentSession) -> i64 {
        session
            .thread_goal
            .as_ref()
            .filter(|goal| {
                goal.status == crate::model::ThreadGoalStatus::Active && session.is_busy()
            })
            .and_then(|_| self.goal_observed_at.get(&session.id))
            .map_or(0, |observed| observed.elapsed().as_secs() as i64)
    }

    fn render_status_panel(
        &self,
        session: &AgentSession,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = session.id;
        let heading = |title: String| {
            div()
                .text_size(sp(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_tertiary)
                .child(title)
        };
        let mut panel = div()
            .id("status-capsule-panel")
            .w(px(PANEL_WIDTH))
            .max_h(px(420.0))
            .overflow_y_scroll()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.raised)
            .shadow_lg()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation());

        if let Some(goal) = session.thread_goal.as_ref() {
            let color = super::goal_dialog::goal_status_color(goal.status, theme);
            let label =
                super::goal_dialog::goal_chip_label(goal, self.goal_live_elapsed_seconds(session));
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(heading(tr!("status.goal")))
                    .child(
                        div()
                            .id("status-capsule-goal")
                            .p(px(8.0))
                            .rounded(px(8.0))
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .cursor_default()
                            .hover(|row| row.bg(theme.overlay))
                            .child(div().text_size(sp(12.5)).text_color(color).child(label))
                            .child(
                                div()
                                    .text_size(sp(12.5))
                                    .line_height(sp(17.0))
                                    .text_color(theme.text_secondary)
                                    .line_clamp(3)
                                    .child(goal.objective.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_goal_dialog(session_id, None, false, cx);
                            })),
                    ),
            );
        }

        if !session.todos.is_empty() {
            let (done, total) = todo_progress(&session.todos);
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(heading(tr!("status.progress")))
                            .child(
                                div()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child(format!("{done}/{total}")),
                            ),
                    )
                    .child(super::todo_list::render_todo_list(
                        &session.todos,
                        12.5,
                        theme,
                    )),
            );
        }

        let background = self.live_background_work(session_id);
        if !background.is_empty() {
            let mut list = div().flex().flex_col().gap(px(2.0));
            for (index, item) in background.into_iter().enumerate() {
                let key = item.key.clone();
                list = list.child(
                    div()
                        .id(("status-capsule-background", index))
                        .h(px(26.0))
                        .px(px(6.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_default()
                        .hover(|row| row.bg(theme.overlay))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(sp(12.5))
                                .text_color(theme.text_secondary)
                                .child(item.title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(sp(12.0))
                                .text_color(work_status_color(item.status, *theme))
                                .child(work_status_label(item.status)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_background_work_surface(session_id, key.clone(), cx);
                        })),
                );
            }
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(heading(tr!("status.background")))
                    .child(list),
            );
        }

        // The latest turn that changed files, a click away from its diff.
        let latest_changes = session.turns.iter().rev().find_map(|turn| {
            turn.checkpoint
                .as_ref()
                .filter(|checkpoint| {
                    checkpoint.status == CheckpointStatus::Ready && !checkpoint.files.is_empty()
                })
                .map(|checkpoint| (turn.id, checkpoint))
        });
        if let Some((turn_id, checkpoint)) = latest_changes {
            let title = if checkpoint.files.len() == 1 {
                tr!("transcript.changed_file", count = checkpoint.files.len())
            } else {
                tr!("transcript.changed_files", count = checkpoint.files.len())
            };
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(heading(tr!("status.changes")))
                    .child(
                        div()
                            .id("status-capsule-changes")
                            .h(px(28.0))
                            .px(px(6.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .cursor_default()
                            .text_size(sp(12.5))
                            .hover(|row| row.bg(theme.overlay))
                            .child(icon("icons/file-diff.svg", 12.0, theme.text_tertiary))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.text_secondary)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(theme.success)
                                    .child(format!("+{}", checkpoint.additions)),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(theme.danger)
                                    .child(format!("-{}", checkpoint.deletions)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_turn_diff(turn_id, None, cx);
                            })),
                    ),
            );
        }
        panel.into_any_element()
    }
}
