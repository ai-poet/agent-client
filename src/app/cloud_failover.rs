//! Settings → Cloud Account: group health and automatic failover.
//!
//! Fork addition. Health from `/group-status` decorates the group pickers
//! and, for a lane with failover on, moves routing off a group that is down
//! and back once the user's own pick has been healthy long enough
//! (`sub2api::failover`).

use std::time::{Duration, Instant};

use sub2api::model_routing::DOMESTIC_LANE;

use super::cloud_groups::{SwitchOrigin, cloud_lane, cloud_lanes};
use super::*;

/// How long fetched group health stays fresh. The account refresh cadence
/// (five minutes, plus every settled turn) calls in through this guard.
const GROUP_STATUS_TTL: Duration = Duration::from_secs(180);

/// The same, while a platform has automatic failover on: an outage is only
/// noticed as fast as health is read, and five minutes of failing requests
/// is exactly what the feature exists to avoid.
const FAILOVER_STATUS_TTL: Duration = Duration::from_secs(60);

impl Waku {
    /// Fetch group health when stale. Failures stay silent — this decorates
    /// the switcher menus; the Model Plaza is the surface with error states.
    pub(super) fn refresh_cloud_group_status(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let ttl = if self.cloud_failover_active() {
            FAILOVER_STATUS_TTL
        } else {
            GROUP_STATUS_TTL
        };
        if self
            .cloud_account
            .group_status_at
            .is_some_and(|at| at.elapsed() < ttl)
        {
            return;
        }
        // Stamped before the fetch so overlapping refreshes collapse and a
        // failing endpoint is retried at the TTL, not every render.
        self.cloud_account.group_status_at = Some(Instant::now());

        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    sub2api::refresh_if_needed(&mut credentials)?;
                    let statuses = sub2api::Client::new(credentials.endpoint.clone())
                        .group_statuses(&credentials.access_token)?;
                    anyhow::Ok(statuses)
                })
                .await;
            if let Ok(statuses) = fetched {
                let _ = this.update(cx, |this, cx| {
                    this.cloud_account.group_status = statuses;
                    this.evaluate_cloud_failover(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Act on the health just fetched: move off a group that is down, and
    /// move back once the user's own choice has been healthy long enough.
    ///
    /// One switch per pass — [`Self::select_cloud_group`] returns early while
    /// a switch is running, and the decision refuses to start another — so a
    /// platform-wide outage cannot set off a storm of rebinds.
    fn evaluate_cloud_failover(&mut self, cx: &mut Context<Self>) {
        use sub2api::failover::{Decision, Input};

        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let catalog = &self.model_plaza.items;
        // The Chinese models' lane falls back on its own (a spent
        // subscription hands its models to pay-as-you-go in routing), so it
        // takes no part here.
        // Before the catalog lands their groups read as `openai`, so Codex
        // waits for it rather than fail over onto one of them.
        let platforms: Vec<String> = cloud_lanes(&self.cloud_account.groups, catalog)
            .into_iter()
            .filter(|platform| platform != DOMESTIC_LANE)
            .filter(|platform| !catalog.is_empty() || platform != "openai")
            .filter(|platform| self.cloud_account.failover.enabled(platform))
            .collect();
        // Groups filed by lane, not platform, so a CLI never fails over to a
        // group of the Chinese models that happens to share its platform.
        let laned_groups: Vec<sub2api::client::Group> = self
            .cloud_account
            .groups
            .iter()
            .map(|group| sub2api::client::Group {
                platform: cloud_lane(group, catalog),
                ..group.clone()
            })
            .collect();
        for platform in platforms {
            let history = self
                .cloud_account
                .failover_history
                .get(&platform)
                .copied()
                .unwrap_or_default();
            // A pick from before the lanes split — one of the Chinese models'
            // groups chosen for Codex, since moved off — is nothing to fail
            // back to: going back would break GPT again.
            let preferred = self
                .cloud_account
                .failover
                .preferred(&platform)
                .filter(|id| {
                    laned_groups
                        .iter()
                        .any(|group| group.id == *id && group.platform == platform)
                });
            let (decision, history) = sub2api::failover::decide(&Input {
                platform: &platform,
                bound: sub2api::bound_group_for_platform(&credentials, &platform),
                preferred,
                groups: &laned_groups,
                statuses: &self.cloud_account.group_status,
                history: &history,
                busy: self.cloud_account.busy,
            });
            self.cloud_account
                .failover_history
                .insert(platform.clone(), history);
            let Some(decision) = decision else {
                continue;
            };
            let name = |id: i64| {
                self.cloud_account
                    .groups
                    .iter()
                    .find(|group| group.id == id)
                    .map(|group| group.name.clone())
                    .unwrap_or_else(|| id.to_string())
            };
            let message = match decision {
                Decision::FailOver { from, to } => {
                    self.cloud_account.failed_over.insert(platform.clone(), from);
                    tr!(
                        "cloud.failover_switched",
                        from = name(from),
                        to = name(to)
                    )
                }
                Decision::FailBack { to, .. } => {
                    self.cloud_account.failed_over.remove(&platform);
                    tr!("cloud.failover_restored", group = name(to))
                }
            };
            self.select_cloud_group_with_origin(
                platform,
                Some(decision.target()),
                SwitchOrigin::Auto,
                cx,
            );
            self.show_toast(message);
            // The binding has changed under every other platform's input;
            // the next refresh re-decides for them.
            break;
        }
    }

    /// Any platform watching for outages.
    fn cloud_failover_active(&self) -> bool {
        self.cloud_account
            .failover
            .platforms
            .values()
            .any(|platform| platform.enabled)
    }

    /// Keep reading group health while failover is on.
    ///
    /// The account refresh runs every five minutes, which is far too coarse
    /// to catch an outage the user is sitting through. One loop serves every
    /// platform and ends as soon as the last toggle goes off, so nothing
    /// polls for a feature nobody enabled.
    pub(super) fn poll_cloud_failover(&mut self, cx: &mut Context<Self>) {
        if self.cloud_account.failover_polling || !self.cloud_failover_active() {
            return;
        }
        self.cloud_account.failover_polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FAILOVER_STATUS_TTL).await;
                let running = this.update(cx, |this, cx| {
                    if !this.cloud_failover_active() || this.cloud_account.credentials.is_none() {
                        this.cloud_account.failover_polling = false;
                        return false;
                    }
                    this.refresh_cloud_group_status(cx);
                    true
                });
                match running {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => break,
                }
            }
        })
        .detach();
    }

    /// Turn automatic failover on or off for one platform. Turning it on
    /// remembers the group in use as the one to come back to.
    pub(super) fn set_cloud_failover(
        &mut self,
        platform: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let bound = self
            .cloud_account
            .credentials
            .as_ref()
            .and_then(|credentials| sub2api::bound_group_for_platform(credentials, &platform));
        self.cloud_account
            .failover
            .set_enabled(&platform, enabled, bound);
        if !enabled {
            self.cloud_account.failover_history.remove(&platform);
            self.cloud_account.failed_over.remove(&platform);
        }
        if let Err(error) = sub2api::failover::save(&self.cloud_account.failover) {
            self.show_toast(format!("{error:#}"));
        }
        if enabled {
            // Read health now rather than at the next five-minute tick, so
            // turning this on acts on the outage the user is looking at.
            self.cloud_account.group_status_at = None;
            self.refresh_cloud_group_status(cx);
            self.poll_cloud_failover(cx);
        }
        cx.notify();
    }
}
