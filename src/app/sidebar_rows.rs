//! The task sidebar's rows, built from plain values.
//!
//! Fork file. The sidebar shows the same tasks three ways, after ZCode's
//! task list:
//!
//! - **By project**: a 「项目」 section with one group per project, each
//!   listing its first five tasks with more revealed five at a time, then a
//!   「任务」 section for the tasks that belong to no project.
//! - **Timeline**: the tasks under calendar headers (today, yesterday, …).
//! - **Archived**: every archived task in one list, newest archive first.
//!
//! Outside the archive, pinned tasks leave their usual place for a
//! 「已置顶」 section on top — moved, never repeated, because a row's focus
//! and context menu are keyed by the task id and two rows would share them —
//! and running tasks come first wherever they are listed.
//!
//! Nothing here touches GPUI or the app state, so every rule is tested on
//! plain values; `sidebar.rs` gathers the inputs and renders the rows.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use sub2api::task_marks::TaskMarks;
use uuid::Uuid;

use crate::persistence::SidebarOrdering;

use super::sidebar::{SessionDateGroup, SidebarGroup, SidebarRow};

/// The view switch, the expand/collapse-all control and the archive switch.
pub(super) const SIDEBAR_TOOLBAR_HEIGHT: f32 = 36.0;
/// A project group's header: a full row, like the tasks under it.
pub(super) const SIDEBAR_PROJECT_HEADER_HEIGHT: f32 = 32.0;
/// A section or date header: a caption above its rows.
pub(super) const SIDEBAR_SECTION_HEADER_HEIGHT: f32 = 28.0;
pub(super) const SIDEBAR_HEADER_BOTTOM_GAP: f32 = 2.0;
/// One task, on a single line.
pub(super) const SIDEBAR_TASK_ROW_HEIGHT: f32 = 32.0;
pub(super) const SIDEBAR_TASK_ROW_GAP: f32 = 2.0;
pub(super) const SIDEBAR_SHOW_MORE_ROW_HEIGHT: f32 = 30.0;
pub(super) const SIDEBAR_EMPTY_ROW_HEIGHT: f32 = 32.0;
pub(super) const SIDEBAR_GROUP_SPACER_HEIGHT: f32 = 10.0;
/// Left padding of a task or project row.
pub(super) const SIDEBAR_ROW_INSET: f32 = 10.0;
/// The status slot before a task title, and the folder before a project name.
pub(super) const SIDEBAR_LEADING_SLOT: f32 = 16.0;
pub(super) const SIDEBAR_LEADING_GAP: f32 = 6.0;
/// Where task titles and project names start, so the rows that stand in for
/// a title (show more, empty) line up with them.
pub(super) const SIDEBAR_TITLE_INSET: f32 =
    SIDEBAR_ROW_INSET + SIDEBAR_LEADING_SLOT + SIDEBAR_LEADING_GAP;

/// Tasks a project lists before "show more", and how many each reveal adds.
const PROJECT_PAGE: usize = 5;
/// The same for the flat lists: pinned, project-less tasks, the archive.
const FLAT_PAGE: usize = 20;

/// How many more tasks one "show more" reveals in this group.
pub(super) fn reveal_batch(group: SidebarGroup) -> usize {
    match group {
        SidebarGroup::Project(_) => PROJECT_PAGE,
        _ => FLAT_PAGE,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SidebarView {
    ByProject,
    Timeline,
    Archived,
}

/// What a folded group still says about the tasks inside it.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct GroupBadge {
    pub running: bool,
    pub unread: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SidebarEmpty {
    NoProjects,
    NoTasks,
    NoArchived,
}

/// What the toolbar's expand/collapse-all control offers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ToggleAll {
    /// Nothing to fold.
    #[default]
    Hidden,
    CollapseAll,
    ExpandAll,
}

/// One started task, reduced to what ordering and grouping read.
#[derive(Clone, Copy, Debug)]
pub(super) struct TaskEntry {
    pub id: Uuid,
    pub project_id: Uuid,
    /// Last reply, or creation for a task that never had one.
    pub timestamp: u64,
    /// Connecting, working or waiting on the person.
    pub running: bool,
    /// Lives under the project-less workspace root.
    pub projectless: bool,
    pub date_group: SessionDateGroup,
}

/// A project the person opened (not a project-less workspace).
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectEntry {
    pub id: Uuid,
    pub created_at: u64,
}

pub(super) struct SidebarRowInputs<'a> {
    pub view: SidebarView,
    pub ordering: SidebarOrdering,
    /// Date groups in display order (newest first unless ordering oldest).
    pub date_order: [SessionDateGroup; 6],
    pub entries: &'a [TaskEntry],
    /// In the order the app keeps them.
    pub projects: &'a [ProjectEntry],
    pub selected_project: Option<Uuid>,
    pub marks: &'a TaskMarks,
    pub collapsed: &'a HashSet<SidebarGroup>,
    /// Extra tasks revealed past each group's first page.
    pub reveal: &'a HashMap<SidebarGroup, usize>,
    /// The task on screen, listed even past its group's page so selecting it
    /// from elsewhere can scroll it into view.
    pub keep_visible: Option<Uuid>,
}

pub(super) struct SidebarLayout {
    pub rows: Vec<SidebarRow>,
    pub toggle_all: ToggleAll,
    /// The groups collapse-all folds: the projects, or the date groups.
    pub toggle_targets: Vec<SidebarGroup>,
}

pub(super) fn build_sidebar_rows(inputs: &SidebarRowInputs) -> SidebarLayout {
    let mut rows = vec![SidebarRow::Search, SidebarRow::Toolbar];
    let marks = inputs.marks;

    if inputs.view == SidebarView::Archived {
        let mut archived = inputs
            .entries
            .iter()
            .filter(|entry| marks.is_archived(entry.id))
            .collect::<Vec<_>>();
        archived.sort_by(|a, b| {
            marks
                .archived_at(b.id)
                .cmp(&marks.archived_at(a.id))
                .then(b.timestamp.cmp(&a.timestamp))
        });
        rows.push(SidebarRow::Header(
            SidebarGroup::Archived,
            GroupBadge::default(),
        ));
        if archived.is_empty() {
            rows.push(SidebarRow::Empty(SidebarEmpty::NoArchived));
        } else {
            push_page(
                &mut rows,
                SidebarGroup::Archived,
                &archived,
                FLAT_PAGE,
                inputs,
            );
        }
        rows.push(SidebarRow::GroupSpacer);
        return SidebarLayout {
            rows,
            toggle_all: ToggleAll::Hidden,
            toggle_targets: Vec::new(),
        };
    }

    let (mut pinned, others): (Vec<&TaskEntry>, Vec<&TaskEntry>) = inputs
        .entries
        .iter()
        .filter(|entry| !marks.is_archived(entry.id))
        .partition(|entry| marks.is_pinned(entry.id));
    let mut sections = Vec::new();
    if !pinned.is_empty() {
        sort_tasks(&mut pinned, inputs.ordering);
        push_section(
            &mut rows,
            SidebarGroup::Pinned,
            &pinned,
            FLAT_PAGE,
            None,
            inputs,
        );
        sections.push(SidebarGroup::Pinned);
    }

    let mut targets = Vec::new();
    match inputs.view {
        SidebarView::Timeline => {
            for date_group in inputs.date_order {
                let mut bucket = others
                    .iter()
                    .copied()
                    .filter(|entry| entry.date_group == date_group)
                    .collect::<Vec<_>>();
                if bucket.is_empty() {
                    continue;
                }
                sort_tasks(&mut bucket, inputs.ordering);
                let group = SidebarGroup::Updated(date_group);
                targets.push(group);
                let collapsed = inputs.collapsed.contains(&group);
                rows.push(SidebarRow::Header(group, badge(collapsed, &bucket, marks)));
                if !collapsed {
                    rows.extend(bucket.iter().map(|entry| SidebarRow::Session(entry.id)));
                }
                rows.push(SidebarRow::GroupSpacer);
            }
            if targets.is_empty() && pinned.is_empty() {
                rows.push(SidebarRow::Empty(SidebarEmpty::NoTasks));
            }
        }
        SidebarView::ByProject => {
            let (projectless, in_projects): (Vec<&TaskEntry>, Vec<&TaskEntry>) =
                others.iter().copied().partition(|entry| entry.projectless);

            // A project is listed while it has a task outside the archive, or
            // while it is the one selected — so a project just added shows up.
            let mut recency = HashMap::new();
            for entry in inputs
                .entries
                .iter()
                .filter(|entry| !entry.projectless && !marks.is_archived(entry.id))
            {
                let latest = recency.entry(entry.project_id).or_insert(entry.timestamp);
                *latest = (*latest).max(entry.timestamp);
            }
            let mut projects = inputs
                .projects
                .iter()
                .filter(|project| {
                    recency.contains_key(&project.id) || inputs.selected_project == Some(project.id)
                })
                .map(|project| {
                    (
                        project.id,
                        recency
                            .get(&project.id)
                            .copied()
                            .unwrap_or(project.created_at),
                    )
                })
                .collect::<Vec<_>>();
            // Running tasks never reorder projects: a group jumping each time
            // one of its tasks starts would move under the pointer.
            projects.sort_by(|(_, a), (_, b)| match inputs.ordering {
                SidebarOrdering::Newest => b.cmp(a),
                SidebarOrdering::Oldest => a.cmp(b),
            });
            targets.extend(projects.iter().map(|(id, _)| SidebarGroup::Project(*id)));

            let projects_collapsed = inputs.collapsed.contains(&SidebarGroup::Projects);
            rows.push(SidebarRow::Header(
                SidebarGroup::Projects,
                badge(projects_collapsed, &in_projects, marks),
            ));
            sections.push(SidebarGroup::Projects);
            if !projects_collapsed {
                if projects.is_empty() {
                    rows.push(SidebarRow::Empty(SidebarEmpty::NoProjects));
                }
                for (project_id, _) in &projects {
                    let mut tasks = in_projects
                        .iter()
                        .copied()
                        .filter(|entry| entry.project_id == *project_id)
                        .collect::<Vec<_>>();
                    sort_tasks(&mut tasks, inputs.ordering);
                    let group = SidebarGroup::Project(*project_id);
                    let collapsed = inputs.collapsed.contains(&group);
                    rows.push(SidebarRow::Header(group, badge(collapsed, &tasks, marks)));
                    if !collapsed {
                        push_page(&mut rows, group, &tasks, PROJECT_PAGE, inputs);
                    }
                }
            }
            rows.push(SidebarRow::GroupSpacer);

            let mut projectless = projectless;
            sort_tasks(&mut projectless, inputs.ordering);
            push_section(
                &mut rows,
                SidebarGroup::Projectless,
                &projectless,
                FLAT_PAGE,
                Some(SidebarEmpty::NoTasks),
                inputs,
            );
            sections.push(SidebarGroup::Projectless);
        }
        SidebarView::Archived => unreachable!("the archive returned above"),
    }

    let any_collapsed = targets
        .iter()
        .chain(&sections)
        .any(|group| inputs.collapsed.contains(group));
    let toggle_all = if any_collapsed {
        ToggleAll::ExpandAll
    } else if targets.is_empty() {
        ToggleAll::Hidden
    } else {
        ToggleAll::CollapseAll
    };
    SidebarLayout {
        rows,
        toggle_all,
        toggle_targets: targets,
    }
}

/// Running tasks first, then by recency in the chosen direction. Stable, so
/// equal tasks keep the order they came in.
fn sort_tasks(tasks: &mut [&TaskEntry], ordering: SidebarOrdering) {
    tasks.sort_by(|a, b| {
        b.running.cmp(&a.running).then_with(|| match ordering {
            SidebarOrdering::Newest => b.timestamp.cmp(&a.timestamp),
            SidebarOrdering::Oldest => a.timestamp.cmp(&b.timestamp),
        })
    });
}

/// A folded group's badge counts every task in it, not only the first page.
fn badge(collapsed: bool, tasks: &[&TaskEntry], marks: &TaskMarks) -> GroupBadge {
    if !collapsed {
        return GroupBadge::default();
    }
    GroupBadge {
        running: tasks.iter().any(|entry| entry.running),
        unread: tasks.iter().any(|entry| marks.is_unread(entry.id)),
    }
}

/// A collapsible section: its header, its first page unless folded (or the
/// empty row), and the spacer after it.
fn push_section(
    rows: &mut Vec<SidebarRow>,
    group: SidebarGroup,
    tasks: &[&TaskEntry],
    page: usize,
    empty: Option<SidebarEmpty>,
    inputs: &SidebarRowInputs,
) {
    let collapsed = inputs.collapsed.contains(&group);
    rows.push(SidebarRow::Header(
        group,
        badge(collapsed, tasks, inputs.marks),
    ));
    if !collapsed {
        if tasks.is_empty() {
            rows.extend(empty.map(SidebarRow::Empty));
        } else {
            push_page(rows, group, tasks, page, inputs);
        }
    }
    rows.push(SidebarRow::GroupSpacer);
}

/// The group's first page plus whatever was revealed, the task on screen
/// wherever it falls, and a "show more" row while anything stays hidden.
fn push_page(
    rows: &mut Vec<SidebarRow>,
    group: SidebarGroup,
    tasks: &[&TaskEntry],
    page: usize,
    inputs: &SidebarRowInputs,
) {
    let limit = page.saturating_add(inputs.reveal.get(&group).copied().unwrap_or_default());
    let mut hidden = false;
    for (index, entry) in tasks.iter().enumerate() {
        if index < limit || inputs.keep_visible == Some(entry.id) {
            rows.push(SidebarRow::Session(entry.id));
        } else {
            hidden = true;
        }
    }
    if hidden {
        rows.push(SidebarRow::ShowMore(group));
    }
}

/// The smallest splice that turns `old` into `new`: the range of `old` to
/// replace and how many rows replace it. `None` when nothing changed.
///
/// GPUI moves the scroll position to the start of a spliced range that holds
/// it, so replacing everything after the first change — a group folding near
/// the top, a task starting to run and moving up — would throw a scrolled
/// list back to that point. Keeping the unchanged tail out of the range
/// leaves the list where it was.
pub(super) fn sidebar_splice(
    old: &[SidebarRow],
    new: &[SidebarRow],
) -> Option<(Range<usize>, usize)> {
    if old == new {
        return None;
    }
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| old == new)
        .count();
    let longest_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(longest_suffix)
        .take_while(|(old, new)| old == new)
        .count();
    Some((prefix..old.len() - suffix, new.len() - prefix - suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    const PROJECT_A: u128 = 1_000;
    const PROJECT_B: u128 = 2_000;
    const LOOSE: u128 = 9_000;

    fn task(n: u128, project: u128, timestamp: u64) -> TaskEntry {
        TaskEntry {
            id: id(n),
            project_id: id(project),
            timestamp,
            running: false,
            projectless: project == LOOSE,
            date_group: SessionDateGroup::Today,
        }
    }

    fn running(mut entry: TaskEntry) -> TaskEntry {
        entry.running = true;
        entry
    }

    fn dated(mut entry: TaskEntry, date_group: SessionDateGroup) -> TaskEntry {
        entry.date_group = date_group;
        entry
    }

    struct Fixture {
        entries: Vec<TaskEntry>,
        projects: Vec<ProjectEntry>,
        marks: TaskMarks,
        collapsed: HashSet<SidebarGroup>,
        reveal: HashMap<SidebarGroup, usize>,
        selected_project: Option<Uuid>,
        keep_visible: Option<Uuid>,
        ordering: SidebarOrdering,
    }

    impl Fixture {
        fn new(entries: Vec<TaskEntry>) -> Self {
            Self {
                entries,
                projects: vec![
                    ProjectEntry {
                        id: id(PROJECT_A),
                        created_at: 1,
                    },
                    ProjectEntry {
                        id: id(PROJECT_B),
                        created_at: 2,
                    },
                ],
                marks: TaskMarks::default(),
                collapsed: HashSet::new(),
                reveal: HashMap::new(),
                selected_project: None,
                keep_visible: None,
                ordering: SidebarOrdering::Newest,
            }
        }

        fn build(&self, view: SidebarView) -> SidebarLayout {
            let mut date_order = [
                SessionDateGroup::Today,
                SessionDateGroup::Yesterday,
                SessionDateGroup::ThisWeek,
                SessionDateGroup::ThisMonth,
                SessionDateGroup::ThisYear,
                SessionDateGroup::More,
            ];
            if self.ordering == SidebarOrdering::Oldest {
                date_order.reverse();
            }
            build_sidebar_rows(&SidebarRowInputs {
                view,
                ordering: self.ordering,
                date_order,
                entries: &self.entries,
                projects: &self.projects,
                selected_project: self.selected_project,
                marks: &self.marks,
                collapsed: &self.collapsed,
                reveal: &self.reveal,
                keep_visible: self.keep_visible,
            })
        }

        fn rows(&self, view: SidebarView) -> Vec<SidebarRow> {
            self.build(view).rows
        }
    }

    fn sessions(rows: &[SidebarRow]) -> Vec<Uuid> {
        rows.iter()
            .filter_map(|row| match row {
                SidebarRow::Session(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    use SidebarRow::{Empty, GroupSpacer, Header, Search, Session, ShowMore, Toolbar};

    const PLAIN: GroupBadge = GroupBadge {
        running: false,
        unread: false,
    };

    #[test]
    fn by_project_lists_projects_then_project_less_tasks() {
        let fixture = Fixture::new(vec![
            task(1, PROJECT_A, 10),
            task(2, PROJECT_B, 30),
            task(3, PROJECT_A, 20),
            task(4, LOOSE, 40),
        ]);
        assert_eq!(
            fixture.rows(SidebarView::ByProject),
            vec![
                Search,
                Toolbar,
                Header(SidebarGroup::Projects, PLAIN),
                Header(SidebarGroup::Project(id(PROJECT_B)), PLAIN),
                Session(id(2)),
                Header(SidebarGroup::Project(id(PROJECT_A)), PLAIN),
                Session(id(3)),
                Session(id(1)),
                GroupSpacer,
                Header(SidebarGroup::Projectless, PLAIN),
                Session(id(4)),
                GroupSpacer,
            ]
        );
    }

    #[test]
    fn pinned_tasks_move_to_their_own_section_and_are_never_repeated() {
        let mut fixture = Fixture::new(vec![
            task(1, PROJECT_A, 10),
            task(2, PROJECT_A, 20),
            task(3, LOOSE, 30),
        ]);
        fixture.marks.pin(id(1), 1);
        fixture.marks.pin(id(3), 2);
        for view in [SidebarView::ByProject, SidebarView::Timeline] {
            let rows = fixture.rows(view);
            assert_eq!(rows[2], Header(SidebarGroup::Pinned, PLAIN));
            assert_eq!(rows[3..5], [Session(id(3)), Session(id(1))]);
            let mut ids = sessions(&rows);
            let listed = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), listed, "{view:?} repeats a task");
            assert_eq!(listed, 3);
        }
    }

    #[test]
    fn archived_tasks_leave_the_lists_for_the_archive_view() {
        let mut fixture = Fixture::new(vec![
            task(1, PROJECT_A, 10),
            task(2, PROJECT_A, 20),
            task(3, LOOSE, 30),
        ]);
        fixture.marks.archive(id(1), 100);
        fixture.marks.archive(id(3), 200);
        for view in [SidebarView::ByProject, SidebarView::Timeline] {
            assert_eq!(sessions(&fixture.rows(view)), vec![id(2)], "{view:?}");
        }
        let archive = fixture.build(SidebarView::Archived);
        assert_eq!(
            archive.rows,
            vec![
                Search,
                Toolbar,
                Header(SidebarGroup::Archived, PLAIN),
                Session(id(3)),
                Session(id(1)),
                GroupSpacer,
            ]
        );
        assert_eq!(archive.toggle_all, ToggleAll::Hidden);

        let empty = Fixture::new(vec![task(1, PROJECT_A, 10)]);
        assert_eq!(
            empty.rows(SidebarView::Archived)[3],
            Empty(SidebarEmpty::NoArchived)
        );
    }

    #[test]
    fn the_archive_shows_twenty_at_a_time() {
        let mut fixture = Fixture::new((1..=25).map(|n| task(n, PROJECT_A, n as u64)).collect());
        for n in 1..=25 {
            fixture.marks.archive(id(n), n as u64);
        }
        let rows = fixture.rows(SidebarView::Archived);
        assert_eq!(sessions(&rows).len(), 20);
        assert_eq!(rows[rows.len() - 2], ShowMore(SidebarGroup::Archived));
        fixture
            .reveal
            .insert(SidebarGroup::Archived, reveal_batch(SidebarGroup::Archived));
        assert_eq!(sessions(&fixture.rows(SidebarView::Archived)).len(), 25);
    }

    #[test]
    fn running_tasks_come_first_but_never_reorder_projects() {
        let fixture = Fixture::new(vec![
            task(1, PROJECT_A, 10),
            running(task(2, PROJECT_A, 5)),
            task(3, PROJECT_B, 30),
        ]);
        let rows = fixture.rows(SidebarView::ByProject);
        assert_eq!(sessions(&rows), vec![id(3), id(2), id(1)]);
        assert_eq!(rows[3], Header(SidebarGroup::Project(id(PROJECT_B)), PLAIN));

        let mut oldest = fixture;
        oldest.ordering = SidebarOrdering::Oldest;
        let rows = oldest.rows(SidebarView::Timeline);
        assert_eq!(sessions(&rows), vec![id(2), id(1), id(3)]);
    }

    #[test]
    fn timeline_keeps_date_groups_and_unfolds_them_whole() {
        let fixture = Fixture::new(vec![
            dated(task(1, PROJECT_A, 10), SessionDateGroup::Yesterday),
            task(2, PROJECT_B, 30),
            dated(task(3, LOOSE, 5), SessionDateGroup::Yesterday),
        ]);
        let layout = fixture.build(SidebarView::Timeline);
        assert_eq!(
            layout.rows,
            vec![
                Search,
                Toolbar,
                Header(SidebarGroup::Updated(SessionDateGroup::Today), PLAIN),
                Session(id(2)),
                GroupSpacer,
                Header(SidebarGroup::Updated(SessionDateGroup::Yesterday), PLAIN),
                Session(id(1)),
                Session(id(3)),
                GroupSpacer,
            ]
        );
        assert_eq!(layout.toggle_all, ToggleAll::CollapseAll);
        assert_eq!(
            layout.toggle_targets,
            vec![
                SidebarGroup::Updated(SessionDateGroup::Today),
                SidebarGroup::Updated(SessionDateGroup::Yesterday),
            ]
        );
    }

    #[test]
    fn a_project_shows_five_tasks_then_five_more_per_reveal() {
        let mut fixture = Fixture::new(
            (1..=12)
                .map(|n| task(n, PROJECT_A, 100 - n as u64))
                .collect(),
        );
        let group = SidebarGroup::Project(id(PROJECT_A));
        let rows = fixture.rows(SidebarView::ByProject);
        assert_eq!(sessions(&rows), (1..=5).map(id).collect::<Vec<_>>());
        assert!(rows.contains(&ShowMore(group)));

        fixture.reveal.insert(group, reveal_batch(group));
        assert_eq!(sessions(&fixture.rows(SidebarView::ByProject)).len(), 10);
        fixture.reveal.insert(group, 2 * reveal_batch(group));
        let rows = fixture.rows(SidebarView::ByProject);
        assert_eq!(sessions(&rows).len(), 12);
        assert!(!rows.contains(&ShowMore(group)));
    }

    #[test]
    fn project_less_tasks_show_twenty_at_a_time() {
        let fixture = Fixture::new((1..=21).map(|n| task(n, LOOSE, n as u64)).collect());
        let rows = fixture.rows(SidebarView::ByProject);
        assert_eq!(sessions(&rows).len(), 20);
        assert!(rows.contains(&ShowMore(SidebarGroup::Projectless)));
    }

    #[test]
    fn the_task_on_screen_stays_listed_past_its_page() {
        let mut fixture = Fixture::new(
            (1..=8)
                .map(|n| task(n, PROJECT_A, 100 - n as u64))
                .collect(),
        );
        fixture.keep_visible = Some(id(7));
        let rows = fixture.rows(SidebarView::ByProject);
        assert_eq!(
            sessions(&rows),
            vec![id(1), id(2), id(3), id(4), id(5), id(7)]
        );
        assert!(rows.contains(&ShowMore(SidebarGroup::Project(id(PROJECT_A)))));

        // Nothing else hidden: no reveal row for nothing.
        fixture.entries.truncate(6);
        fixture.keep_visible = Some(id(6));
        let rows = fixture.rows(SidebarView::ByProject);
        assert_eq!(sessions(&rows).len(), 6);
        assert!(!rows.contains(&ShowMore(SidebarGroup::Project(id(PROJECT_A)))));
    }

    #[test]
    fn projects_need_a_live_task_unless_selected() {
        let mut fixture = Fixture::new(vec![task(1, PROJECT_A, 10), task(2, PROJECT_B, 20)]);
        fixture.marks.archive(id(2), 1);
        let rows = fixture.rows(SidebarView::ByProject);
        assert!(!rows.contains(&Header(SidebarGroup::Project(id(PROJECT_B)), PLAIN)));

        fixture.selected_project = Some(id(PROJECT_B));
        let rows = fixture.rows(SidebarView::ByProject);
        assert!(rows.contains(&Header(SidebarGroup::Project(id(PROJECT_B)), PLAIN)));
    }

    #[test]
    fn empty_sections_say_so() {
        let fixture = Fixture::new(Vec::new());
        assert_eq!(
            fixture.rows(SidebarView::ByProject),
            vec![
                Search,
                Toolbar,
                Header(SidebarGroup::Projects, PLAIN),
                Empty(SidebarEmpty::NoProjects),
                GroupSpacer,
                Header(SidebarGroup::Projectless, PLAIN),
                Empty(SidebarEmpty::NoTasks),
                GroupSpacer,
            ]
        );
        assert_eq!(
            fixture.rows(SidebarView::Timeline),
            vec![Search, Toolbar, Empty(SidebarEmpty::NoTasks)]
        );
        assert_eq!(
            fixture.build(SidebarView::Timeline).toggle_all,
            ToggleAll::Hidden
        );
    }

    #[test]
    fn a_folded_group_keeps_a_badge_for_every_task_inside() {
        let mut fixture = Fixture::new(
            (1..=7)
                .map(|n| task(n, PROJECT_A, 100 - n as u64))
                .chain([running(task(8, PROJECT_A, 1))])
                .collect(),
        );
        // Past the first page, yet the folded header still reports them.
        fixture.marks.mark_unread(id(7));
        let group = SidebarGroup::Project(id(PROJECT_A));
        fixture.collapsed.insert(group);
        let layout = fixture.build(SidebarView::ByProject);
        assert_eq!(
            layout.rows[3],
            Header(
                group,
                GroupBadge {
                    running: true,
                    unread: true
                }
            )
        );
        assert!(sessions(&layout.rows).is_empty());
        assert_eq!(layout.toggle_all, ToggleAll::ExpandAll);

        fixture.collapsed.clear();
        fixture.collapsed.insert(SidebarGroup::Projects);
        let rows = fixture.rows(SidebarView::ByProject);
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, Header(SidebarGroup::Project(_), _)))
        );
    }

    #[test]
    fn splice_keeps_the_unchanged_tail_out_of_the_range() {
        let rows = |ids: &[u128]| ids.iter().map(|n| Session(id(*n))).collect::<Vec<_>>();
        assert_eq!(sidebar_splice(&rows(&[1, 2, 3]), &rows(&[1, 2, 3])), None);
        assert_eq!(
            sidebar_splice(&rows(&[1, 2, 3, 4]), &rows(&[1, 9, 3, 4])),
            Some((1..2, 1))
        );
        assert_eq!(
            sidebar_splice(&rows(&[1, 2]), &rows(&[1, 2, 3])),
            Some((2..2, 1))
        );
        assert_eq!(
            sidebar_splice(&rows(&[1, 2, 3]), &rows(&[1, 3])),
            Some((1..2, 0))
        );
        // A repeated row cannot be counted on both sides.
        assert_eq!(
            sidebar_splice(&rows(&[1, 1]), &rows(&[1, 1, 1])),
            Some((2..2, 1))
        );
    }
}
