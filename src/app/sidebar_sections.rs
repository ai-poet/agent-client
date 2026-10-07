//! Sidebar group headers, "show more" rows and empty states.
//!
//! Fork file. Sections (pinned, 项目, 任务, the archive) and calendar groups
//! get a quiet caption whose fold chevron shows on hover; a project gets a
//! full row with its folder, like the tasks under it. A folded group keeps a
//! spinner while a task inside runs and a dot while one is unread, so
//! folding never hides that something needs a look. Rows that stand in for a
//! task title start where the titles do.

use super::sidebar::SidebarGroup;
use super::sidebar_rows::{
    GroupBadge, SIDEBAR_EMPTY_ROW_HEIGHT, SIDEBAR_HEADER_BOTTOM_GAP, SIDEBAR_LEADING_GAP,
    SIDEBAR_LEADING_SLOT, SIDEBAR_PROJECT_HEADER_HEIGHT, SIDEBAR_ROW_INSET,
    SIDEBAR_SECTION_HEADER_HEIGHT, SIDEBAR_SHOW_MORE_ROW_HEIGHT, SIDEBAR_TITLE_INSET, SidebarEmpty,
};
use super::*;

/// The unread dot: ZCode's sky, which reads on both themes and never passes
/// for the accent, a warning or a failure.
pub(super) fn unread_dot_color(theme: &Theme) -> Hsla {
    if theme.is_dark {
        rgb(0x38BDF8).into()
    } else {
        rgb(0x0EA5E9).into()
    }
}

pub(super) const UNREAD_DOT_SIZE: f32 = 6.0;

impl Waku {
    /// A section or calendar group caption. The archive's caption does not
    /// fold: it is the whole view.
    pub(super) fn render_sidebar_section_header(
        &self,
        group: SidebarGroup,
        badge: GroupBadge,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let foldable = group != SidebarGroup::Archived;
        let collapsed = self.sidebar_collapsed_groups.contains(&group);
        let key = group.element_key();
        let group_name = SharedString::from(format!("sidebar-section-{key}"));
        let label = match group {
            SidebarGroup::Updated(date_group) => date_group.label(),
            SidebarGroup::Pinned => tr!("sidebar.pinned"),
            SidebarGroup::Projects => tr!("sidebar.projects"),
            SidebarGroup::Projectless => tr!("sidebar.tasks"),
            SidebarGroup::Archived => tr!("sidebar.archived_tasks"),
            SidebarGroup::Project(project_id) => self.sidebar_project_name(project_id),
        };
        let hover_text = theme.text_secondary;
        let chevron = foldable.then(|| {
            icon("icons/chevron-down.svg", 12.0, theme.text_tertiary)
                .when(collapsed, |icon| {
                    icon.with_transformation(gpui::Transformation::rotate(gpui::percentage(0.75)))
                })
                .invisible()
                .group_hover(group_name.clone(), |icon| icon.visible())
        });
        let action = match group {
            SidebarGroup::Projectless => {
                Some(self.render_group_compose(group, group_name.clone(), cx))
            }
            SidebarGroup::Projects => Some(self.render_add_project(cx)),
            _ => None,
        };

        let header = div()
            .id(SharedString::from(format!("sidebar-section-{key}")))
            .group(group_name)
            .w_full()
            .h(px(SIDEBAR_SECTION_HEADER_HEIGHT))
            .pl(px(SIDEBAR_ROW_INSET))
            .pr(px(4.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(sp(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_tertiary)
            .cursor_default()
            .child(div().min_w_0().truncate().child(label))
            .children(chevron)
            .children(self.render_group_badge(&key, badge, &theme))
            .child(div().flex_1())
            .children(action)
            .when(foldable, |header| {
                let focus = self
                    .sidebar_group_header_focuses
                    .borrow_mut()
                    .entry(group)
                    .or_insert_with(|| cx.focus_handle())
                    .clone();
                header
                    .track_focus(&focus)
                    .tab_index(0)
                    .tab_stop(true)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .hover(move |style| style.text_color(hover_text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_sidebar_group(group, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        fold_keys(this, group, collapsed, event, cx);
                    }))
            });

        div()
            .w_full()
            .pb(px(SIDEBAR_HEADER_BOTTOM_GAP))
            .child(header)
    }

    /// A project group: a row with its folder, the name, what it holds while
    /// folded, and a new-task button on hover.
    pub(super) fn render_sidebar_project_header(
        &self,
        group: SidebarGroup,
        badge: GroupBadge,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let collapsed = self.sidebar_collapsed_groups.contains(&group);
        let key = group.element_key();
        let group_name = SharedString::from(format!("sidebar-group-header-{key}"));
        let focus = self
            .sidebar_group_header_focuses
            .borrow_mut()
            .entry(group)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let name = match group {
            SidebarGroup::Project(project_id) => self.sidebar_project_name(project_id),
            _ => tr!("project.no_project_name"),
        };
        let folder = if collapsed {
            "icons/folder.svg"
        } else {
            "icons/folder-open.svg"
        };

        let header = div()
            .id(SharedString::from(format!("sidebar-group-toggle-{key}")))
            .track_focus(&focus)
            .tab_index(0)
            .tab_group()
            .tab_stop(true)
            .group(group_name.clone())
            .w_full()
            .h(px(SIDEBAR_PROJECT_HEADER_HEIGHT))
            .pl(px(SIDEBAR_ROW_INSET))
            .pr(px(4.0))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .gap(px(SIDEBAR_LEADING_GAP))
            .cursor_default()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .hover(|style| style.bg(theme.sidebar_item_background))
            .active(|style| style.bg(theme.overlay_strong))
            .child(
                div()
                    .flex_none()
                    .w(px(SIDEBAR_LEADING_SLOT))
                    .flex()
                    .justify_center()
                    .child(icon(folder, 14.0, theme.text_tertiary)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(sp(13.5))
                    .text_color(theme.text_secondary)
                    .child(name),
            )
            .children(self.render_group_badge(&key, badge, &theme))
            .child(self.render_group_compose(group, group_name, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_sidebar_group(group, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                fold_keys(this, group, collapsed, event, cx);
            }));

        div()
            .w_full()
            .pb(px(SIDEBAR_HEADER_BOTTOM_GAP))
            .child(header)
    }

    pub(super) fn render_sidebar_show_more(
        &self,
        group: SidebarGroup,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let key = group.element_key();
        let focus = self
            .sidebar_show_more_focuses
            .borrow_mut()
            .entry(group)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let button = div()
            .id(SharedString::from(format!("sidebar-show-more-{key}")))
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(true)
            .flex_none()
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_tertiary)
            .focus_visible(|style| style.text_color(theme.text))
            .hover(|style| style.text_color(theme.text))
            .child(tr!("sidebar.show_more"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.show_more_project_sessions(group, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.show_more_project_sessions(group, cx);
                    cx.stop_propagation();
                }
            }));

        div()
            .w_full()
            .h(px(SIDEBAR_SHOW_MORE_ROW_HEIGHT))
            .pl(px(SIDEBAR_TITLE_INSET))
            .flex()
            .items_center()
            .child(button)
    }

    pub(super) fn render_sidebar_empty(&self, kind: SidebarEmpty, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        div()
            .w_full()
            .h(px(SIDEBAR_EMPTY_ROW_HEIGHT))
            .pl(px(SIDEBAR_TITLE_INSET))
            .flex()
            .items_center()
            .text_size(sp(12.5))
            .text_color(theme.text_ghost)
            .child(match kind {
                SidebarEmpty::NoProjects => tr!("sidebar.no_projects"),
                SidebarEmpty::NoTasks => tr!("sidebar.no_tasks"),
                SidebarEmpty::NoArchived => tr!("sidebar.no_archived"),
            })
    }

    fn sidebar_project_name(&self, project_id: Uuid) -> String {
        self.state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .map(Project::display_name)
            .unwrap_or_else(|| tr!("project.no_project_name"))
    }

    /// What a folded group still reports: a spinner while a task inside runs,
    /// a dot while one is unread. Each says what it means in a tooltip.
    fn render_group_badge(
        &self,
        key: &SharedString,
        badge: GroupBadge,
        theme: &Theme,
    ) -> Option<Div> {
        if !badge.running && !badge.unread {
            return None;
        }
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.0))
                .when(badge.running, |element| {
                    element.child(
                        div()
                            .id(SharedString::from(format!("sidebar-group-running-{key}")))
                            .flex()
                            .items_center()
                            .tooltip(Tooltip::text(tr!("sidebar.group_running")))
                            .child(motion::spin_slow(icon(
                                "icons/loader-circle.svg",
                                12.0,
                                theme.accent,
                            ))),
                    )
                })
                .when(badge.unread, |element| {
                    element.child(
                        div()
                            .id(SharedString::from(format!("sidebar-group-unread-{key}")))
                            .size(px(12.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .tooltip(Tooltip::text(tr!("sidebar.group_unread")))
                            .child(
                                div()
                                    .size(px(UNREAD_DOT_SIZE))
                                    .rounded_full()
                                    .bg(unread_dot_color(theme)),
                            ),
                    )
                }),
        )
    }

    /// The new-task button a group shows on hover or keyboard focus.
    fn render_group_compose(
        &self,
        group: SidebarGroup,
        group_name: SharedString,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let focus = self
            .sidebar_group_compose_focuses
            .borrow_mut()
            .entry(group)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        hover_action(
            div()
                .id(SharedString::from(format!(
                    "sidebar-group-compose-{}",
                    group.element_key()
                )))
                .track_focus(&focus),
            group_name,
            &theme,
        )
        .tooltip(Tooltip::text(tr!("menu.new_task")))
        .child(icon("icons/compose.svg", 14.0, theme.text_secondary))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.open_new_task_for_sidebar_group(group, window, cx);
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.open_new_task_for_sidebar_group(group, window, cx);
                cx.stop_propagation();
            }
        }))
        .into_wrapper()
    }

    /// Adding a project, on the section that lists them.
    /// Always shown, unlike the per-group new-task buttons: it is the way in
    /// for a project that is not listed yet, so it must not wait for a hover.
    fn render_add_project(&self, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        let focus = self
            .sidebar_group_compose_focuses
            .borrow_mut()
            .entry(SidebarGroup::Projects)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        div()
            .id("sidebar-add-project")
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(true)
            .w(px(20.0))
            .h(px(22.0))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .hover(|style| style.bg(theme.overlay))
            .active(|style| style.bg(theme.overlay_strong))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .tooltip(Tooltip::text(tr!("project.new_project")))
        .child(icon("icons/folder-new.svg", 14.0, theme.text_secondary))
        .on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.add_project(cx);
        }))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.add_project(cx);
                cx.stop_propagation();
            }
        }))
        .into_wrapper()
    }
}

/// Enter or Space folds or unfolds a group; Left folds it and Right unfolds.
fn fold_keys(
    waku: &mut Waku,
    group: SidebarGroup,
    collapsed: bool,
    event: &KeyDownEvent,
    cx: &mut Context<Waku>,
) {
    match event.keystroke.key.as_str() {
        "enter" | "space" => {
            waku.toggle_sidebar_group(group, cx);
            cx.stop_propagation();
        }
        "left" if !collapsed => {
            waku.set_sidebar_group_collapsed(group, true, cx);
            cx.stop_propagation();
        }
        "right" if collapsed => {
            waku.set_sidebar_group_collapsed(group, false, cx);
            cx.stop_propagation();
        }
        _ => {}
    }
}

/// A header button that takes no room until its header is hovered or the
/// button itself has keyboard focus.
fn hover_action(button: Stateful<Div>, group_name: SharedString, theme: &Theme) -> Stateful<Div> {
    button
        .tab_index(0)
        .tab_stop(true)
        .w_0()
        .h(px(22.0))
        .overflow_hidden()
        .rounded(px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .opacity(0.0)
        .group_hover(group_name, |style| style.w(px(20.0)).opacity(1.0))
        .focus_visible(|style| {
            style
                .w(px(20.0))
                .opacity(1.0)
                .border_1()
                .border_color(theme.accent)
        })
        .hover(|style| style.bg(theme.overlay))
        .active(|style| style.bg(theme.overlay_strong))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

trait IntoWrapper {
    fn into_wrapper(self) -> Div;
}

impl IntoWrapper for Stateful<Div> {
    /// Hover buttons sit in a fixed 20px slot so the header does not shift
    /// when one appears.
    fn into_wrapper(self) -> Div {
        div()
            .w(px(20.0))
            .h(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .child(self)
    }
}
