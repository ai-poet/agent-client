//! A new reasoning effort reaching Claude Code.
//!
//! Fork addition. Claude Code takes its effort only as the `--effort` launch
//! flag: its control protocol has a model setter but no effort setter
//! (`set_max_thinking_tokens` sets a budget, not the effort). So an effort
//! picked for a running session used to look accepted and change nothing
//! until the process happened to be restarted.
//!
//! The relaunch happens here, at the start of the next submission, and only
//! when the session is idle: the CLI is closed and the submission starts a
//! fresh one with `--resume` and the new `--effort`, so the conversation
//! carries on. Deciding and restarting in the same UI step that starts the
//! turn leaves no window for a queued message to land on a dying process,
//! and a running turn is never interrupted. It catches every path that
//! changes the effort — the slider, the cycle shortcut, a model switch that
//! restores remembered traits.
//!
//! The effort a runtime was launched with is remembered in memory. A runtime
//! this client did not see start (one re-attached after a restart) is taken
//! to match the current effort.

use super::*;

/// The effort a live Claude runtime was started with.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct LaunchedEffort {
    effort: Option<String>,
    /// The pending effort a "waits for background work" notice was already
    /// shown for, so it is said once per change rather than per message.
    notified_for: Option<Option<String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Relaunch {
    /// Nothing to do.
    Keep,
    /// Remember `effective` as what the runtime runs with.
    Record,
    /// Close the runtime; this submission starts it with the new effort.
    Relaunch,
    /// The effort changed, but closing the CLI would end background work it
    /// is still running. Try again with the next message.
    WaitForBackgroundWork,
}

/// Decide what a submission does about the effort.
pub(super) fn decide(
    provider: ProviderKind,
    runtime_live: bool,
    busy: bool,
    launched: Option<&Option<String>>,
    effective: &Option<String>,
    live_background_work: bool,
) -> Relaunch {
    // Every other transport takes the effort per turn or through its own
    // setter (`apply_options`).
    if provider != ProviderKind::Claude {
        return Relaunch::Keep;
    }
    if !runtime_live {
        return Relaunch::Record;
    }
    if busy {
        return Relaunch::Keep;
    }
    let Some(launched) = launched else {
        return Relaunch::Record;
    };
    if launched == effective {
        return Relaunch::Keep;
    }
    if live_background_work {
        return Relaunch::WaitForBackgroundWork;
    }
    Relaunch::Relaunch
}

impl Waku {
    /// Called as a submission starts, before the driver start request is
    /// built: relaunches an idle Claude runtime whose effort is stale.
    pub(super) fn relaunch_for_effort(&mut self, session_id: Uuid) {
        // A goal operation is already starting this session's provider with
        // the current options, or a submission or response fork is on its
        // way to the runtime that is live now.
        if self.goal_runtime_starts.contains(&session_id)
            || self.submission_preparations.contains(&session_id)
            || self.response_fork_preparations.contains_key(&session_id)
        {
            return;
        }
        let Some(session) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        let provider = session.provider;
        let busy = session.is_busy() || session.active_turn_id().is_some();
        // The same resolution the start request uses, so "launched with" and
        // "would launch with" compare like for like.
        let effective = self.session_options(session).reasoning_effort;
        let runtime_live = self.runtimes.contains_key(&session_id);
        let live_background_work = self.session_has_live_background_work(session_id);
        let launched = self
            .effort_launches
            .get(&session_id)
            .map(|launched| &launched.effort);
        match decide(
            provider,
            runtime_live,
            busy,
            launched,
            &effective,
            live_background_work,
        ) {
            Relaunch::Keep => {}
            Relaunch::Record => {
                self.effort_launches.insert(
                    session_id,
                    LaunchedEffort {
                        effort: effective,
                        notified_for: None,
                    },
                );
            }
            Relaunch::Relaunch => {
                self.reset_session_runtime(session_id);
                self.effort_launches.insert(
                    session_id,
                    LaunchedEffort {
                        effort: effective,
                        notified_for: None,
                    },
                );
            }
            Relaunch::WaitForBackgroundWork => {
                let Some(launched) = self.effort_launches.get_mut(&session_id) else {
                    return;
                };
                if launched.notified_for.as_ref() != Some(&effective) {
                    launched.notified_for = Some(effective);
                    self.show_toast(tr!("effort_panel.waits_for_background"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(effort: &str) -> Option<String> {
        Some(effort.to_owned())
    }

    #[test]
    fn only_claude_relaunches_for_an_effort() {
        for provider in [
            ProviderKind::Codex,
            ProviderKind::Native,
            ProviderKind::Grok,
            ProviderKind::Kimi,
        ] {
            assert_eq!(
                decide(provider, true, false, Some(&some("high")), &some("max"), false),
                Relaunch::Keep,
                "{provider:?}"
            );
        }
    }

    #[test]
    fn a_runtime_about_to_start_is_recorded_with_what_it_starts_with() {
        assert_eq!(
            decide(ProviderKind::Claude, false, false, None, &some("max"), false),
            Relaunch::Record
        );
        assert_eq!(
            decide(
                ProviderKind::Claude,
                false,
                false,
                Some(&some("low")),
                &some("max"),
                false
            ),
            Relaunch::Record
        );
    }

    #[test]
    fn a_running_turn_is_never_interrupted() {
        assert_eq!(
            decide(
                ProviderKind::Claude,
                true,
                true,
                Some(&some("high")),
                &some("max"),
                false
            ),
            Relaunch::Keep
        );
    }

    #[test]
    fn a_runtime_this_client_did_not_start_is_taken_as_current() {
        assert_eq!(
            decide(ProviderKind::Claude, true, false, None, &some("max"), false),
            Relaunch::Record
        );
    }

    #[test]
    fn an_unchanged_effort_keeps_the_runtime() {
        assert_eq!(
            decide(
                ProviderKind::Claude,
                true,
                false,
                Some(&some("high")),
                &some("high"),
                false
            ),
            Relaunch::Keep
        );
        assert_eq!(
            decide(ProviderKind::Claude, true, false, Some(&None), &None, false),
            Relaunch::Keep
        );
    }

    #[test]
    fn a_changed_effort_relaunches_an_idle_runtime() {
        assert_eq!(
            decide(
                ProviderKind::Claude,
                true,
                false,
                Some(&some("high")),
                &some("max"),
                false
            ),
            Relaunch::Relaunch
        );
        // Back to "whatever the CLI is configured with" is a change too.
        assert_eq!(
            decide(
                ProviderKind::Claude,
                true,
                false,
                Some(&some("high")),
                &None,
                false
            ),
            Relaunch::Relaunch
        );
    }

    #[test]
    fn live_background_work_postpones_the_relaunch() {
        assert_eq!(
            decide(
                ProviderKind::Claude,
                true,
                false,
                Some(&some("high")),
                &some("max"),
                true
            ),
            Relaunch::WaitForBackgroundWork
        );
    }
}
