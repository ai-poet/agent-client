//! The session's goal: an objective the agent keeps working toward across
//! turns until it proves it done.
//!
//! The engine already has all of it — a per-session store
//! (`claurst_core::GoalStore`), a continuation policy that keeps the loop
//! going while the goal is active (`ContinuationMode::Goal`), and a
//! `GoalComplete` tool the model closes it with. What it lacked here was a
//! caller: the engine's CLI used to set goals and start their turns. This is
//! that caller, in the vocabulary Waku's goal chip and dialog already speak
//! for Codex.

use claurst_core::{Goal, GoalStatus, GoalStore};

/// Where a goal stands, in the terms the desktop shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalState {
    Active,
    Paused,
    BudgetLimited,
    Complete,
}

impl GoalState {
    fn from_engine(status: &GoalStatus) -> Self {
        match status {
            GoalStatus::Active => Self::Active,
            GoalStatus::Paused => Self::Paused,
            GoalStatus::BudgetLimited => Self::BudgetLimited,
            GoalStatus::Complete => Self::Complete,
        }
    }

    fn to_engine(self) -> GoalStatus {
        match self {
            Self::Active => GoalStatus::Active,
            Self::Paused => GoalStatus::Paused,
            Self::BudgetLimited => GoalStatus::BudgetLimited,
            Self::Complete => GoalStatus::Complete,
        }
    }
}

/// What the desktop shows of a goal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalSnapshot {
    pub objective: String,
    pub status: GoalState,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_secs: u64,
}

impl GoalSnapshot {
    fn from_engine(goal: &Goal) -> Self {
        Self {
            objective: goal.objective.clone(),
            status: GoalState::from_engine(&goal.status),
            token_budget: goal.token_budget,
            tokens_used: goal.tokens_used,
            time_used_secs: goal.time_used_secs,
        }
    }
}

/// A change the desktop asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalOp {
    /// Report the current goal without changing it.
    Refresh,
    /// Set or edit the goal. A new objective, or `replace`, starts a fresh
    /// goal (the store keeps one per session); `status` alone pauses or
    /// resumes the existing one.
    Set {
        objective: Option<String>,
        status: Option<GoalState>,
        replace: bool,
    },
    Clear,
}

/// What applying an operation calls for next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GoalEffect {
    /// Nothing to start.
    Settled,
    /// A new goal: send this as the prompt that starts pursuing it.
    Kickoff(String),
    /// A goal resumed: send this as the prompt that carries on.
    Resume(String),
}

/// Apply `op` to the goal stored for `session_id`.
pub(crate) fn apply(store: &GoalStore, session_id: &str, op: GoalOp) -> Result<GoalEffect, String> {
    match op {
        GoalOp::Refresh => Ok(GoalEffect::Settled),
        GoalOp::Clear => {
            store.clear_goal(session_id).map_err(|error| error.to_string())?;
            Ok(GoalEffect::Settled)
        }
        GoalOp::Set {
            objective,
            status,
            replace,
        } => {
            let current = store.get_goal(session_id);
            let objective = objective
                .map(|objective| objective.trim().to_owned())
                .filter(|objective| !objective.is_empty());
            let fresh = match (&objective, &current) {
                (Some(objective), Some(goal)) => replace || *objective != goal.objective,
                (Some(_), None) => true,
                (None, _) => false,
            };
            if fresh {
                let objective = objective.expect("a fresh goal has an objective");
                let goal = store
                    .set_goal(session_id, &objective, None)
                    .map_err(|error| error.to_string())?;
                if let Some(status) = status.filter(|status| *status != GoalState::Active) {
                    store
                        .set_status(session_id, status.to_engine())
                        .map_err(|error| error.to_string())?;
                    return Ok(GoalEffect::Settled);
                }
                return Ok(GoalEffect::Kickoff(claurst_core::goal_kickoff_message(&goal)));
            }
            let Some(goal) = current else {
                return Err("there is no goal to change".to_owned());
            };
            let Some(status) = status else {
                return Ok(GoalEffect::Settled);
            };
            store
                .set_status(session_id, status.to_engine())
                .map_err(|error| error.to_string())?;
            let resumed = status == GoalState::Active && goal.status != GoalStatus::Active;
            Ok(if resumed {
                GoalEffect::Resume(claurst_core::goal_continuation_message(&goal))
            } else {
                GoalEffect::Settled
            })
        }
    }
}

/// The goal stored for `session_id`, as the desktop shows it.
pub(crate) fn snapshot(store: &GoalStore, session_id: &str) -> Option<GoalSnapshot> {
    store
        .get_goal(session_id)
        .as_ref()
        .map(GoalSnapshot::from_engine)
}

/// Whether the session has a goal the engine should keep pursuing.
pub(crate) fn is_active(store: &GoalStore, session_id: &str) -> bool {
    store.get_active_goal(session_id).is_some()
}

/// The shared store, or `None` when its database cannot be opened — goals
/// are then simply unavailable, which must never fail a turn.
pub(crate) fn open_store() -> Option<GoalStore> {
    GoalStore::open_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, GoalStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = GoalStore::open(&dir.path().join("goals.sqlite")).unwrap();
        (dir, store)
    }

    fn set(objective: &str) -> GoalOp {
        GoalOp::Set {
            objective: Some(objective.into()),
            status: Some(GoalState::Active),
            replace: false,
        }
    }

    #[test]
    fn a_new_objective_starts_a_goal_and_asks_for_its_first_turn() {
        let (_dir, store) = store();
        let effect = apply(&store, "s", set("make the tests pass")).unwrap();
        let GoalEffect::Kickoff(prompt) = effect else {
            panic!("a new goal starts working: {effect:?}");
        };
        assert!(prompt.contains("make the tests pass"));
        let goal = snapshot(&store, "s").unwrap();
        assert_eq!(goal.status, GoalState::Active);
        assert!(is_active(&store, "s"));
    }

    #[test]
    fn pausing_and_resuming_keep_the_goal_and_only_resuming_starts_a_turn() {
        let (_dir, store) = store();
        apply(&store, "s", set("ship it")).unwrap();
        let pause = GoalOp::Set {
            objective: None,
            status: Some(GoalState::Paused),
            replace: false,
        };
        assert_eq!(apply(&store, "s", pause).unwrap(), GoalEffect::Settled);
        assert!(!is_active(&store, "s"));

        let resume = GoalOp::Set {
            objective: None,
            status: Some(GoalState::Active),
            replace: false,
        };
        assert!(matches!(apply(&store, "s", resume.clone()).unwrap(), GoalEffect::Resume(_)));
        // Resuming a goal that is already being pursued starts nothing.
        assert_eq!(apply(&store, "s", resume).unwrap(), GoalEffect::Settled);
    }

    #[test]
    fn the_same_objective_again_is_an_edit_and_a_different_one_replaces() {
        let (_dir, store) = store();
        apply(&store, "s", set("first")).unwrap();
        assert_eq!(
            apply(
                &store,
                "s",
                GoalOp::Set {
                    objective: Some("first".into()),
                    status: None,
                    replace: false,
                }
            )
            .unwrap(),
            GoalEffect::Settled
        );
        assert!(matches!(apply(&store, "s", set("second")).unwrap(), GoalEffect::Kickoff(_)));
        assert_eq!(snapshot(&store, "s").unwrap().objective, "second");
    }

    #[test]
    fn clearing_removes_it_and_a_status_change_without_one_is_refused() {
        let (_dir, store) = store();
        apply(&store, "s", set("anything")).unwrap();
        apply(&store, "s", GoalOp::Clear).unwrap();
        assert!(snapshot(&store, "s").is_none());
        let pause = GoalOp::Set {
            objective: None,
            status: Some(GoalState::Paused),
            replace: false,
        };
        assert!(apply(&store, "s", pause).is_err());
    }

    #[test]
    fn goals_belong_to_their_own_session() {
        let (_dir, store) = store();
        apply(&store, "a", set("a's goal")).unwrap();
        assert!(snapshot(&store, "b").is_none());
    }
}
