//! Pinned, archived and unread tasks: the app's side of
//! `sub2api::task_marks`.
//!
//! Fork file. The marks are read once when the window opens, so an archived
//! task never flashes in the list, and written in the background after every
//! change, one write at a time with the newest marks.
//!
//! A task becomes unread when a turn finishes while it is not the task on
//! screen, and read when it comes on screen. That moment is noticed from
//! `render` rather than from selection: returning from Settings or an image
//! page shows a task without selecting it. Only the change counts, so a task
//! marked unread on purpose stays unread while it is still being looked at.

use sub2api::task_marks::{self, TaskMarks};

use super::sidebar::{SidebarRow, localized_session_title};
use super::*;

pub(super) struct TaskMarksState {
    pub(super) marks: TaskMarks,
    /// Bumped on every change; the sidebar's row fingerprint folds it in.
    revision: u64,
    /// A write is in flight; the next one waits for it.
    saving: bool,
    /// The task last seen on screen.
    last_on_screen: Option<Uuid>,
}

impl TaskMarksState {
    pub(super) fn load() -> Self {
        Self {
            marks: task_marks::load(),
            revision: 0,
            saving: false,
            last_on_screen: None,
        }
    }

    pub(super) fn is_archived(&self, session_id: Uuid) -> bool {
        self.marks.is_archived(session_id)
    }

    /// At launch: open the sidebar on its project view, once (see
    /// [`switch_to_project_view_once`]). The marks are written right here,
    /// before the window draws, so the switch is never repeated. Returns
    /// whether the app state changed and needs saving.
    pub(super) fn adopt_project_view(&mut self, state: &mut PersistedState) -> bool {
        let (state_changed, marks_changed) =
            switch_to_project_view_once(&mut state.sidebar_grouping, &mut self.marks);
        if marks_changed && let Err(error) = task_marks::save(&self.marks) {
            eprintln!("could not save the task marks: {error:#}");
        }
        state_changed
    }
}

/// The project view is the sidebar's default, but builds before the redesign
/// saved the timeline for everyone whether they chose it or not. Switch once,
/// and leave whatever is picked afterwards alone. Returns whether the grouping
/// and whether the marks changed.
fn switch_to_project_view_once(grouping: &mut SidebarGrouping, marks: &mut TaskMarks) -> (bool, bool) {
    if marks.project_view_adopted {
        return (false, false);
    }
    marks.project_view_adopted = true;
    let changed = *grouping != SidebarGrouping::Project;
    *grouping = SidebarGrouping::Project;
    (changed, true)
}

/// The task to select after archiving one that was showing: the most recent
/// other task of its project that is not archived itself.
pub(super) fn archive_successor(
    sessions: &[AgentSession],
    project_id: Uuid,
    archived: Uuid,
    marks: &TaskMarks,
) -> Option<Uuid> {
    sessions
        .iter()
        .filter(|session| {
            session.project_id == project_id
                && session.id != archived
                && !marks.is_archived(session.id)
        })
        .max_by_key(|session| session.updated_at)
        .map(|session| session.id)
}

impl Waku {
    /// Everything the sidebar rows read from the marks and the archive view.
    pub(super) fn mix_task_marks_fingerprint(&self, fingerprint: u64) -> u64 {
        let fingerprint = mix(fingerprint, self.task_marks.revision);
        let fingerprint = mix(fingerprint, u64::from(self.sidebar_toolbar.archived_open));
        match self.sidebar_keep_visible() {
            Some(session_id) => mix_uuid(fingerprint, session_id),
            None => mix(fingerprint, u64::MAX),
        }
    }

    /// The task the sidebar keeps listed past its group's page.
    pub(super) fn sidebar_keep_visible(&self) -> Option<Uuid> {
        self.pending_session_activation
            .map(|pending| pending.session_id)
            .or(self.state.selected_session)
    }

    /// The task in the main column, if one is: not while a fork page or
    /// Settings covers it.
    fn on_screen_task(&self) -> Option<Uuid> {
        self.state
            .selected_session
            .filter(|_| !self.main_page_open() && self.settings_page.is_none())
    }

    /// Hook in `render`: a task coming on screen has been read.
    pub(super) fn note_task_on_screen(&mut self, cx: &mut Context<Self>) {
        let on_screen = self.on_screen_task();
        if on_screen == self.task_marks.last_on_screen {
            return;
        }
        self.task_marks.last_on_screen = on_screen;
        if let Some(session_id) = on_screen
            && self.task_marks.marks.mark_read(session_id)
        {
            self.task_marks_changed(cx);
        }
    }

    /// Hook where a turn finishes: a reply nobody is looking at is unread.
    pub(super) fn note_task_turn_finished(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.on_screen_task() != Some(session_id)
            && self.task_marks.marks.mark_unread(session_id)
        {
            self.task_marks_changed(cx);
        }
    }

    /// Hook where a message is submitted: writing to an archived task brings
    /// it back.
    pub(super) fn unarchive_on_submit(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        self.unarchive_task(session_id, cx);
    }

    /// Hook where a task is deleted.
    pub(super) fn forget_task_marks(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.task_marks.marks.forget(session_id) {
            self.task_marks_changed(cx);
        }
    }

    /// Hook where the daemon's catalogue drops tasks.
    pub(super) fn forget_task_marks_many(&mut self, session_ids: &[Uuid], cx: &mut Context<Self>) {
        if self.task_marks.marks.forget_many(session_ids) {
            self.task_marks_changed(cx);
        }
    }

    pub(super) fn toggle_task_pin(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let marks = &mut self.task_marks.marks;
        let changed = if marks.is_pinned(session_id) {
            marks.unpin(session_id)
        } else {
            marks.pin(session_id, unix_time())
        };
        if changed {
            self.task_marks_changed(cx);
        }
    }

    pub(super) fn mark_task_unread(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.task_marks.marks.mark_unread(session_id) {
            self.task_marks_changed(cx);
        }
    }

    pub(super) fn unarchive_task(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.task_marks.marks.unarchive(session_id) {
            self.task_marks_changed(cx);
        }
    }

    /// Put a task away. A running task stays: its turn would go on out of
    /// sight. The task showing moves on to the next one of its project.
    pub(super) fn archive_task(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(project_id) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id && !session.is_busy())
            .map(|session| session.project_id)
        else {
            return;
        };
        if !self.task_marks.marks.archive(session_id, unix_time()) {
            return;
        }
        self.task_switcher.remove(session_id);
        let showing = self.sidebar_keep_visible() == Some(session_id);
        self.task_marks_changed(cx);
        if showing {
            match archive_successor(
                &self.state.sessions,
                project_id,
                session_id,
                &self.task_marks.marks,
            ) {
                Some(next) => self.select_session(next, cx),
                None => {
                    let projectless = self
                        .state
                        .projects
                        .iter()
                        .find(|project| project.id == project_id)
                        .is_some_and(Project::is_projectless);
                    if projectless {
                        self.create_projectless_session(cx);
                    } else {
                        self.create_session_for(project_id, self.state.last_provider, cx);
                    }
                }
            }
        }
    }

    /// Deleting is for the archive and asks first: a task's transcript does
    /// not come back.
    pub(super) fn confirm_delete_task(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(title) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(localized_session_title)
        else {
            return;
        };
        let detail = format!("{title}\n{}", tr!("session.remove_confirm_detail"));
        self.request_confirm(
            tr!("sidebar.delete_confirm_title"),
            Some(detail),
            tr!("sidebar.delete"),
            true,
            cx,
            move |this, _, cx| this.remove_session(session_id, cx),
        );
    }

    /// Before a row leaves its list (archived or restored), hand keyboard
    /// focus to the row beside it, so a keyboard user keeps their place.
    pub(super) fn leave_task_row(
        &mut self,
        session_id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row_focus = self
            .menu_handle(format!("session-{session_id}"), cx)
            .trigger_focus_handle()
            .clone();
        if !row_focus.contains_focused(window, cx) {
            return;
        }
        let neighbour = {
            let rows = self.sidebar_row_cache.borrow();
            rows.iter()
                .position(|row| *row == SidebarRow::Session(session_id))
                .and_then(|index| {
                    rows[index + 1..]
                        .iter()
                        .chain(rows[..index].iter().rev())
                        .find_map(|row| match row {
                            SidebarRow::Session(id) => Some(*id),
                            _ => None,
                        })
                })
        };
        if let Some(neighbour) = neighbour {
            let focus = self
                .menu_handle(format!("session-{neighbour}"), cx)
                .trigger_focus_handle()
                .clone();
            window.focus(&focus, cx);
        }
    }

    fn task_marks_changed(&mut self, cx: &mut Context<Self>) {
        self.task_marks.revision = self.task_marks.revision.wrapping_add(1);
        self.schedule_task_marks_save(cx);
        cx.notify();
    }

    /// One write at a time, always of the newest marks: a change made while
    /// a write is in flight is written right after it.
    fn schedule_task_marks_save(&mut self, cx: &mut Context<Self>) {
        if self.task_marks.saving {
            return;
        }
        self.task_marks.saving = true;
        let revision = self.task_marks.revision;
        let marks = self.task_marks.marks.clone();
        cx.spawn(async move |waku, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { task_marks::save(&marks) })
                .await;
            let _ = waku.update(cx, |this, cx| {
                this.task_marks.saving = false;
                if let Err(error) = result {
                    this.show_toast(tr!("errors.save_local_state", error = error));
                    cx.notify();
                }
                if this.task_marks.revision != revision {
                    this.schedule_task_marks_save(cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archiving_the_task_on_screen_moves_to_the_newest_other_task_of_its_project() {
        let project = Uuid::from_u128(1);
        let other_project = Uuid::from_u128(2);
        let mut sessions = Vec::new();
        for (project_id, updated_at) in [
            (project, 10),
            (project, 30),
            (other_project, 50),
            (project, 20),
        ] {
            let mut session = AgentSession::new(project_id, ProviderKind::Native);
            session.updated_at = updated_at;
            sessions.push(session);
        }
        let archived = sessions[1].id;
        let mut marks = TaskMarks::default();
        marks.archive(archived, 1);
        // The newest is the one being archived; the next newest is already
        // archived too; the project's remaining task is picked.
        marks.archive(sessions[3].id, 2);
        assert_eq!(
            archive_successor(&sessions, project, archived, &marks),
            Some(sessions[0].id)
        );
        marks.archive(sessions[0].id, 3);
        assert_eq!(
            archive_successor(&sessions, project, archived, &marks),
            None
        );
    }

    #[test]
    fn the_project_view_is_switched_to_once_then_left_to_the_person() {
        let mut marks = TaskMarks::default();
        let mut grouping = SidebarGrouping::Updated;
        assert_eq!(
            switch_to_project_view_once(&mut grouping, &mut marks),
            (true, true)
        );
        assert_eq!(grouping, SidebarGrouping::Project);
        assert!(marks.project_view_adopted);

        // Picked the timeline afterwards: it stays.
        grouping = SidebarGrouping::Updated;
        assert_eq!(
            switch_to_project_view_once(&mut grouping, &mut marks),
            (false, false)
        );
        assert_eq!(grouping, SidebarGrouping::Updated);

        // Already on the project view: only the flag is recorded.
        let mut fresh = TaskMarks::default();
        let mut grouping = SidebarGrouping::Project;
        assert_eq!(
            switch_to_project_view_once(&mut grouping, &mut fresh),
            (false, true)
        );
    }
}
