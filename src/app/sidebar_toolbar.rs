//! The sidebar toolbar: the view switch, expand/collapse all, ordering and
//! the archive.
//!
//! Fork file, after ZCode's task toolbar. A two-segment pill switches
//! between the project view and the timeline (it stands for the persisted
//! grouping preference), one button folds or unfolds every group, a menu
//! picks the ordering, and the archive switch swaps the lists for the
//! archived tasks. Adding a project lives on the 「项目」 section header,
//! where the projects are.

use std::cell::{Cell, RefCell};

use crate::ui::ActivationExt as _;

use super::sidebar::SidebarGroup;
use super::sidebar_rows::{SIDEBAR_TOOLBAR_HEIGHT, SidebarView, ToggleAll};
use super::*;

pub(super) struct SidebarToolbarState {
    /// Showing the archive instead of the task lists. Runtime-only, like the
    /// folded groups: every launch opens on the tasks.
    pub(super) archived_open: bool,
    /// What the expand/collapse-all control offers, from the latest rows.
    toggle_all: Cell<ToggleAll>,
    /// The groups collapse-all folds, from the latest rows.
    toggle_targets: RefCell<Vec<SidebarGroup>>,
    /// The pill is one tab stop with arrow keys inside, and its row is
    /// virtualized, so its focus has to outlive the row.
    view_focus: FocusHandle,
}

impl SidebarToolbarState {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            archived_open: false,
            toggle_all: Cell::new(ToggleAll::Hidden),
            toggle_targets: RefCell::new(Vec::new()),
            view_focus: cx.focus_handle(),
        }
    }

    /// Called whenever the rows are rebuilt.
    pub(super) fn remember_layout(&self, toggle_all: ToggleAll, targets: Vec<SidebarGroup>) {
        self.toggle_all.set(toggle_all);
        *self.toggle_targets.borrow_mut() = targets;
    }
}

impl Waku {
    pub(super) fn sidebar_view(&self) -> SidebarView {
        if self.sidebar_toolbar.archived_open {
            return SidebarView::Archived;
        }
        match self.state.sidebar_grouping {
            SidebarGrouping::Project => SidebarView::ByProject,
            SidebarGrouping::Updated => SidebarView::Timeline,
        }
    }

    /// Picking a view also leaves the archive.
    fn show_sidebar_view(&mut self, grouping: SidebarGrouping, cx: &mut Context<Self>) {
        if self.sidebar_toolbar.archived_open {
            self.sidebar_toolbar.archived_open = false;
            self.sidebar_rows_fingerprint.set(None);
            self.scroll_sidebar_to_top();
            cx.notify();
        }
        self.set_sidebar_grouping(grouping, cx);
    }

    fn toggle_archived_view(&mut self, cx: &mut Context<Self>) {
        self.sidebar_toolbar.archived_open = !self.sidebar_toolbar.archived_open;
        // The archive opens on its first page each time.
        self.sidebar_project_reveal_counts
            .remove(&SidebarGroup::Archived);
        self.sidebar_rows_fingerprint.set(None);
        self.scroll_sidebar_to_top();
        cx.notify();
    }

    fn scroll_sidebar_to_top(&self) {
        self.sidebar_list_state.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: Pixels::ZERO,
        });
    }

    /// Fold every project (or every date group), or unfold everything,
    /// sections included.
    pub(super) fn set_all_sidebar_groups_collapsed(
        &mut self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        // The targets come from the latest rows; rebuild them first in case
        // the sidebar has not drawn since they changed (it may be hidden).
        self.refresh_sidebar_rows();
        let targets = self.sidebar_toolbar.toggle_targets.borrow().clone();
        let mut changed = false;
        if collapsed {
            for group in targets {
                changed |= self.sidebar_collapsed_groups.insert(group);
                changed |= self.sidebar_project_reveal_counts.remove(&group).is_some();
            }
        } else {
            let sections = [
                SidebarGroup::Pinned,
                SidebarGroup::Projects,
                SidebarGroup::Projectless,
            ];
            for group in targets.into_iter().chain(sections) {
                changed |= self.sidebar_collapsed_groups.remove(&group);
            }
        }
        if changed {
            self.sidebar_rows_fingerprint.set(None);
            cx.notify();
        }
    }

    pub(super) fn render_sidebar_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        let archived = self.sidebar_toolbar.archived_open;
        let grouping = self.state.sidebar_grouping;
        let toggle_all = self.sidebar_toolbar.toggle_all.get();

        let pill = div()
            .id("sidebar-view-switch")
            .track_focus(&self.sidebar_toolbar.view_focus)
            .tab_index(0)
            .flex_none()
            .min_w_0()
            .h(px(28.0))
            .p(px(2.0))
            .rounded_full()
            .bg(theme.overlay_strong)
            .flex()
            .items_center()
            .gap(px(2.0))
            .cursor_default()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .child(self.render_view_segment(
                "sidebar-view-project",
                tr!("sidebar.view_by_project"),
                !archived && grouping == SidebarGrouping::Project,
                SidebarGrouping::Project,
                &theme,
                cx,
            ))
            .child(self.render_view_segment(
                "sidebar-view-timeline",
                tr!("sidebar.view_timeline"),
                !archived && grouping == SidebarGrouping::Updated,
                SidebarGrouping::Updated,
                &theme,
                cx,
            ))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let target = match event.keystroke.key.as_str() {
                    "left" | "home" => SidebarGrouping::Project,
                    "right" | "end" => SidebarGrouping::Updated,
                    "enter" | "space" => match this.state.sidebar_grouping {
                        SidebarGrouping::Project => SidebarGrouping::Updated,
                        SidebarGrouping::Updated => SidebarGrouping::Project,
                    },
                    _ => return,
                };
                this.show_sidebar_view(target, cx);
                cx.stop_propagation();
            }));

        let toggle = (!archived && toggle_all != ToggleAll::Hidden).then(|| {
            let collapse = toggle_all == ToggleAll::CollapseAll;
            toolbar_button(
                "sidebar-toggle-all",
                if collapse {
                    "icons/chevrons-down-up.svg"
                } else {
                    "icons/chevrons-up-down.svg"
                },
                if collapse {
                    tr!("sidebar.collapse_all")
                } else {
                    tr!("sidebar.expand_all")
                },
                false,
                &theme,
            )
            .on_activation(cx, move |this, _, cx| {
                this.set_all_sidebar_groups_collapsed(collapse, cx);
            })
        });

        let menu = self.menu_handle("sidebar-options", cx);
        let menu_open = menu.is_open();
        let weak = cx.entity().downgrade();
        let ordering = self.state.sidebar_ordering;
        let options = dropdown_menu(
            toolbar_button(
                "sidebar-options",
                "icons/list-filter.svg",
                tr!("sidebar.options"),
                menu_open,
                &theme,
            ),
            "sidebar-options-menu",
            &menu,
            MenuAlign::BelowLeft,
            move |_| {
                let newest_weak = weak.clone();
                let oldest_weak = weak.clone();
                vec![
                    MenuItem::Header(tr!("sidebar.ordering").into()),
                    MenuItem::new(tr!("sidebar.ordering_newest"), move |_, cx| {
                        let _ = newest_weak.update(cx, |this, cx| {
                            this.set_sidebar_ordering(SidebarOrdering::Newest, cx);
                        });
                    })
                    .selected(ordering == SidebarOrdering::Newest),
                    MenuItem::new(tr!("sidebar.ordering_oldest"), move |_, cx| {
                        let _ = oldest_weak.update(cx, |this, cx| {
                            this.set_sidebar_ordering(SidebarOrdering::Oldest, cx);
                        });
                    })
                    .selected(ordering == SidebarOrdering::Oldest),
                ]
            },
        );

        let archive = toolbar_button(
            "sidebar-archive",
            if archived {
                "icons/x.svg"
            } else {
                "icons/archive.svg"
            },
            if archived {
                tr!("common.close")
            } else {
                tr!("sidebar.archived")
            },
            archived,
            &theme,
        )
        .on_activation(cx, |this, _, cx| this.toggle_archived_view(cx));

        div()
            .w_full()
            .h(px(SIDEBAR_TOOLBAR_HEIGHT))
            .pl(px(2.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(pill)
                    .children(toggle),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(options)
                    .child(archive),
            )
    }

    fn render_view_segment(
        &self,
        id: &'static str,
        label: String,
        active: bool,
        target: SidebarGrouping,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let text = theme.text;
        div()
            .id(id)
            .min_w_0()
            .h(px(24.0))
            .px(px(9.0))
            .rounded_full()
            .flex()
            .items_center()
            .text_size(sp(12.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_default()
            .when(active, |segment| {
                segment
                    .bg(theme.composer)
                    .border_1()
                    .border_color(theme.border)
                    .text_color(theme.text)
            })
            .when(!active, |segment| {
                segment
                    .text_color(theme.text_tertiary)
                    .hover(move |style| style.text_color(text))
            })
            // A narrow sidebar cuts the label rather than the buttons beside it.
            .child(div().min_w_0().truncate().child(label))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.show_sidebar_view(target, cx);
            }))
    }
}

/// A 24px icon button in the toolbar, reachable by Tab.
fn toolbar_button(
    id: &'static str,
    icon_path: &'static str,
    tooltip: String,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .flex_none()
        .w(px(24.0))
        .h(px(24.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .when(active, |element| element.bg(theme.overlay_strong))
        .hover(|element| element.bg(theme.overlay))
        .active(|element| element.bg(theme.overlay_strong))
        .tooltip(Tooltip::text(tooltip))
        .child(icon(icon_path, 14.0, theme.text_secondary))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}
