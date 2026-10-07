//! Sidebar task rows.
//!
//! Fork file. A task is one line, after ZCode's task list: a status slot,
//! the title, and on the right the time since the last reply — or, while the
//! task waits on the person, a green 「等待确认」 tag. The slot shows the
//! one state that matters most: a failure, then an unread reply, then a
//! running turn, then a pin. Hovering a row swaps the time for its actions
//! (pin and archive; restore and delete in the archive), and each action is
//! also reachable by Tab. Where the task lives — its project and branch, the
//! old second line — is in the title's tooltip.
//!
//! Upstream marks a failed task with a bare red `×` that does nothing when
//! clicked — it reads as a close button and answers like a label, and the
//! failure's cause is nowhere near it. Here the mark is a circled `×` that
//! carries the failure's own words as a tooltip and, when clicked, opens the
//! task at the point where they were said.

use crate::ui::ActivationExt as _;

use super::sidebar::{
    CancelSessionRename, SESSION_RENAME_PARENT_CONTEXT, format_time_ago, localized_session_title,
    persisted_sidebar_branch_label, sidebar_session_selected,
};
use super::sidebar_rows::{
    SIDEBAR_LEADING_GAP, SIDEBAR_LEADING_SLOT, SIDEBAR_ROW_INSET, SIDEBAR_TASK_ROW_GAP,
    SIDEBAR_TASK_ROW_HEIGHT,
};
use super::sidebar_sections::{UNREAD_DOT_SIZE, unread_dot_color};
use super::transcript::format_working_elapsed;
use super::*;

/// How much of the failure to put in a tooltip.
const FAILURE_SUMMARY_CHARS: usize = 120;
/// One hover action's hit area.
const ROW_ACTION_SIZE: f32 = 22.0;

/// The failed turn's last words from the assistant — where the drivers put
/// a provider error or an exit reason — flattened to one line and cut to
/// tooltip length. `None` for a task that has not failed or said nothing.
pub(super) fn failure_summary(session: &AgentSession) -> Option<String> {
    if session.status != SessionStatus::Failed {
        return None;
    }
    let text = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| {
            message
                .display_content
                .as_deref()
                .unwrap_or(&message.content)
        })?;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(&collapsed, FAILURE_SUMMARY_CHARS))
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit).collect();
    cut.push('…');
    cut
}

/// The row's trailing time: how long the live turn has run, or how long ago
/// the agent last replied. A task that never replied shows nothing. The
/// spinner in the status slot already says it is working, so the running
/// form is only the duration.
pub(super) fn compact_task_time(session: &AgentSession, now: u64) -> Option<String> {
    if session.is_busy()
        && let Some(turn) = session
            .turns
            .last()
            .filter(|turn| turn.status == TurnStatus::Running)
    {
        return Some(format_working_elapsed(now.saturating_sub(turn.started_at)));
    }
    session
        .last_reply_at
        .map(|last_reply_at| format_time_ago(now.saturating_sub(last_reply_at)))
}

/// "Project · branch", or just the project when no branch is known.
fn location_label(place: String, branch: Option<&str>) -> String {
    match branch {
        Some(branch) => format!("{place} · {branch}"),
        None => place,
    }
}

/// The 「等待确认」 tag: ZCode's confirmation green, in both themes.
fn approval_tag_colors(theme: &Theme) -> (Hsla, Hsla) {
    if theme.is_dark {
        (gpui::rgba(0x46BF7229).into(), rgb(0x87D9A4).into())
    } else {
        (rgb(0xEAF7EE).into(), rgb(0x166B32).into())
    }
}

impl Waku {
    pub(super) fn render_task_row(&self, session_id: Uuid, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(session) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return div().into_any_element();
        };
        let marks = &self.task_marks.marks;
        let archived = marks.is_archived(session_id);
        let pinned = marks.is_pinned(session_id);
        let unread = marks.is_unread(session_id);
        let busy = session.is_busy();
        // While a fork page has the main column, no task is showing.
        let selected = !self.main_page_open()
            && sidebar_session_selected(
                self.state.selected_session,
                self.pending_session_activation
                    .map(|pending| pending.session_id),
                session_id,
            );
        let menu = self.menu_handle(format!("session-{session_id}"), cx);
        let row_focus = menu.trigger_focus_handle().clone();
        let keyboard_menu = menu.clone();
        let row_group = SharedString::from(format!("session-row-{session_id}"));

        let rename_input =
            (self.session_rename == Some(session_id)).then(|| self.session_rename_input.clone());
        let renaming = rename_input.is_some();
        let title = if let Some(rename_input) = rename_input {
            div()
                .id(SharedString::from(format!(
                    "session-rename-field-{session_id}"
                )))
                .key_context(SESSION_RENAME_PARENT_CONTEXT)
                .on_action(cx.listener(|this, _: &CancelSessionRename, window, cx| {
                    this.cancel_session_rename(window, cx);
                }))
                .h(px(22.0))
                .flex_1()
                .min_w_0()
                .px(px(4.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(theme.accent)
                .bg(theme.inset)
                .flex()
                .items_center()
                .text_size(sp(13.5))
                .text_color(theme.text)
                .child(rename_input)
                .into_any_element()
        } else {
            let waku = cx.entity().downgrade();
            div()
                .id(SharedString::from(format!("task-title-{session_id}")))
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .line_clamp(1)
                .text_overflow(gpui::TextOverflow::Truncate("...".into()))
                .text_size(sp(13.5))
                .text_color(theme.text)
                .when(unread, |title| title.font_weight(FontWeight::MEDIUM))
                .child(SharedString::from(localized_session_title(session)))
                // Built when shown, not on every frame.
                .tooltip(move |window, cx| {
                    let label = waku
                        .upgrade()
                        .and_then(|waku| waku.read(cx).task_location_label(session_id))
                        .unwrap_or_default();
                    Tooltip::new(label).build(window, cx)
                })
                .into_any_element()
        };

        let status = if session.status == SessionStatus::Waiting {
            // Several requests can wait at once; say how many.
            let waiting = self.runtimes.get(&session_id).map_or(0, |runtime| {
                runtime.pending_permissions.len()
                    + usize::from(runtime.pending_user_input.is_some())
                    + usize::from(runtime.pending_computer_approval.is_some())
            });
            let (background, foreground) = approval_tag_colors(&theme);
            Some(
                div()
                    .flex_none()
                    .h(px(20.0))
                    .px(px(7.0))
                    .rounded_full()
                    .bg(background)
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .text_size(sp(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(foreground)
                    .child(tr!("sidebar.awaiting_approval"))
                    .when(waiting > 1, |tag| tag.child(format!("· {waiting}"))),
            )
        } else {
            compact_task_time(session, unix_time()).map(|label| {
                div()
                    .flex_none()
                    .text_size(sp(12.0))
                    .text_color(if busy {
                        theme.text_tertiary
                    } else {
                        theme.text_ghost
                    })
                    .child(SharedString::from(label))
            })
        };

        let row = div()
            .id(SharedString::from(format!("session-{session_id}")))
            .group(row_group.clone())
            .w_full()
            .min_w_0()
            .h(px(SIDEBAR_TASK_ROW_HEIGHT))
            .pl(px(SIDEBAR_ROW_INSET))
            .pr(px(4.0))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .gap(px(SIDEBAR_LEADING_GAP))
            .cursor_default()
            .when(selected, |element| {
                element.bg(theme.sidebar_item_background)
            })
            .hover(|element| element.bg(theme.sidebar_item_background))
            .active(|element| element.bg(theme.sidebar_item_background))
            .child(self.render_task_status_slot(session, unread, pinned, &theme, cx))
            .child(title)
            .when(!renaming, |row| {
                row.children(status.map(|status| {
                    // The actions take the time's place on hover.
                    div()
                        .flex_none()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .group_hover(row_group.clone(), |style| style.w_0().opacity(0.0))
                        .child(status)
                }))
                .children(self.render_task_actions(session, archived, pinned, &row_group, cx))
            })
            .when(!renaming, |element| {
                element
                    .track_focus(&row_focus)
                    .tab_index(0)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let key = event.keystroke.key.as_str();
                        if matches!(key, "enter" | "space") {
                            this.select_session(session_id, cx);
                            cx.stop_propagation();
                        } else if key == "f10" && event.keystroke.modifiers.shift {
                            keyboard_menu.open_context_menu(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_session(session_id, cx);
                    }))
            });

        let row = if renaming {
            div()
                .w_full()
                .child(row)
                .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                    if this.session_rename == Some(session_id) {
                        this.commit_session_rename(cx);
                    }
                }))
                .into_any_element()
        } else {
            let waku = cx.entity().downgrade();
            context_menu(
                div().w_full().child(row),
                SharedString::from(format!("session-menu-{session_id}")),
                &menu,
                move |_| task_menu_items(waku.clone(), session_id, archived, pinned, unread, busy),
            )
        };

        div()
            .w_full()
            .pb(px(SIDEBAR_TASK_ROW_GAP))
            .child(row)
            .into_any_element()
    }

    /// The one state worth a glance, most urgent first: a failure, an
    /// unread reply, a running turn, a pin.
    fn render_task_status_slot(
        &self,
        session: &AgentSession,
        unread: bool,
        pinned: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let slot = div()
            .flex_none()
            .w(px(SIDEBAR_LEADING_SLOT))
            .h(px(16.0))
            .flex()
            .items_center()
            .justify_center();
        if session.status == SessionStatus::Failed {
            return slot.children(self.render_task_failure_badge(session, cx));
        }
        if unread {
            return slot.child(
                div()
                    .id(SharedString::from(format!("task-unread-{}", session.id)))
                    .size(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(Tooltip::text(tr!("sidebar.unread")))
                    .child(
                        div()
                            .size(px(UNREAD_DOT_SIZE))
                            .rounded_full()
                            .bg(unread_dot_color(theme)),
                    ),
            );
        }
        if matches!(
            session.status,
            SessionStatus::Connecting | SessionStatus::Working
        ) {
            return slot.child(motion::spin_slow(icon(
                "icons/loader-circle.svg",
                12.0,
                status_color(theme, session.status),
            )));
        }
        if pinned {
            return slot.child(icon("icons/pin.svg", 12.0, theme.text_tertiary));
        }
        slot
    }

    /// Pin and archive, or restore and delete in the archive. Each takes no
    /// room until the row is hovered or the button itself has keyboard focus.
    fn render_task_actions(
        &self,
        session: &AgentSession,
        archived: bool,
        pinned: bool,
        row_group: &SharedString,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let theme = Theme::current(cx);
        let session_id = session.id;
        if archived {
            return vec![
                row_action(
                    format!("task-unarchive-{session_id}"),
                    "icons/archive-restore.svg",
                    tr!("sidebar.unarchive"),
                    row_group,
                    true,
                    &theme,
                )
                .on_activation(cx, move |this, window, cx| {
                    this.leave_task_row(session_id, window, cx);
                    this.unarchive_task(session_id, cx);
                }),
                row_action(
                    format!("task-delete-{session_id}"),
                    "icons/trash.svg",
                    tr!("sidebar.delete"),
                    row_group,
                    true,
                    &theme,
                )
                .on_activation(cx, move |this, _, cx| {
                    this.confirm_delete_task(session_id, cx);
                }),
            ];
        }
        let busy = session.is_busy();
        vec![
            row_action(
                format!("task-pin-{session_id}"),
                if pinned {
                    "icons/pin-off.svg"
                } else {
                    "icons/pin.svg"
                },
                if pinned {
                    tr!("sidebar.unpin")
                } else {
                    tr!("sidebar.pin")
                },
                row_group,
                true,
                &theme,
            )
            .on_activation(cx, move |this, _, cx| this.toggle_task_pin(session_id, cx)),
            row_action(
                format!("task-archive-{session_id}"),
                "icons/archive.svg",
                if busy {
                    tr!("sidebar.archive_running")
                } else {
                    tr!("sidebar.archive")
                },
                row_group,
                !busy,
                &theme,
            )
            .when(!busy, |button| {
                button.on_activation(cx, move |this, window, cx| {
                    this.leave_task_row(session_id, window, cx);
                    this.archive_task(session_id, cx);
                })
            }),
        ]
    }

    /// Where a task lives, for its title's tooltip: the project (or "no
    /// project") and the branch when one is known.
    pub(super) fn task_location_label(&self, session_id: Uuid) -> Option<String> {
        let session = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)?;
        let project = self
            .state
            .projects
            .iter()
            .find(|project| project.id == session.project_id);
        let place = project
            .filter(|project| !project.is_projectless())
            .map(Project::display_name)
            .unwrap_or_else(|| tr!("project.no_project_name"));
        let branch = persisted_sidebar_branch_label(&session.workspace)
            .map(str::to_owned)
            .or_else(|| {
                if !matches!(&session.workspace, SessionWorkspace::Local) {
                    return None;
                }
                let project = project?;
                self.sidebar_branch_labels
                    .borrow()
                    .get(&project.path)
                    .map(ToString::to_string)
            });
        Some(location_label(place, branch.as_deref()))
    }

    /// The failure mark for a task row: tooltip with the cause, click to
    /// open the task where the cause was reported.
    pub(super) fn render_task_failure_badge(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let mut marks = Vec::new();
        if session.status != SessionStatus::Failed {
            return marks;
        }
        let theme = Theme::current(cx);
        let session_id = session.id;
        let summary = failure_summary(session).unwrap_or_else(|| tr!("session.failed"));
        marks.push(
            div()
                .id(SharedString::from(format!("task-failed-{session_id}")))
                .flex_none()
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(4.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .hover(|style| style.bg(theme.overlay))
                .tooltip(Tooltip::text(SharedString::from(summary)))
                .child(icon("icons/circle-x.svg", 12.0, theme.danger))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.select_session(session_id, cx);
                    this.scroll_transcript_to_bottom(cx);
                })),
        );
        marks
    }
}

fn task_menu_items(
    waku: WeakEntity<Waku>,
    session_id: Uuid,
    archived: bool,
    pinned: bool,
    unread: bool,
    busy: bool,
) -> Vec<MenuItem> {
    let on = |action: fn(&mut Waku, Uuid, &mut Window, &mut Context<Waku>)| {
        let waku = waku.clone();
        move |window: &mut Window, cx: &mut App| {
            let _ = waku.update(cx, |waku, cx| action(waku, session_id, window, cx));
        }
    };
    if archived {
        return vec![
            MenuItem::new(
                tr!("sidebar.unarchive"),
                on(|waku, id, window, cx| {
                    waku.leave_task_row(id, window, cx);
                    waku.unarchive_task(id, cx);
                }),
            ),
            MenuItem::Separator,
            MenuItem::new(
                tr!("sidebar.delete"),
                on(|waku, id, _, cx| waku.confirm_delete_task(id, cx)),
            ),
        ];
    }
    let mut items = vec![
        MenuItem::new(
            if pinned {
                tr!("sidebar.unpin")
            } else {
                tr!("sidebar.pin")
            },
            on(|waku, id, _, cx| waku.toggle_task_pin(id, cx)),
        ),
        MenuItem::new(
            tr!("common.rename"),
            on(|waku, id, window, cx| waku.begin_session_rename(id, window, cx)),
        ),
    ];
    if !unread {
        items.push(MenuItem::new(
            tr!("sidebar.mark_unread"),
            on(|waku, id, _, cx| waku.mark_task_unread(id, cx)),
        ));
    }
    items.push(MenuItem::Separator);
    items.push(
        MenuItem::new(
            tr!("sidebar.archive"),
            on(|waku, id, window, cx| {
                waku.leave_task_row(id, window, cx);
                waku.archive_task(id, cx);
            }),
        )
        .disabled(busy),
    );
    items
}

/// A row action: no room until the row is hovered or it has keyboard focus.
/// A disabled one shows dimmed on hover so its tooltip can say why.
fn row_action(
    id: String,
    icon_path: &'static str,
    tooltip: String,
    row_group: &SharedString,
    enabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let shown_opacity = if enabled { 1.0 } else { 0.4 };
    div()
        .id(SharedString::from(id))
        .flex_none()
        .w_0()
        .h(px(ROW_ACTION_SIZE))
        .overflow_hidden()
        .rounded(px(5.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .opacity(0.0)
        .group_hover(row_group.clone(), move |style| {
            style.w(px(ROW_ACTION_SIZE)).opacity(shown_opacity)
        })
        .when(enabled, |button| {
            button
                .tab_index(0)
                .focus_visible(|style| {
                    style
                        .w(px(ROW_ACTION_SIZE))
                        .opacity(1.0)
                        .border_1()
                        .border_color(theme.accent)
                })
                .hover(|style| style.bg(theme.overlay))
        })
        .tooltip(Tooltip::text(tooltip))
        .child(icon(icon_path, 13.0, theme.text_tertiary))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_session_with(messages: &[(MessageRole, &str)]) -> AgentSession {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Claude);
        session.status = SessionStatus::Failed;
        for (role, content) in messages {
            session.push_message(*role, (*content).to_owned());
        }
        session
    }

    #[test]
    fn failure_summary_takes_the_last_assistant_words_flattened() {
        let session = failed_session_with(&[
            (MessageRole::User, "do the thing"),
            (
                MessageRole::Assistant,
                "Error:   provider\nexited  before   a response",
            ),
        ]);
        assert_eq!(
            failure_summary(&session).as_deref(),
            Some("Error: provider exited before a response")
        );
    }

    #[test]
    fn failure_summary_is_cut_to_tooltip_length() {
        let long = "x".repeat(300);
        let session = failed_session_with(&[(MessageRole::Assistant, &long)]);
        let summary = failure_summary(&session).unwrap();
        assert_eq!(summary.chars().count(), FAILURE_SUMMARY_CHARS + 1);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn failure_summary_needs_a_failed_task_with_something_said() {
        let mut idle = failed_session_with(&[(MessageRole::Assistant, "fine")]);
        idle.status = SessionStatus::Idle;
        assert!(failure_summary(&idle).is_none());
        let silent = failed_session_with(&[(MessageRole::User, "hello")]);
        assert!(failure_summary(&silent).is_none());
    }

    #[test]
    fn task_time_is_the_running_duration_or_the_last_reply() {
        let mut session = AgentSession::new(Uuid::new_v4(), ProviderKind::Native);
        assert_eq!(compact_task_time(&session, 1_000), None);

        session.last_reply_at = Some(1_000 - 5 * 60);
        assert_eq!(
            compact_task_time(&session, 1_000),
            Some(format_time_ago(5 * 60))
        );

        session.begin_turn("go");
        session.status = SessionStatus::Working;
        let started_at = session.turns.last().unwrap().started_at;
        assert_eq!(
            compact_task_time(&session, started_at + 42),
            Some(format_working_elapsed(42))
        );
    }

    #[test]
    fn location_label_names_the_branch_when_known() {
        assert_eq!(
            location_label("waku".to_owned(), Some("main")),
            "waku · main"
        );
        assert_eq!(location_label("waku".to_owned(), None), "waku");
    }
}
