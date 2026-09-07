//! Automatic group failover for the managed gateway.
//!
//! CC Switch fails over between *providers* behind a local reverse proxy it
//! runs itself. This app has no proxy — each CLI is pointed straight at an
//! endpoint — so the same idea lands one level up: the service already
//! reports per-group health from `/group-status`, and when the bound group
//! goes down the only fix is to bind a different one. Doing that by hand
//! means noticing the outage first, which is exactly the part a user cannot
//! do while waiting on a request.
//!
//! Three rules keep an automatic switch from being worse than the outage:
//!
//! * **Off unless asked.** Groups bill at different rates, so this is opt-in
//!   per platform and the app says which group it moved to.
//! * **Only "down" moves anything.** `degraded` and an unknown status leave
//!   the binding alone; a missing status item is never read as an outage.
//! * **Coming back needs to hold.** The original group is restored only
//!   after it reports healthy on [`RECOVERY_STREAK`] consecutive refreshes,
//!   so a flapping group cannot bounce the routing back and forth.
//!
//! This module decides; the desktop performs the switch through the same
//! path a manual pick uses.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;
use crate::client::{Group, GroupStatusItem};
use crate::global_config::atomic_write_private;

/// Healthy refreshes the preferred group needs before routing returns to it.
pub const RECOVERY_STREAK: u8 = 2;

/// One platform's failover setting.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlatformFailover {
    #[serde(default)]
    pub enabled: bool,
    /// The group the user actually chose — what routing returns to once it
    /// is healthy again. Seeded from the binding when failover is turned on
    /// and replaced by any later manual pick.
    #[serde(default)]
    pub preferred_group_id: Option<i64>,
}

/// Failover settings for every platform, keyed by platform id.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FailoverConfig {
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformFailover>,
}

impl FailoverConfig {
    pub fn enabled(&self, platform: &str) -> bool {
        self.platforms
            .get(platform)
            .is_some_and(|entry| entry.enabled)
    }

    pub fn preferred(&self, platform: &str) -> Option<i64> {
        self.platforms
            .get(platform)
            .and_then(|entry| entry.preferred_group_id)
    }

    /// Turn failover on or off, seeding the preferred group from the current
    /// binding so the first outage has somewhere to return to.
    pub fn set_enabled(&mut self, platform: &str, enabled: bool, bound: Option<i64>) {
        let entry = self.platforms.entry(platform.to_owned()).or_default();
        entry.enabled = enabled;
        if enabled {
            entry.preferred_group_id = entry.preferred_group_id.or(bound);
        }
    }

    /// Record a deliberate choice: whatever the user picks becomes the group
    /// to come back to.
    pub fn set_preferred(&mut self, platform: &str, group_id: Option<i64>) {
        let entry = self.platforms.entry(platform.to_owned()).or_default();
        entry.preferred_group_id = group_id;
    }
}

/// Where the settings live.
pub fn config_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("failover.json"))
}

/// Load the settings; absent or unreadable means "off everywhere".
pub fn load() -> FailoverConfig {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(config: &FailoverConfig) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut encoded =
        serde_json::to_string_pretty(config).context("could not encode failover settings")?;
    encoded.push('\n');
    atomic_write_private(&path, encoded.as_bytes())
}

/// How many consecutive healthy readings the preferred group has had.
/// Carried between refreshes by the caller; not persisted, so a restart
/// simply starts counting again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub preferred_up_streak: u8,
}

/// What should happen to a platform's binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The bound group is down; move to a healthy one.
    FailOver { from: i64, to: i64 },
    /// The group the user chose is healthy again; move back.
    FailBack { from: i64, to: i64 },
}

impl Decision {
    pub fn target(self) -> i64 {
        match self {
            Self::FailOver { to, .. } | Self::FailBack { to, .. } => to,
        }
    }
}

/// Everything one decision is made from.
pub struct Input<'a> {
    pub platform: &'a str,
    /// The group currently routing this platform.
    pub bound: Option<i64>,
    /// The group the user chose, when it differs from `bound`.
    pub preferred: Option<i64>,
    pub groups: &'a [Group],
    pub statuses: &'a [GroupStatusItem],
    pub history: &'a History,
    /// A switch is already running; nothing may start another.
    pub busy: bool,
}

/// The service's vocabulary is `up` / `degraded` / `down`, and anything it
/// has not measured yet is empty. Only an explicit `down` is an outage.
pub fn is_down(status: &str) -> bool {
    status.eq_ignore_ascii_case("down")
}

/// Only an explicit `up` is healthy enough to fail back to; `degraded` is not.
pub fn is_up(status: &str) -> bool {
    status.eq_ignore_ascii_case("up")
}

fn status_for(statuses: &[GroupStatusItem], group_id: i64) -> Option<&GroupStatusItem> {
    statuses
        .iter()
        .find(|status| status.group_id == group_id)
}

/// A group is a candidate when the account may use it, it belongs to the
/// platform, and it is not the one that just failed or known to be down.
fn usable(group: &Group, platform: &str, exclude: i64, statuses: &[GroupStatusItem]) -> bool {
    if group.id == exclude || group.platform != platform {
        return false;
    }
    // The service leaves `status` empty for ordinary groups and marks the
    // ones it has taken out of service.
    if !(group.status.is_empty() || group.status.eq_ignore_ascii_case("active")) {
        return false;
    }
    !status_for(statuses, group.id).is_some_and(|status| is_down(status.effective_status()))
}

/// The best group to move to: healthy first, then the most available, then
/// the fastest. An unmeasured group is still a candidate — unknown is not
/// the same as down — it just sorts below a group known to be up.
pub fn best_alternative(
    platform: &str,
    exclude: i64,
    groups: &[Group],
    statuses: &[GroupStatusItem],
) -> Option<i64> {
    groups
        .iter()
        .filter(|group| usable(group, platform, exclude, statuses))
        .min_by(|a, b| {
            let key = |group: &Group| {
                let status = status_for(statuses, group.id);
                let up = status.is_some_and(|status| is_up(status.effective_status()));
                let availability = status
                    .and_then(|status| status.availability_24h)
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.0);
                let latency = status
                    .and_then(|status| status.latency_ms)
                    .filter(|value| value.is_finite())
                    .unwrap_or(f64::MAX);
                (u8::from(!up), -availability, latency, group.id)
            };
            let (a, b) = (key(a), key(b));
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.total_cmp(&b.2))
                .then(a.3.cmp(&b.3))
        })
        .map(|group| group.id)
}

/// Decide what to do with one platform's binding, and what to remember.
///
/// Pure: the caller performs the switch and carries the returned history
/// into the next refresh.
pub fn decide(input: &Input) -> (Option<Decision>, History) {
    let unchanged = *input.history;
    let Some(bound) = input.bound else {
        return (None, History::default());
    };
    if input.busy {
        return (None, unchanged);
    }
    let preferred = input.preferred.unwrap_or(bound);

    // An outage on the group in use outranks everything else.
    let bound_down = status_for(input.statuses, bound)
        .is_some_and(|status| is_down(status.effective_status()));
    if bound_down {
        return match best_alternative(input.platform, bound, input.groups, input.statuses) {
            Some(to) => (
                Some(Decision::FailOver { from: bound, to }),
                History::default(),
            ),
            // Nothing healthy to move to: stay put rather than churn.
            None => (None, History::default()),
        };
    }

    if bound == preferred {
        return (None, History::default());
    }

    // Routing is on a fallback. Count healthy readings of the group the user
    // actually chose, and only go back once they hold.
    let Some(status) = status_for(input.statuses, preferred) else {
        // Not measured this round — neither progress nor a reset.
        return (None, unchanged);
    };
    if !is_up(status.effective_status()) {
        return (None, History::default());
    }
    let streak = unchanged.preferred_up_streak.saturating_add(1);
    if streak >= RECOVERY_STREAK {
        (
            Some(Decision::FailBack {
                from: bound,
                to: preferred,
            }),
            History::default(),
        )
    } else {
        (
            None,
            History {
                preferred_up_streak: streak,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: i64, platform: &str) -> Group {
        Group {
            id,
            name: format!("g{id}"),
            platform: platform.to_owned(),
            ..Group::default()
        }
    }

    fn status(group_id: i64, state: &str) -> GroupStatusItem {
        GroupStatusItem {
            group_id,
            group_name: format!("g{group_id}"),
            latest_status: state.to_owned(),
            stable_status: state.to_owned(),
            latency_ms: None,
            availability_24h: None,
            availability_7d: None,
        }
    }

    fn input<'a>(
        bound: Option<i64>,
        preferred: Option<i64>,
        groups: &'a [Group],
        statuses: &'a [GroupStatusItem],
        history: &'a History,
    ) -> Input<'a> {
        Input {
            platform: "anthropic",
            bound,
            preferred,
            groups,
            statuses,
            history,
            busy: false,
        }
    }

    #[test]
    fn nothing_happens_without_a_binding_or_while_busy() {
        let groups = vec![group(1, "anthropic"), group(2, "anthropic")];
        let statuses = vec![status(1, "down"), status(2, "up")];
        let history = History::default();
        assert_eq!(
            decide(&input(None, None, &groups, &statuses, &history)).0,
            None
        );
        let mut busy = input(Some(1), None, &groups, &statuses, &history);
        busy.busy = true;
        assert_eq!(decide(&busy).0, None);
    }

    #[test]
    fn fails_over_to_the_best_alternative() {
        let groups = vec![
            group(1, "anthropic"),
            group(2, "anthropic"),
            group(3, "anthropic"),
            group(4, "openai"),
        ];
        let statuses = vec![
            status(1, "down"),
            // Degraded is usable as a destination but ranks below healthy.
            status(2, "degraded"),
            status(3, "up"),
            status(4, "up"),
        ];
        let history = History::default();
        assert_eq!(
            decide(&input(Some(1), None, &groups, &statuses, &history)).0,
            Some(Decision::FailOver { from: 1, to: 3 })
        );
        // Another platform's healthy group is never borrowed.
        let only_openai = vec![group(1, "anthropic"), group(4, "openai")];
        assert_eq!(
            decide(&input(Some(1), None, &only_openai, &statuses, &history)).0,
            None
        );
    }

    #[test]
    fn availability_then_latency_break_ties() {
        let groups = vec![group(1, "anthropic"), group(2, "anthropic"), group(3, "anthropic")];
        let statuses = vec![
            status(1, "down"),
            GroupStatusItem {
                availability_24h: Some(97.0),
                latency_ms: Some(100.0),
                ..status(2, "up")
            },
            GroupStatusItem {
                availability_24h: Some(99.5),
                latency_ms: Some(900.0),
                ..status(3, "up")
            },
        ];
        let history = History::default();
        // More available wins even though it is slower.
        assert_eq!(
            decide(&input(Some(1), None, &groups, &statuses, &history)).0,
            Some(Decision::FailOver { from: 1, to: 3 })
        );
        let tied = vec![
            status(1, "down"),
            GroupStatusItem {
                availability_24h: Some(99.5),
                latency_ms: Some(400.0),
                ..status(2, "up")
            },
            GroupStatusItem {
                availability_24h: Some(99.5),
                latency_ms: Some(900.0),
                ..status(3, "up")
            },
        ];
        assert_eq!(
            decide(&input(Some(1), None, &groups, &tied, &history)).0,
            Some(Decision::FailOver { from: 1, to: 2 })
        );
    }

    #[test]
    fn an_unknown_status_is_not_an_outage() {
        let groups = vec![group(1, "anthropic"), group(2, "anthropic")];
        // Nothing reported for the bound group at all.
        let statuses = vec![status(2, "up")];
        let history = History::default();
        assert_eq!(
            decide(&input(Some(1), None, &groups, &statuses, &history)).0,
            None
        );
        // Neither is a degraded one.
        let degraded = vec![status(1, "degraded"), status(2, "up")];
        assert_eq!(
            decide(&input(Some(1), None, &groups, &degraded, &history)).0,
            None
        );
    }

    #[test]
    fn inactive_groups_are_never_chosen() {
        let mut disabled = group(2, "anthropic");
        disabled.status = "disabled".to_owned();
        let groups = vec![group(1, "anthropic"), disabled, group(3, "anthropic")];
        let statuses = vec![status(1, "down"), status(2, "up"), status(3, "up")];
        assert_eq!(
            decide(&input(Some(1), None, &groups, &statuses, &History::default())).0,
            Some(Decision::FailOver { from: 1, to: 3 })
        );
    }

    #[test]
    fn fails_back_only_after_two_consecutive_healthy_readings() {
        let groups = vec![group(1, "anthropic"), group(2, "anthropic")];
        let statuses = vec![status(1, "up"), status(2, "up")];
        // Routing sits on 2 after an outage; 1 is what the user chose.
        let (decision, history) = decide(&input(
            Some(2),
            Some(1),
            &groups,
            &statuses,
            &History::default(),
        ));
        assert_eq!(decision, None);
        assert_eq!(history.preferred_up_streak, 1);
        let (decision, history) = decide(&input(Some(2), Some(1), &groups, &statuses, &history));
        assert_eq!(decision, Some(Decision::FailBack { from: 2, to: 1 }));
        assert_eq!(history.preferred_up_streak, 0);
    }

    #[test]
    fn a_flapping_group_resets_the_streak() {
        let groups = vec![group(1, "anthropic"), group(2, "anthropic")];
        let healthy = vec![status(1, "up"), status(2, "up")];
        let flapping = vec![status(1, "degraded"), status(2, "up")];
        let unmeasured = vec![status(2, "up")];

        let (_, history) = decide(&input(Some(2), Some(1), &groups, &healthy, &History::default()));
        assert_eq!(history.preferred_up_streak, 1);
        let (decision, history) = decide(&input(Some(2), Some(1), &groups, &flapping, &history));
        assert_eq!(decision, None);
        assert_eq!(history.preferred_up_streak, 0, "degraded resets the count");

        let (_, history) = decide(&input(Some(2), Some(1), &groups, &healthy, &history));
        // A refresh that measured nothing holds the count where it was.
        let (decision, held) = decide(&input(Some(2), Some(1), &groups, &unmeasured, &history));
        assert_eq!(decision, None);
        assert_eq!(held.preferred_up_streak, history.preferred_up_streak);
    }

    #[test]
    fn a_fallback_that_also_goes_down_moves_again() {
        let groups = vec![group(1, "anthropic"), group(2, "anthropic"), group(3, "anthropic")];
        let statuses = vec![status(1, "down"), status(2, "down"), status(3, "up")];
        assert_eq!(
            decide(&input(Some(2), Some(1), &groups, &statuses, &History::default())).0,
            Some(Decision::FailOver { from: 2, to: 3 })
        );
    }

    #[test]
    fn config_round_trips_and_missing_file_is_default() {
        let mut config = FailoverConfig::default();
        assert!(!config.enabled("anthropic"));
        assert_eq!(config.preferred("anthropic"), None);

        config.set_enabled("anthropic", true, Some(7));
        assert!(config.enabled("anthropic"));
        assert_eq!(config.preferred("anthropic"), Some(7));
        // Turning it on again does not overwrite a later manual choice.
        config.set_preferred("anthropic", Some(9));
        config.set_enabled("anthropic", true, Some(7));
        assert_eq!(config.preferred("anthropic"), Some(9));

        let encoded = serde_json::to_string(&config).expect("encode");
        let decoded: FailoverConfig = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, config);
        let empty: FailoverConfig = serde_json::from_str("{}").expect("decode empty");
        assert_eq!(empty, FailoverConfig::default());
    }
}
