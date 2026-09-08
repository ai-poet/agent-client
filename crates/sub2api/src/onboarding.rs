//! The first-run checklist's state: two steps, what "done" means for each,
//! and whether the card should show at all.
//!
//! Fork addition. It used to be three — sign in, install a CLI, send a
//! message — because every agent Waku could drive was a Node package the user
//! had to install first. The built-in agent removed that step rather than
//! automating it: there is nothing to install, so the checklist stops asking.
//! Installing an external CLI is still possible, but it now belongs in
//! Settings → Providers as a choice, not in the path to a first message.
//!
//! Everything here is pure; the desktop feeds it what it already knows
//! (signed in? any user message anywhere?) and renders the answer. Only the
//! dismissal and the completion are persisted, in
//! `~/.cheaprouter/onboarding.json`, so the card stays gone once the user has
//! either finished or said "later".

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;
use crate::global_config::atomic_write_private;

/// What survives a restart.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct OnboardingState {
    /// The user closed the checklist before finishing it.
    #[serde(default)]
    pub dismissed: bool,
    /// Unix seconds when all three steps were first seen complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
}

/// The two steps, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    SignIn,
    FirstMessage,
}

impl Step {
    pub const ALL: [Step; 2] = [Step::SignIn, Step::FirstMessage];
}

/// How many steps the checklist has. Named so the desktop's array types read
/// as "the checklist" rather than as a bare number repeated at every call.
pub const STEP_COUNT: usize = Step::ALL.len();

/// Which steps are done, from what the app already knows.
pub fn steps(signed_in: bool, message_sent: bool) -> [(Step, bool); STEP_COUNT] {
    [(Step::SignIn, signed_in), (Step::FirstMessage, message_sent)]
}

pub fn all_done(steps: &[(Step, bool); STEP_COUNT]) -> bool {
    steps.iter().all(|(_, done)| *done)
}

/// The first step still to do, for the strip's single call to action.
pub fn next_step(steps: &[(Step, bool); STEP_COUNT]) -> Option<Step> {
    steps.iter().find(|(_, done)| !done).map(|(step, _)| *step)
}

/// Whether the checklist should be on screen: not dismissed, not already
/// recorded complete, and something left to do. A completion recorded
/// earlier keeps it hidden even if a step regresses (signing out), which is
/// the point of recording it — the user has seen the flow through once.
pub fn visible(state: &OnboardingState, steps: &[(Step, bool); STEP_COUNT]) -> bool {
    !state.dismissed && state.completed_at.is_none() && !all_done(steps)
}

/// Where the state lives.
pub fn config_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("onboarding.json"))
}

/// Load the stored state; absent or unreadable means "never dismissed".
pub fn load() -> OnboardingState {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(state: &OnboardingState) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut encoded =
        serde_json::to_string_pretty(state).context("could not encode onboarding state")?;
    encoded.push('\n');
    atomic_write_private(&path, encoded.as_bytes())
}

/// Record completion at `now`, once: a later call keeps the first stamp.
pub fn mark_completed(state: &mut OnboardingState, now: u64) -> bool {
    if state.completed_at.is_some() {
        return false;
    }
    state.completed_at = Some(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_matrix() {
        let fresh = OnboardingState::default();
        assert!(visible(&fresh, &steps(false, false)));
        assert!(visible(&fresh, &steps(true, false)));
        assert!(!visible(&fresh, &steps(true, true)));

        let dismissed = OnboardingState {
            dismissed: true,
            ..OnboardingState::default()
        };
        assert!(!visible(&dismissed, &steps(false, false)));

        let completed = OnboardingState {
            completed_at: Some(1),
            ..OnboardingState::default()
        };
        // Signing out afterwards does not bring the card back.
        assert!(!visible(&completed, &steps(false, true)));
    }

    #[test]
    fn next_step_is_the_first_unfinished_one() {
        assert_eq!(next_step(&steps(false, false)), Some(Step::SignIn));
        assert_eq!(next_step(&steps(true, false)), Some(Step::FirstMessage));
        assert_eq!(next_step(&steps(true, true)), None);
        assert!(all_done(&steps(true, true)));
    }

    #[test]
    fn installing_a_cli_is_not_on_the_path_to_a_first_message() {
        // The built-in agent needs nothing installed, so a signed-in user with
        // no CLI on the machine is one step from done.
        assert_eq!(Step::ALL.len(), 2);
        assert_eq!(next_step(&steps(true, false)), Some(Step::FirstMessage));
    }

    #[test]
    fn completion_is_recorded_once_and_round_trips() {
        let mut state = OnboardingState::default();
        assert!(mark_completed(&mut state, 100));
        assert!(!mark_completed(&mut state, 200));
        assert_eq!(state.completed_at, Some(100));

        let encoded = serde_json::to_string(&state).unwrap();
        let restored: OnboardingState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored, state);
        let legacy: OnboardingState = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy, OnboardingState::default());
    }
}
