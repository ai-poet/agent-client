// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The task state machine and execution generations.
//!
//! Every claim starts a new *attempt*: a counter plus a random capability id
//! the worker must present on each update. Reassigning or retrying a task
//! revokes the capability first, so a late update from the previous owner is
//! rejected as stale instead of overwriting the new owner's work.

use crate::types::{TaskStatus, TeamTask, now_ms};

/// The allowed transitions out of `current`. Terminal statuses have none.
pub fn allowed_transitions(current: TaskStatus) -> &'static [TaskStatus] {
    match current {
        TaskStatus::Pending => &[TaskStatus::Claimed, TaskStatus::Cancelled],
        TaskStatus::Claimed => &[
            TaskStatus::InProgress,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ],
        TaskStatus::InProgress => &[
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ],
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => &[],
    }
}

/// The transition error, or `None` when allowed. Staying put is allowed.
pub fn transition_error(current: TaskStatus, next: TaskStatus) -> Option<String> {
    if current == next || allowed_transitions(current).contains(&next) {
        return None;
    }
    Some(format!(
        "task status cannot move from \"{current}\" to \"{next}\""
    ))
}

/// The ids among `dependencies` that are not yet completed (or do not
/// exist); empty when the task is claimable.
pub fn unsatisfied_dependencies(tasks: &[TeamTask], dependencies: &[String]) -> Vec<String> {
    dependencies
        .iter()
        .filter(|id| {
            !tasks
                .iter()
                .any(|task| &task.id == *id && task.status == TaskStatus::Completed)
        })
        .cloned()
        .collect()
}

/// Execution results belong to one attempt; a retry must produce its own.
fn clear_attempt_result(task: &mut TeamTask) {
    task.output = None;
    task.verdict = None;
    task.findings = None;
    task.changed_paths = None;
    task.acceptance_results = None;
    task.commands_run = None;
}

/// Activate the task's current generation for one owner; returns the
/// capability id.
pub fn activate_task_attempt(task: &mut TeamTask, assignee: &str) -> String {
    let attempt_id = uuid::Uuid::new_v4().to_string();
    task.status = TaskStatus::Claimed;
    task.assignee = Some(assignee.to_owned());
    task.attempt_id = Some(attempt_id.clone());
    task.handoff_id = None;
    task.reassigning = Some(false);
    clear_attempt_result(task);
    task.updated_at = now_ms();
    attempt_id
}

/// Start a fresh generation for one owner.
pub fn begin_task_attempt(task: &mut TeamTask, assignee: &str) -> String {
    task.attempt = Some(task.attempt_number() + 1);
    activate_task_attempt(task, assignee)
}

/// Cancel one unfinished task without returning it to the ready pool.
pub fn cancel_unfinished_task(task: &mut TeamTask, output: Option<&str>) {
    if task.status.is_terminal() {
        return;
    }
    task.status = TaskStatus::Cancelled;
    task.attempt_id = None;
    task.handoff_id = None;
    task.reassigning = Some(false);
    if let Some(output) = output {
        task.output = Some(output.to_owned());
    }
    task.updated_at = now_ms();
}

/// Revoke the current worker at once. Clearing the capability makes old
/// updates stale; a separate handoff generation serializes the quiescing.
pub fn invalidate_task_attempt(
    task: &mut TeamTask,
    next_assignee: Option<&str>,
    reassigning: bool,
) {
    task.attempt_id = None;
    task.handoff_id = Some(uuid::Uuid::new_v4().to_string());
    task.status = TaskStatus::Pending;
    task.assignee = next_assignee.map(str::to_owned);
    task.reassigning = Some(reassigning);
    clear_attempt_result(task);
    task.updated_at = now_ms();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_follow_the_table() {
        assert!(transition_error(TaskStatus::Pending, TaskStatus::Claimed).is_none());
        assert!(transition_error(TaskStatus::Claimed, TaskStatus::InProgress).is_none());
        assert!(transition_error(TaskStatus::InProgress, TaskStatus::InProgress).is_none());
        assert_eq!(
            transition_error(TaskStatus::Claimed, TaskStatus::Completed).as_deref(),
            Some("task status cannot move from \"claimed\" to \"completed\"")
        );
        assert!(transition_error(TaskStatus::Completed, TaskStatus::Failed).is_some());
    }

    #[test]
    fn attempts_rotate_the_capability_and_clear_results() {
        let mut task = TeamTask::new("t1", "do", 1);
        let first = begin_task_attempt(&mut task, "dev");
        assert_eq!(task.attempt, Some(1));
        assert_eq!(task.status, TaskStatus::Claimed);
        task.output = Some("partial".into());
        invalidate_task_attempt(&mut task, Some("ops"), true);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.attempt_id.is_none() && task.handoff_id.is_some());
        assert!(task.output.is_none());
        assert!(task.is_reassigning());
        let second = begin_task_attempt(&mut task, "ops");
        assert_ne!(first, second);
        assert_eq!(task.attempt, Some(2));
        assert!(task.handoff_id.is_none() && !task.is_reassigning());
    }

    #[test]
    fn cancel_keeps_terminal_tasks_untouched() {
        let mut task = TeamTask::new("t1", "do", 1);
        task.status = TaskStatus::Completed;
        cancel_unfinished_task(&mut task, Some("stopped"));
        assert_eq!(task.status, TaskStatus::Completed);
        task.status = TaskStatus::InProgress;
        cancel_unfinished_task(&mut task, Some("stopped"));
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert_eq!(task.output.as_deref(), Some("stopped"));
    }

    #[test]
    fn unsatisfied_lists_missing_and_unfinished() {
        let mut done = TeamTask::new("t1", "a", 1);
        done.status = TaskStatus::Completed;
        let open = TeamTask::new("t2", "b", 1);
        let tasks = vec![done, open];
        let deps = vec!["t1".to_owned(), "t2".to_owned(), "t9".to_owned()];
        assert_eq!(unsatisfied_dependencies(&tasks, &deps), ["t2", "t9"]);
    }
}
