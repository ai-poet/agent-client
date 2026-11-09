//! The one fact the status capsule over the transcript shows.
//!
//! A long task has several things worth a glance — the goal it pursues, the
//! step of its plan it is on, how far through the plan it is, work left
//! running in the background — and room for one line. This picks the line;
//! the capsule opens onto all of them.

use crate::model::ThreadGoalStatus;
use crate::todo::{TodoItem, TodoStatus};

/// What the capsule says, most pressing first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapsuleFact {
    /// A goal still being pursued, or held up.
    Goal,
    /// The plan step being worked on.
    CurrentTodo(String),
    /// How far through its plan the agent is.
    Progress { done: usize, total: usize },
    /// Processes and agents left running.
    Background(usize),
    /// The goal was reached.
    GoalComplete,
}

/// Pick the capsule's line: an open goal, then the step in progress, then
/// progress through an unfinished plan, then background work, then a goal
/// reached. `None` when there is nothing to say.
pub fn select_fact(
    goal: Option<ThreadGoalStatus>,
    todos: &[TodoItem],
    background_running: usize,
) -> Option<CapsuleFact> {
    if goal.is_some_and(|status| status != ThreadGoalStatus::Complete) {
        return Some(CapsuleFact::Goal);
    }
    if let Some(item) = todos
        .iter()
        .find(|item| item.status == TodoStatus::InProgress)
    {
        return Some(CapsuleFact::CurrentTodo(item.label().to_owned()));
    }
    let (done, total) = todo_progress(todos);
    if total > 0 && done < total {
        return Some(CapsuleFact::Progress { done, total });
    }
    if background_running > 0 {
        return Some(CapsuleFact::Background(background_running));
    }
    (goal == Some(ThreadGoalStatus::Complete)).then_some(CapsuleFact::GoalComplete)
}

/// Steps done, and steps in all — a dropped step counts toward neither.
pub fn todo_progress(todos: &[TodoItem]) -> (usize, usize) {
    todos
        .iter()
        .filter(|item| item.status != TodoStatus::Cancelled)
        .fold((0, 0), |(done, total), item| {
            (
                done + usize::from(item.status == TodoStatus::Completed),
                total + 1,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(content: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            content: content.into(),
            status,
            active_form: None,
        }
    }

    #[test]
    fn an_open_goal_comes_before_everything_else() {
        let todos = [todo("Write the parser", TodoStatus::InProgress)];
        assert_eq!(
            select_fact(Some(ThreadGoalStatus::Active), &todos, 2),
            Some(CapsuleFact::Goal)
        );
        assert_eq!(
            select_fact(Some(ThreadGoalStatus::Paused), &[], 0),
            Some(CapsuleFact::Goal)
        );
    }

    #[test]
    fn the_step_in_progress_then_progress_then_background() {
        let mut todos = vec![
            todo("Read the code", TodoStatus::Completed),
            todo("Write the parser", TodoStatus::InProgress),
            todo("Test it", TodoStatus::Pending),
            todo("Drop this", TodoStatus::Cancelled),
        ];
        assert_eq!(
            select_fact(None, &todos, 1),
            Some(CapsuleFact::CurrentTodo("Write the parser".into()))
        );

        todos[1].status = TodoStatus::Completed;
        assert_eq!(
            select_fact(None, &todos, 1),
            Some(CapsuleFact::Progress { done: 2, total: 3 })
        );

        todos[2].status = TodoStatus::Completed;
        assert_eq!(
            select_fact(None, &todos, 1),
            Some(CapsuleFact::Background(1))
        );
        assert_eq!(select_fact(None, &todos, 0), None);
    }

    #[test]
    fn a_reached_goal_is_said_last() {
        assert_eq!(
            select_fact(Some(ThreadGoalStatus::Complete), &[], 1),
            Some(CapsuleFact::Background(1))
        );
        assert_eq!(
            select_fact(Some(ThreadGoalStatus::Complete), &[], 0),
            Some(CapsuleFact::GoalComplete)
        );
    }
}
