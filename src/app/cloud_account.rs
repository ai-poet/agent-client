//! Settings → Cloud Account.
//!
//! Fork addition. All the logic lives in the `sub2api` crate, which has no
//! GPUI dependency and is unit-tested on its own; this file is the view and the
//! plumbing that runs that logic off the UI thread.
//!
//! Routing is applied by writing each CLI's own global configuration —
//! [`Waku::apply_cloud_routing`] → `sub2api::global_config::reconcile` — the
//! moment the state changes (sign-in, group switch, toggle, sign-out). The
//! daemon carries no routing state.

use std::time::{Duration, Instant};

use super::providers_page::card_button;
use super::*;

/// How long the loopback listener waits for the browser before giving up.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// How long fetched group health stays fresh. The account refresh cadence
/// (five minutes, plus every settled turn) calls in through this guard.
const GROUP_STATUS_TTL: Duration = Duration::from_secs(180);

/// The same, while a platform has automatic failover on: an outage is only
/// noticed as fast as health is read, and five minutes of failing requests
/// is exactly what the feature exists to avoid.
const FAILOVER_STATUS_TTL: Duration = Duration::from_secs(60);

/// Balance polling after the top-up page is opened.
const TOP_UP_POLL_INTERVAL: Duration = Duration::from_secs(15);
const TOP_UP_POLL_ATTEMPTS: usize = 8;

/// View state for the Cloud Account page.
///
/// Owned by [`Waku`] as a single field so the fork adds one line to the app
/// struct rather than a scattering of flags.
#[derive(Default)]
pub(super) struct CloudAccountState {
    /// Stored session, or `None` when signed out.
    pub credentials: Option<sub2api::Credentials>,
    /// Account summary from `/auth/me`; `None` until the first fetch lands.
    pub user: Option<sub2api::client::User>,
    /// Whether agents should be routed through the gateway.
    pub routing_enabled: bool,
    /// A sign-in or fetch is in flight.
    pub pending: bool,
    /// Last failure, shown inline rather than as a transient toast so the user
    /// can still read it afterwards.
    pub error: Option<String>,
    /// Groups this account may route through.
    pub groups: Vec<sub2api::client::Group>,
    /// Group health from `/group-status`, decorating the switcher UIs.
    pub group_status: Vec<sub2api::client::GroupStatusItem>,
    /// Last group-health fetch attempt, for the TTL guard.
    pub group_status_at: Option<Instant>,
    /// Referral code and share link.
    pub referral: Option<sub2api::client::ReferralInfo>,
    /// A group switch or redemption is in flight.
    pub busy: bool,
    /// Per-platform automatic failover settings, loaded at startup.
    pub failover: sub2api::failover::FailoverConfig,
    /// How long each platform's preferred group has looked healthy, so a
    /// flapping group cannot bounce the routing back and forth.
    pub failover_history: std::collections::BTreeMap<String, sub2api::failover::History>,
    /// Platforms currently routed somewhere other than the user's choice,
    /// and the group they came from.
    pub failed_over: std::collections::BTreeMap<String, i64>,
    /// The failover health poll is running; one loop serves every platform.
    pub failover_polling: bool,
    /// Which of the service's domains the CLIs are pointed at.
    pub gateway_origin: sub2api::gateway_origin::GatewayOriginConfig,
    /// The last domain measurement's progress and results.
    pub origin_test: Option<OriginTest>,
    /// Bumped per measurement; a result from a superseded run is discarded.
    pub origin_generation: u64,
    /// The once-per-run check of the chosen domain has been made.
    pub origin_checked: bool,
    /// Field for adding a domain, built on first use — creating a
    /// `TextInput` needs a `Window`, which render does not have.
    pub origin_input: Option<Entity<TextInput>>,
}

/// A gateway-domain measurement. Which run it belongs to is tracked by
/// `origin_generation` on the state, so a superseded result is discarded.
#[derive(Default)]
pub(super) struct OriginTest {
    pub running: bool,
    pub results: Vec<sub2api::speedtest::CandidateResult>,
}

/// Whether a group switch was asked for or decided automatically. A manual
/// pick also records the new preference; an automatic one must not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SwitchOrigin {
    Manual,
    Auto,
}

/// What a card's button does.
#[derive(Clone, Copy)]
enum CloudAction {
    SignIn,
    SignOut,
    SetRouting(bool),
    TopUp,
    CopyReferral,
}

impl Waku {
    /// Load the stored session at startup and refresh the account summary.
    pub(super) fn load_cloud_account(&mut self, cx: &mut Context<Self>) {
        self.migrate_legacy_routing_transport();
        self.cloud_account.failover = sub2api::failover::load();
        self.cloud_account.gateway_origin = sub2api::gateway_origin::load();
        let Some(credentials) = sub2api::Credentials::load() else {
            // Reconcile anyway: custom endpoints apply while signed out, and
            // a takeover left behind by a wiped login must be restored.
            self.apply_cloud_routing();
            return;
        };
        self.cloud_account.routing_enabled = !credentials.routing_disabled;
        self.cloud_account.credentials = Some(credentials);
        // Startup reconcile: an app update may write the files differently,
        // and any drift between the ledger and the live configs heals here.
        self.apply_cloud_routing();
        self.refresh_cloud_account(cx);
        self.load_cloud_details(cx);
        self.poll_cloud_failover(cx);
    }

    /// Drain the injection-era routing transport out of daemon settings.
    ///
    /// Older builds carried the gateway and custom-endpoint configuration in
    /// `DaemonSettings.extra`; routing is desktop-local now. Runs every
    /// startup but only writes when a legacy key was actually present.
    fn migrate_legacy_routing_transport(&mut self) {
        let mut settings = self.state.daemon_settings();
        let migrated_custom = sub2api::custom_api::migrate_from_extra(&mut settings.extra);
        let had_gateway = settings.extra.remove("sub2apiCloudGateway").is_some();
        if migrated_custom.is_none() && !had_gateway {
            return;
        }
        if let Some(config) = migrated_custom
            && sub2api::custom_api::config_path().is_some_and(|path| !path.exists())
            && let Err(error) = sub2api::custom_api::save(&config)
        {
            self.show_toast(format!("{error:#}"));
        }
        self.state.apply_daemon_settings(settings);
        self.save();
    }

    /// Fold a background task's renewed session into the in-memory one.
    ///
    /// Only the token fields are taken. The background clone was made before
    /// the task ran, so adopting it wholesale would silently roll back
    /// anything the user did meanwhile — picking a group rebinds `api_key`,
    /// and a balance poll finishing a moment later must not undo that.
    pub(super) fn adopt_cloud_tokens(&mut self, renewed: sub2api::Credentials) {
        match self.cloud_account.credentials.as_mut() {
            Some(existing) if existing.endpoint == renewed.endpoint => {
                existing.access_token = renewed.access_token;
                existing.refresh_token = renewed.refresh_token;
                existing.expires_at = renewed.expires_at;
            }
            _ => self.cloud_account.credentials = Some(renewed),
        }
    }

    /// Refresh the account summary, renewing the access token if it is due.
    pub(super) fn refresh_cloud_account(&mut self, cx: &mut Context<Self>) {
        // Announcements and group health ride the same cadence, each behind
        // its own TTL guard so the turn-end balance refresh does not hammer
        // those endpoints.
        self.refresh_cloud_announcements(false, cx);
        self.refresh_cloud_group_status(cx);
        self.verify_gateway_origin(cx);
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.cloud_account.pending {
            return;
        }
        self.cloud_account.pending = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    sub2api::refresh_if_needed(&mut credentials)?;
                    let user = sub2api::Client::new(credentials.endpoint.clone())
                        .me(&credentials.access_token)?;
                    anyhow::Ok((credentials, user))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.pending = false;
                match fetched {
                    Ok((credentials, user)) => {
                        this.adopt_cloud_tokens(credentials);
                        this.cloud_account.user = Some(user);
                        this.cloud_account.error = None;
                    }
                    // The service refused the refresh token itself: no retry
                    // will bring the session back, so stop pretending.
                    Err(error) if sub2api::session_ended(&error) => this.end_cloud_session(cx),
                    Err(error) => this.cloud_account.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

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
        let platforms: Vec<String> = cloud_platforms(&self.cloud_account.groups)
            .into_iter()
            .filter(|platform| self.cloud_account.failover.enabled(platform))
            .collect();
        for platform in platforms {
            let history = self
                .cloud_account
                .failover_history
                .get(&platform)
                .copied()
                .unwrap_or_default();
            let (decision, history) = sub2api::failover::decide(&Input {
                platform: &platform,
                bound: sub2api::bound_group_for_platform(&credentials, &platform),
                preferred: self.cloud_account.failover.preferred(&platform),
                groups: &self.cloud_account.groups,
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

    // ── Service domains ────────────────────────────────────────────────

    /// Build (once) the field for adding a service domain.
    fn ensure_origin_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = self.cloud_account.origin_input.clone() {
            return input;
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .select_all_on_focus_click()
                .placeholder(tr!("cloud.origin_placeholder"))
        });
        cx.subscribe(
            &input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.add_gateway_origin(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.cloud_account.origin_input = Some(input.clone());
        input
    }

    /// Reveal the domain field and put the cursor in it.
    pub(super) fn open_gateway_origin_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.ensure_origin_input(window, cx);
        let focus = input.read(cx).focus();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Save the origin choice and re-point every routed CLI at it.
    fn persist_gateway_origin(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = sub2api::gateway_origin::save(&self.cloud_account.gateway_origin) {
            self.show_toast(format!("{error:#}"));
        }
        self.apply_cloud_routing();
        // The model list is served by the domain that was just swapped.
        self.refresh_provider_detection(None);
        cx.notify();
    }

    pub(super) fn add_gateway_origin(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.cloud_account.origin_input.clone() else {
            return;
        };
        let raw = input.read(cx).content().trim().to_owned();
        if raw.is_empty() {
            return;
        }
        match self.cloud_account.gateway_origin.add(&raw) {
            Ok(true) => {
                input.update(cx, |input, cx| input.clear(cx));
                self.cloud_account.error = None;
                // Adding a domain does not route to it; measuring comes
                // first, then a deliberate choice.
                if let Err(error) =
                    sub2api::gateway_origin::save(&self.cloud_account.gateway_origin)
                {
                    self.show_toast(format!("{error:#}"));
                }
                cx.notify();
            }
            Ok(false) => {
                self.cloud_account.error = Some(tr!("cloud.origin_duplicate"));
                cx.notify();
            }
            Err(error) => {
                self.cloud_account.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    pub(super) fn remove_gateway_origin(&mut self, origin: String, cx: &mut Context<Self>) {
        let routed = self.cloud_account.gateway_origin.origin();
        self.cloud_account.gateway_origin.remove(&origin);
        self.cloud_account.origin_test = None;
        if routed.as_deref() == Some(origin.as_str()) {
            self.persist_gateway_origin(cx);
        } else if let Err(error) =
            sub2api::gateway_origin::save(&self.cloud_account.gateway_origin)
        {
            self.show_toast(format!("{error:#}"));
        }
        cx.notify();
    }

    pub(super) fn select_gateway_origin(&mut self, origin: String, cx: &mut Context<Self>) {
        if self.cloud_account.gateway_origin.origin().as_deref() == Some(origin.as_str()) {
            return;
        }
        if let Err(error) = self.cloud_account.gateway_origin.select(&origin) {
            self.show_toast(format!("{error:#}"));
            return;
        }
        self.persist_gateway_origin(cx);
    }

    pub(super) fn set_gateway_origin_auto_select(&mut self, on: bool, cx: &mut Context<Self>) {
        self.cloud_account.gateway_origin.auto_select = on;
        if let Err(error) = sub2api::gateway_origin::save(&self.cloud_account.gateway_origin) {
            self.show_toast(format!("{error:#}"));
        }
        cx.notify();
    }

    /// Check the chosen domain once per run.
    ///
    /// A domain that stopped serving this account would break every CLI with
    /// no visible cause, so the fallback to the build's primary happens
    /// whether or not automatic selection is on.
    fn verify_gateway_origin(&mut self, cx: &mut Context<Self>) {
        if self.cloud_account.origin_checked {
            return;
        }
        let nothing_to_check = self.cloud_account.gateway_origin.effective_candidates().len() < 2;
        let no_key = self
            .cloud_account
            .credentials
            .as_ref()
            .is_none_or(|credentials| {
                sub2api::gateway_origin::probe_target(credentials).is_none()
            });
        if nothing_to_check || no_key {
            return;
        }
        self.cloud_account.origin_checked = true;
        self.run_gateway_origin_test(true, cx);
    }

    /// Measure every service domain, and act on the result.
    pub(super) fn run_gateway_origin_test(&mut self, automatic: bool, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let config = self.cloud_account.gateway_origin.clone();
        if sub2api::gateway_origin::probe_target(&credentials).is_none() {
            if !automatic {
                self.show_toast(tr!("cloud.origin_no_key"));
            }
            return;
        }
        self.cloud_account.origin_generation += 1;
        let generation = self.cloud_account.origin_generation;
        self.cloud_account.origin_test = Some(OriginTest {
            running: true,
            results: Vec::new(),
        });
        cx.notify();

        let routed = config.origin();
        let auto_select = config.auto_select;
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    sub2api::gateway_origin::test_origins(
                        &config,
                        &credentials,
                        sub2api::speedtest::DEFAULT_TIMEOUT_SECS,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.cloud_account.origin_generation != generation {
                    return;
                }
                this.cloud_account.origin_test = Some(OriginTest {
                    running: false,
                    results: results.clone(),
                });
                cx.notify();

                let fastest = sub2api::speedtest::fastest_ok(&results)
                    .map(|index| results[index].url.clone());
                let routed_ok = routed.as_ref().is_some_and(|current| {
                    results
                        .iter()
                        .any(|candidate| candidate.url == *current && candidate.is_ok())
                });
                // A domain that no longer answers is left in place only when
                // there is nothing better to move to.
                if !routed_ok
                    && let Some(fallback) = fastest.clone()
                    && routed.as_deref() != Some(fallback.as_str())
                {
                    this.select_gateway_origin(fallback.clone(), cx);
                    this.show_toast(tr!("cloud.origin_fallback", url = fallback));
                    return;
                }
                if !auto_select {
                    if !automatic && fastest.is_none() {
                        this.show_toast(tr!("cloud.origin_no_ok"));
                    }
                    return;
                }
                match fastest {
                    Some(best) if routed.as_deref() != Some(best.as_str()) => {
                        this.select_gateway_origin(best.clone(), cx);
                        this.show_toast(tr!("cloud.origin_auto_selected", url = best));
                    }
                    Some(_) => {}
                    None if !automatic => this.show_toast(tr!("cloud.origin_no_ok")),
                    None => {}
                }
            });
        })
        .detach();
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

    /// Open the browser and wait for the sign-in redirect.
    pub(super) fn start_cloud_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.cloud_account.pending {
            return;
        }
        self.cloud_account.pending = true;
        self.cloud_account.error = None;
        cx.notify();

        let endpoint = sub2api::brand::MANAGED_SERVICE_URL.to_owned();
        cx.spawn(async move |this, cx| {
            // Bind before opening the browser: a redirect arriving at a closed
            // port is unrecoverable, and the user would see the login page fail
            // with no way back.
            let flow = match sub2api::auth::LoginFlow::start(&endpoint) {
                Ok(flow) => flow,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.cloud_account.pending = false;
                        this.cloud_account.error = Some(format!("{error:#}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let url = flow.login_url();
            let _ = this.update(cx, |_, cx| cx.open_url(&url));

            let result = cx
                .background_executor()
                .spawn(async move {
                    let credentials = flow.wait(SIGN_IN_TIMEOUT)?;
                    credentials.save()?;
                    let user = sub2api::Client::new(credentials.endpoint.clone())
                        .me(&credentials.access_token)?;
                    anyhow::Ok((credentials, user))
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.cloud_account.pending = false;
                match result {
                    Ok((credentials, user)) => {
                        this.cloud_account.credentials = Some(credentials);
                        this.cloud_account.user = Some(user);
                        this.cloud_account.error = None;
                        // Signing in is an explicit request to use the service,
                        // so routing starts on rather than needing a second
                        // switch nobody would find.
                        this.cloud_account.routing_enabled = true;
                        this.apply_cloud_routing();
                        this.load_cloud_details(cx);
                    }
                    Err(error) => this.cloud_account.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Forget the session and stop routing.
    pub(super) fn sign_out_cloud(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = sub2api::Credentials::clear() {
            self.show_toast(format!("{error:#}"));
        }
        self.cloud_account.credentials = None;
        self.cloud_account.user = None;
        self.cloud_account.routing_enabled = false;
        self.cloud_account.error = None;
        self.apply_cloud_routing();
        cx.notify();
    }

    /// The stored session can no longer be renewed. Forget it the way a
    /// sign-out does - credentials gone, the CLIs' own configuration
    /// restored - and say why, so the user sees "sign in again" rather than
    /// a footer that still claims to be signed in while every request fails.
    pub(super) fn end_cloud_session(&mut self, cx: &mut Context<Self>) {
        self.sign_out_cloud(cx);
        self.cloud_account.error = Some(tr!("cloud.session_expired"));
        self.show_toast(tr!("cloud.session_expired"));
        cx.notify();
    }

    /// Fetch the things that change rarely: groups and referral. Model
    /// pricing lives on its own page (the Model Plaza) and loads there.
    ///
    /// Kept out of [`Self::refresh_cloud_account`], which runs every five
    /// minutes for the balance.
    pub(super) fn load_cloud_details(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let client = sub2api::authenticated(&mut credentials)?;
                    let token = credentials.access_token.as_str();
                    // Each of these is optional decoration: a deployment that
                    // has referrals switched off must not blank the whole page.
                    anyhow::Ok((
                        credentials.clone(),
                        client.available_groups(token).unwrap_or_default(),
                        client.referral_info(token).ok(),
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Ok((credentials, groups, referral)) = loaded else {
                    // A failed renewal here is not worth a banner: the balance
                    // poll reports the same problem, and this data is optional.
                    return;
                };
                this.adopt_cloud_tokens(credentials);
                this.cloud_account.groups = groups;
                this.cloud_account.referral = referral;
                this.ensure_cloud_group_bindings(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Bind every platform that has groups but no selection yet to its first
    /// group. There is no "account default" choice: the server-side fallback
    /// is invisible to the user, so an explicit group keeps the routing (and
    /// the switcher menus) honest. Runs after each group fetch, which also
    /// migrates accounts signed in before this rule existed.
    fn ensure_cloud_group_bindings(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.cloud_account.busy {
            return;
        }
        let pending: Vec<(String, i64)> = cloud_platforms(&self.cloud_account.groups)
            .into_iter()
            .filter(|platform| {
                sub2api::bound_group_for_platform(&credentials, platform).is_none()
            })
            .filter_map(|platform| {
                self.cloud_account
                    .groups
                    .iter()
                    .find(|group| group.platform == platform)
                    .map(|group| (platform, group.id))
            })
            .collect();
        if pending.is_empty() {
            return;
        }
        self.cloud_account.busy = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    for (platform, group_id) in &pending {
                        sub2api::bind_group_for_platform(
                            &mut credentials,
                            platform,
                            Some(*group_id),
                        )?;
                    }
                    anyhow::Ok(credentials)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.busy = false;
                match result {
                    Ok(renewed) => {
                        this.cloud_account.credentials = Some(renewed);
                        this.apply_cloud_routing();
                    }
                    // No toast: nothing was user-initiated here. The balance
                    // poll surfaces a broken session on its own, and the next
                    // group fetch retries the binding.
                    Err(error) => this.cloud_account.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Bind a platform's routing to a group (or back to the account default),
    /// reusing an existing key for it when there is one.
    pub(super) fn select_cloud_group(
        &mut self,
        platform: String,
        group_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.select_cloud_group_with_origin(platform, group_id, SwitchOrigin::Manual, cx);
    }

    /// [`Self::select_cloud_group`], distinguishing a deliberate pick from an
    /// automatic one: a pick is also a statement about which group the user
    /// wants, so it becomes the group failover returns to.
    pub(super) fn select_cloud_group_with_origin(
        &mut self,
        platform: String,
        group_id: Option<i64>,
        origin: SwitchOrigin,
        cx: &mut Context<Self>,
    ) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.cloud_account.busy {
            return;
        }
        if origin == SwitchOrigin::Manual {
            self.cloud_account.failed_over.remove(&platform);
            self.cloud_account.failover_history.remove(&platform);
            if self.cloud_account.failover.enabled(&platform) {
                self.cloud_account.failover.set_preferred(&platform, group_id);
                if let Err(error) = sub2api::failover::save(&self.cloud_account.failover) {
                    self.show_toast(format!("{error:#}"));
                }
            }
        }
        self.cloud_account.busy = true;
        self.cloud_account.error = None;
        cx.notify();

        // The CLI whose route this group drives; its model list follows
        // the group (Codex asks the gateway for a manifest with the key).
        let probe_scope = match platform.as_str() {
            "openai" => Some(ProviderKind::Codex),
            "anthropic" => Some(ProviderKind::Claude),
            _ => None,
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    sub2api::bind_group_for_platform(&mut credentials, &platform, group_id)?;
                    anyhow::Ok(credentials)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.busy = false;
                match result {
                    Ok(renewed) => {
                        this.cloud_account.credentials = Some(renewed);
                        this.apply_cloud_routing();
                        // Re-read the CLI's catalog under the new route;
                        // routing has already dropped Codex's manifest cache.
                        this.refresh_provider_detection(probe_scope);
                        let name = group_id
                            .and_then(|id| {
                                this.cloud_account
                                    .groups
                                    .iter()
                                    .find(|group| group.id == id)
                                    .map(|group| group.name.clone())
                            })
                            .unwrap_or_else(|| tr!("cloud.group_default"));
                        // A CLI reads its config at process start, so running
                        // sessions keep their old route; say so instead of
                        // letting it read as "nothing happened". An automatic
                        // switch says it in its own words instead.
                        if origin == SwitchOrigin::Manual {
                            this.show_toast(tr!("cloud.group_switched", group = name));
                        }
                    }
                    // The switch usually happens from the footer menu, where
                    // the settings page's inline error area is invisible.
                    Err(error) => {
                        let message = format!("{error:#}");
                        this.cloud_account.error = Some(message.clone());
                        this.show_toast(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Redeem the code currently in the input field.
    pub(super) fn redeem_cloud_code(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let code = self.cloud_redeem_input.read(cx).content().trim().to_owned();
        if code.is_empty() || self.cloud_account.busy {
            return;
        }
        self.cloud_account.busy = true;
        self.cloud_account.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let client = sub2api::authenticated(&mut credentials)?;
                    let redeemed = client.redeem_code(&credentials.access_token, &code)?;
                    anyhow::Ok((credentials, redeemed))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.busy = false;
                match result {
                    Ok((renewed, redeemed)) => {
                        this.adopt_cloud_tokens(renewed);
                        this.cloud_redeem_input
                            .update(cx, |input, cx| input.clear(cx));
                        if let Some(balance) = redeemed.new_balance {
                            if let Some(user) = this.cloud_account.user.as_mut() {
                                user.balance = balance;
                            }
                            // The top-up sheet hosts the redeem field now;
                            // its account card must show the credited figure.
                            if let Some(config) = this
                                .cloud_pay
                                .as_mut()
                                .and_then(|state| state.config.as_mut())
                            {
                                config.user_balance = Some(balance);
                            }
                        }
                        let message = if redeemed.message.is_empty() {
                            tr!("cloud.redeemed", value = format!("{:.2}", redeemed.value))
                        } else {
                            redeemed.message
                        };
                        this.show_toast(message);
                    }
                    // The field lives in the top-up sheet, where the account
                    // page's inline error area is invisible — toast instead.
                    Err(error) => this.show_toast(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Watch for the balance changing after payment moved to the browser.
    ///
    /// Payment completing in the browser has no way to call back into a
    /// desktop app, so there is nothing to await. Poll for a couple of
    /// minutes instead: the hosted page opens the instant the URL is handed
    /// over, so refreshing right here would only ever re-read the same
    /// pre-payment figure, and the five-minute tick is long enough that the
    /// user would assume the top-up failed.
    pub(super) fn poll_cloud_balance_until_changed(&mut self, cx: &mut Context<Self>) {
        let before = self.cloud_account.user.as_ref().map(|user| user.balance);
        cx.spawn(async move |this, cx| {
            for _ in 0..TOP_UP_POLL_ATTEMPTS {
                cx.background_executor().timer(TOP_UP_POLL_INTERVAL).await;
                let settled = this.update(cx, |this, cx| {
                    // Compare before requesting: this reads the result of the
                    // previous iteration's refresh.
                    let current = this.cloud_account.user.as_ref().map(|user| user.balance);
                    if current != before {
                        return true;
                    }
                    this.refresh_cloud_account(cx);
                    false
                });
                match settled {
                    Ok(true) | Err(_) => break,
                    Ok(false) => {}
                }
            }
        })
        .detach();
    }

    /// Turn gateway routing on or off without signing out. The choice
    /// persists in the credential file so a restart keeps it.
    pub(super) fn set_cloud_routing_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.cloud_account.routing_enabled = enabled;
        if let Some(credentials) = self.cloud_account.credentials.as_mut() {
            credentials.routing_disabled = !enabled;
            if let Err(error) = credentials.save() {
                self.show_toast(format!("{error:#}"));
            }
        }
        self.apply_cloud_routing();
        cx.notify();
    }

    /// Drive every CLI's global configuration to the current routing state.
    ///
    /// This *is* the routing mechanism now: the gateway (or a custom
    /// endpoint) is written into `~/.claude/settings.json`, `~/.codex/*`,
    /// `~/.grok/config.toml`, `opencode.json`, and Pi's `models.json` —
    /// with the pre-takeover originals backed up, and restored the moment a
    /// CLI stops being routed. Files change immediately; running sessions
    /// keep whatever they already read.
    pub(super) fn apply_cloud_routing(&mut self) {
        let custom = sub2api::custom_api::load();
        let origin = self.cloud_account.gateway_origin.origin();
        let cloud = self.cloud_account.credentials.as_ref().map(|credentials| {
            sub2api::gateway_config_with_origin(
                credentials,
                self.cloud_account.routing_enabled,
                origin.as_deref(),
            )
        });
        let desired = sub2api::global_config::desired_routes(cloud.as_ref(), &custom);
        match sub2api::global_config::reconcile(&desired) {
            Ok(warnings) => {
                for warning in warnings {
                    self.show_toast(warning);
                }
            }
            Err(error) => self.show_toast(format!("{error:#}")),
        }
    }

    pub(super) fn render_cloud_account_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let signed_in = self.cloud_account.credentials.is_some();
        let pending = self.cloud_account.pending;
        let routing_enabled = self.cloud_account.routing_enabled;
        let balance = self.cloud_account.user.as_ref().map(|user| user.balance);
        let identity = match self.cloud_account.user.as_ref() {
            Some(user) if !user.email.is_empty() => user.email.clone(),
            Some(user) if !user.username.is_empty() => user.username.clone(),
            Some(_) => tr!("cloud.signed_in"),
            None if signed_in => tr!("cloud.signed_in"),
            None => tr!("cloud.not_signed_in"),
        };
        let error = self.cloud_account.error.clone();

        let mut page = div().mt(px(15.0)).w_full().flex().flex_col().gap(px(12.0));

        page = page.child(cloud_card(
            theme,
            "cloud-account-identity",
            &tr!("cloud.account"),
            identity,
            Some(if signed_in {
                (tr!("cloud.sign_out"), CloudAction::SignOut)
            } else {
                (tr!("cloud.sign_in"), CloudAction::SignIn)
            }),
            pending,
            cx,
        ));

        if let Some(balance) = balance {
            page = page.child(
                cloud_card(
                    theme,
                    "cloud-account-balance",
                    &tr!("cloud.balance"),
                    // The figure is US dollars; bare it reads as an abstract
                    // count. Same presentation as the old client's formatUsd.
                    format!("${balance:.2}"),
                    Some((tr!("cloud.top_up"), CloudAction::TopUp)),
                    pending,
                    cx,
                )
                // Same grading as the old client's header badge: an empty or
                // nearly empty balance is the reason the next request will
                // fail, so it should not read as ordinary body text.
                .text_color(balance_color(balance, theme)),
            );
        }

        if signed_in {
            page = page.child(cloud_card(
                theme,
                "cloud-account-routing",
                &tr!("cloud.routing_title"),
                if routing_enabled {
                    tr!("cloud.routing_on")
                } else {
                    tr!("cloud.routing_off")
                },
                Some((
                    if routing_enabled {
                        tr!("cloud.turn_off")
                    } else {
                        tr!("cloud.turn_on")
                    },
                    CloudAction::SetRouting(!routing_enabled),
                )),
                pending,
                cx,
            ));
        }

        if signed_in {
            // Redeem codes live in the top-up sheet and pricing on the Model
            // Plaza page; this page keeps identity, routing, and groups.
            page = page
                .child(self.render_gateway_origins(theme, cx))
                .child(self.render_cloud_groups(theme, cx))
                .child(self.render_cloud_referral(theme, cx));
        }

        if let Some(error) = error {
            page = page.child(
                div()
                    .w_full()
                    .px(px(20.0))
                    .py(px(14.0))
                    .rounded(px(13.0))
                    .bg(theme.raised)
                    .child(
                        div()
                            .text_size(sp(12.5))
                            .line_height(sp(18.0))
                            .text_color(theme.text_secondary)
                            .child(error),
                    ),
            );
        }

        page.into_any_element()
    }

    /// The service domains this build knows, their measured latency, and
    /// which one the CLIs are pointed at.
    fn render_gateway_origins(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let candidates = self.cloud_account.gateway_origin.effective_candidates();
        let extra = self.cloud_account.origin_input.is_some();
        // One domain and nothing added: there is no choice to present.
        if candidates.len() < 2 && !extra {
            return div().child(card_button(
                theme,
                "cloud-origin-open".into(),
                tr!("cloud.origin_title"),
                false,
                false,
                cx,
                |this, window, cx| this.open_gateway_origin_input(window, cx),
            ));
        }

        let routed = self.cloud_account.gateway_origin.origin();
        let auto_select = self.cloud_account.gateway_origin.auto_select;
        let test = self.cloud_account.origin_test.as_ref();
        let testing = test.is_some_and(|test| test.running);
        let results = test.map(|test| test.results.clone()).unwrap_or_default();
        let outcome = |url: &str| results.iter().find(|candidate| candidate.url == url).cloned();

        let mut ordered = candidates.clone();
        if !results.is_empty() {
            ordered.sort_by_key(|url| match outcome(url) {
                Some(candidate) => match candidate.latency_ms() {
                    Some(ms) => (0u8, ms),
                    None => (1, 0),
                },
                None => (2, 0),
            });
        }

        let mut rows = div().flex().flex_col().gap(px(6.0)).child(section_title(
            theme,
            &tr!("cloud.origin_title"),
            &tr!("cloud.origin_detail"),
        ));

        for (index, url) in ordered.iter().enumerate() {
            let in_use = routed.as_deref() == Some(url.as_str());
            let candidate = outcome(url);
            let (status_text, status_color) = match candidate.as_ref() {
                Some(result) if result.invalid.is_some() => {
                    (tr!("cli_setup.speed_invalid_url"), theme.danger)
                }
                Some(result) => match result.result.as_ref() {
                    Some(probe) => match probe.verdict {
                        sub2api::custom_api::ProbeVerdict::Ok => (
                            tr!("cli_setup.candidate_latency", ms = probe.latency_ms),
                            theme.success,
                        ),
                        sub2api::custom_api::ProbeVerdict::Unauthorized => {
                            (tr!("cli_setup.candidate_unauthorized"), theme.warning)
                        }
                        sub2api::custom_api::ProbeVerdict::HttpError => (
                            tr!(
                                "cli_setup.candidate_http",
                                status = probe.status.unwrap_or_default()
                            ),
                            theme.warning,
                        ),
                        sub2api::custom_api::ProbeVerdict::Unreachable => {
                            (tr!("cli_setup.candidate_unreachable"), theme.danger)
                        }
                    },
                    None => (String::new(), theme.text_ghost),
                },
                None if testing => (tr!("cli_setup.custom_testing"), theme.text_ghost),
                None => (String::new(), theme.text_ghost),
            };

            let mut row = div()
                .w_full()
                .px(px(16.0))
                .py(px(10.0))
                .rounded(px(11.0))
                .bg(theme.raised)
                .border_1()
                .border_color(if in_use { theme.accent } else { theme.raised })
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(crate::md::render::MONO_FAMILY)
                        .text_size(sp(12.0))
                        .text_color(theme.text)
                        .child(url.clone()),
                );
            if !status_text.is_empty() {
                row = row.child(
                    div()
                        .flex_none()
                        .text_size(sp(11.5))
                        .text_color(status_color)
                        .child(status_text),
                );
            }
            if in_use {
                row = row.child(
                    div()
                        .flex_none()
                        .text_size(sp(12.0))
                        .text_color(theme.accent)
                        .child(tr!("cli_setup.candidate_active")),
                );
            } else {
                let pick = url.clone();
                row = row.child(card_button(
                    theme,
                    SharedString::from(format!("cloud-origin-use-{index}")),
                    tr!("cli_setup.candidate_use"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.select_gateway_origin(pick.clone(), cx),
                ));
            }
            // The domains the build ships with are not the user's to remove.
            if self.cloud_account.gateway_origin.is_removable(url) {
                let drop = url.clone();
                row = row.child(card_button(
                    theme,
                    SharedString::from(format!("cloud-origin-remove-{index}")),
                    tr!("cli_setup.candidate_remove"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.remove_gateway_origin(drop.clone(), cx),
                ));
            }
            rows = rows.child(row);
        }

        if let Some(input) = self.cloud_account.origin_input.clone() {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(TextField::new("cloud-origin-input", input).flex_1())
                    .child(card_button(
                        theme,
                        "cloud-origin-add".into(),
                        tr!("cli_setup.candidate_add"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.add_gateway_origin(cx),
                    )),
            );
        } else {
            rows = rows.child(div().flex().child(card_button(
                theme,
                "cloud-origin-add-open".into(),
                tr!("cli_setup.candidate_add"),
                false,
                false,
                cx,
                |this, window, cx| this.open_gateway_origin_input(window, cx),
            )));
        }

        rows.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .child(card_button(
                    theme,
                    "cloud-origin-test".into(),
                    if testing {
                        tr!("cli_setup.speed_testing")
                    } else {
                        tr!("cli_setup.speed_test_all")
                    },
                    false,
                    testing,
                    cx,
                    |this, _, cx| this.run_gateway_origin_test(false, cx),
                ))
                .child(toggle_switch(
                    "cloud-origin-auto",
                    auto_select,
                    false,
                    theme,
                    cx,
                    move |this, _, cx| this.set_gateway_origin_auto_select(!auto_select, cx),
                ))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("cli_setup.speed_auto_select")),
                ),
        )
    }

    /// Group picker, one section per CLI. Selecting a group rebinds that
    /// CLI's gateway key.
    fn render_cloud_groups(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        if self.cloud_account.groups.is_empty() {
            return div();
        }
        let busy = self.cloud_account.busy;
        let mut rows = div().flex().flex_col().gap(px(6.0)).child(section_title(
            theme,
            &tr!("cloud.group_title"),
            &tr!("cloud.group_detail"),
        ));

        for platform in cloud_platforms(&self.cloud_account.groups) {
            let bound = self
                .cloud_account
                .credentials
                .as_ref()
                .and_then(|credentials| {
                    sub2api::bound_group_for_platform(credentials, &platform)
                });
            let failover_on = self.cloud_account.failover.enabled(&platform);
            let toggle_platform = platform.clone();
            rows = rows.child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_secondary)
                            .child(platform_display_name(&platform)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(sp(11.5))
                            .text_color(theme.text_ghost)
                            .child(tr!("cloud.failover_title")),
                    )
                    .child(toggle_switch(
                        SharedString::from(format!("cloud-failover-{platform}")),
                        failover_on,
                        busy,
                        theme,
                        cx,
                        move |this, _, cx| {
                            this.set_cloud_failover(
                                toggle_platform.clone(),
                                !failover_on,
                                cx,
                            );
                        },
                    )),
            );
            if failover_on {
                rows = rows.child(
                    div()
                        .text_size(sp(11.5))
                        .line_height(sp(16.0))
                        .text_color(theme.text_ghost)
                        .child(tr!("cloud.failover_detail")),
                );
            }
            let failed_over_from = self
                .cloud_account
                .failed_over
                .get(&platform)
                .and_then(|from| {
                    self.cloud_account
                        .groups
                        .iter()
                        .find(|group| group.id == *from)
                        .map(|group| group.name.clone())
                });
            for group in self
                .cloud_account
                .groups
                .iter()
                .filter(|group| group.platform == platform)
                .cloned()
            {
                let id = Some(group.id);
                let name = group.name.clone();
                let detail = group_status_suffix(
                    &group,
                    self.cloud_account
                        .group_status
                        .iter()
                        .find(|status| status.group_id == group.id),
                );
                let active = bound == id;
                // On a fallback the "Active" tag would read as the user's own
                // choice; name the group routing was moved off instead.
                let active_label = match failed_over_from.clone() {
                    Some(from) if active => tr!("cloud.failover_active", group = from),
                    _ => tr!("cloud.group_active"),
                };
                let row_platform = platform.clone();
                rows = rows.child(
                    div()
                        .id(SharedString::from(format!(
                            "cloud-group-{platform}-{}",
                            id.unwrap_or(-1)
                        )))
                        .tab_index(0)
                        .w_full()
                        .px(px(16.0))
                        .py(px(11.0))
                        .rounded(px(11.0))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(if active { theme.accent } else { theme.raised })
                        .cursor_default()
                        .opacity(if busy { 0.55 } else { 1.0 })
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(sp(12.8))
                                        .text_color(theme.text)
                                        .child(name),
                                )
                                .when(!detail.is_empty(), |element| {
                                    element.child(
                                        div()
                                            .mt(px(2.0))
                                            .text_size(sp(12.0))
                                            .text_color(theme.text_ghost)
                                            .truncate()
                                            .child(detail),
                                    )
                                }),
                        )
                        .when(active, |element| {
                            element.child(
                                div()
                                    .flex_none()
                                    .text_size(sp(12.0))
                                    .text_color(theme.accent)
                                    .child(active_label),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if busy {
                                return;
                            }
                            this.select_cloud_group(row_platform.clone(), id, cx);
                        })),
                );
            }
        }
        rows
    }

    /// Referral code and share link.
    fn render_cloud_referral(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let Some(referral) = self.cloud_account.referral.clone() else {
            return div();
        };
        if referral.referral_code.is_empty() {
            return div();
        }
        let link = referral.referral_link.clone();
        cloud_card(
            theme,
            "cloud-referral",
            &tr!("cloud.invite"),
            tr!(
                "cloud.invite_detail",
                code = referral.referral_code,
                count = referral.stats.total_referrals
            ),
            (!link.is_empty()).then_some((tr!("cloud.copy_link"), CloudAction::CopyReferral)),
            false,
            cx,
        )
    }

}

impl Waku {
    /// Account chip for the sidebar footer: the always-visible way in.
    ///
    /// Signed out it invites and opens the account page. Signed in it opens a
    /// menu: balance, per-CLI group switching, the model catalog, and the
    /// hosted usage history.
    pub(super) fn render_cloud_footer_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let signed_in = self.cloud_account.credentials.is_some();
        // Identity only — the balance is a figure to check deliberately, not
        // to have on screen at all times, so it lives in the opened menu.
        let label = match self.cloud_account.user.as_ref() {
            Some(user) if !user.email.is_empty() => user.email.clone(),
            Some(user) if !user.username.is_empty() => user.username.clone(),
            Some(_) => tr!("cloud.signed_in"),
            None if signed_in => tr!("cloud.signed_in"),
            None => tr!("cloud.sidebar_sign_in"),
        };

        let pending = self.cloud_account.pending;
        let label = if !signed_in && pending {
            tr!("cloud.onboarding_waiting")
        } else {
            label
        };
        let trigger = div()
            .id("cloud-footer-chip")
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .h(px(26.0))
            .px(px(9.0))
            .max_w(px(190.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .cursor_default()
            .hover(|element| element.bg(theme.overlay))
            .text_size(sp(12.0))
            .text_color(if signed_in {
                theme.text_secondary
            } else {
                theme.accent
            })
            .child(div().truncate().child(label));

        if !signed_in {
            // Straight into the browser flow — routing the user through a
            // settings page to find the same button is a detour.
            return trigger
                .opacity(if pending { 0.6 } else { 1.0 })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !pending {
                        this.start_cloud_sign_in(cx);
                    }
                }))
                .into_any_element();
        }

        // Snapshots for the item builder, which runs on every open frame.
        // Opening the menu re-pulls groups and balance, so a group created in
        // the web console a moment ago is switchable without a restart.
        let refresh_weak = cx.entity().downgrade();
        let handle = self.menu_handle_with("cloud-account-menu", cx, move |open, _, cx| {
            if open {
                let _ = refresh_weak.update(cx, |this, cx| {
                    this.load_cloud_details(cx);
                    this.refresh_cloud_account(cx);
                });
            }
        });
        let weak = cx.entity().downgrade();
        let balance = self.cloud_account.user.as_ref().map(|user| user.balance);
        let groups = self.cloud_account.groups.clone();
        let statuses = self.cloud_account.group_status.clone();
        let bindings: Vec<(String, Option<i64>)> = cloud_platforms(&groups)
            .into_iter()
            .map(|platform| {
                let bound = self
                    .cloud_account
                    .credentials
                    .as_ref()
                    .and_then(|credentials| {
                        sub2api::bound_group_for_platform(credentials, &platform)
                    });
                (platform, bound)
            })
            .collect();

        dropdown_menu(
            trigger,
            "cloud-account-menu",
            &handle,
            MenuAlign::AboveLeft,
            move |_| {
                let mut items = Vec::new();
                if let Some(balance) = balance {
                    items.push(MenuItem::Header(SharedString::from(format!(
                        "{}  ${balance:.2}",
                        tr!("cloud.balance")
                    ))));
                    let top_up_weak = weak.clone();
                    items.push(
                        MenuItem::new(tr!("cloud.top_up"), move |_, cx| {
                            let _ = top_up_weak.update(cx, |this, cx| {
                                this.open_cloud_pay_modal(cx);
                            });
                        })
                        .icon("icons/wallet.svg"),
                    );
                    items.push(MenuItem::Separator);
                }

                // One submenu per CLI: pick the group its traffic routes
                // through, or drop back to the account default.
                for (platform, bound) in &bindings {
                    // (id, plain name, menu label with rate + 24h availability).
                    let platform_groups: Vec<(i64, String, String)> = groups
                        .iter()
                        .filter(|group| &group.platform == platform)
                        .map(|group| {
                            let status = statuses
                                .iter()
                                .find(|status| status.group_id == group.id);
                            let suffix = group_status_suffix(group, status);
                            let label = if suffix.is_empty() {
                                group.name.clone()
                            } else {
                                format!("{}   {suffix}", group.name)
                            };
                            (group.id, group.name.clone(), label)
                        })
                        .collect();
                    if platform_groups.is_empty() {
                        continue;
                    }
                    let current = bound
                        .and_then(|id| {
                            platform_groups
                                .iter()
                                .find(|(group_id, ..)| *group_id == id)
                                .map(|(_, name, _)| name.clone())
                        })
                        .unwrap_or_else(|| tr!("cloud.group_default"));
                    let bound = *bound;
                    let submenu_platform = platform.clone();
                    let submenu_weak = weak.clone();
                    items.push(MenuItem::submenu_with_value(
                        tr!("cloud.group_menu", cli = platform_display_name(platform)),
                        current,
                        move |_| {
                            let mut entries = Vec::new();
                            for (group_id, _, label) in &platform_groups {
                                let entry_platform = submenu_platform.clone();
                                let entry_weak = submenu_weak.clone();
                                let group_id = *group_id;
                                entries.push(
                                    MenuItem::new(label.clone(), move |_, cx| {
                                        let platform = entry_platform.clone();
                                        let _ = entry_weak.update(cx, |this, cx| {
                                            this.select_cloud_group(
                                                platform,
                                                Some(group_id),
                                                cx,
                                            );
                                        });
                                    })
                                    .selected(bound == Some(group_id)),
                                );
                            }
                            entries
                        },
                    ));
                }
                if !bindings.is_empty() {
                    items.push(MenuItem::Separator);
                }

                let catalog_weak = weak.clone();
                items.push(MenuItem::new(tr!("cloud.menu_catalog"), move |_, cx| {
                    let _ = catalog_weak.update(cx, |this, cx| {
                        this.open_settings_page(SettingsPage::ModelPlaza, cx);
                    });
                }));
                let usage_weak = weak.clone();
                items.push(MenuItem::new(tr!("cloud.menu_usage"), move |_, cx| {
                    let _ = usage_weak.update(cx, |this, cx| {
                        this.open_settings_page(SettingsPage::CloudUsage, cx);
                    });
                }));

                items.push(MenuItem::Separator);
                let sign_out_weak = weak.clone();
                items.push(MenuItem::new(tr!("cloud.sign_out"), move |_, cx| {
                    let _ = sign_out_weak.update(cx, |this, cx| {
                        this.sign_out_cloud(cx);
                        // Make the consequence explicit: from here the agents
                        // run on whatever the user's own CLIs are configured
                        // with, exactly as if this app were stock.
                        this.show_toast(tr!("cloud.signed_out_note"));
                    });
                }));
                items
            },
        )
    }

    /// Balance badge for the status strip above the composer — the old
    /// client's `HeaderBalanceBadge`: a wallet glyph and the graded figure,
    /// and clicking it opens the top-up sheet directly.
    ///
    /// The old client kept this in the window header; the native app's
    /// equivalent always-visible strip is the one that already carries the
    /// project, branch, and plan-usage controls.
    ///
    /// Shown only while routing is on: with routing off the agent spends the
    /// user's own vendor credit, and a cloud balance would be describing money
    /// that has nothing to do with the request about to be sent.
    pub(super) fn render_cloud_balance_badge(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.cloud_account.routing_enabled {
            return None;
        }
        let balance = self.cloud_account.user.as_ref()?.balance;
        let theme = Theme::current(cx);
        let color = balance_color(balance, theme);
        Some(
            div()
                .id("cloud-balance-badge")
                .tab_index(0)
                .h(px(22.0))
                .px(px(7.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .cursor_default()
                .text_size(sp(12.5))
                .text_color(color)
                .hover(|style| style.bg(theme.raised))
                .child(icon("icons/wallet.svg", 13.0, color))
                .child(format!("${balance:.2}"))
                // The modal is safe under a stray click — it only shows the
                // form; nothing is charged without further explicit steps.
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_cloud_pay_modal(cx);
                }))
                .into_any_element(),
        )
    }
}

/// The platforms present in `groups`, in a stable, CLI-meaningful order.
fn cloud_platforms(groups: &[sub2api::client::Group]) -> Vec<String> {
    let mut platforms: Vec<String> = Vec::new();
    // The two first-class CLIs come first when present.
    for known in ["anthropic", "openai"] {
        if groups.iter().any(|group| group.platform == known) {
            platforms.push(known.to_owned());
        }
    }
    for group in groups {
        if !platforms.contains(&group.platform) && !group.platform.is_empty() {
            platforms.push(group.platform.clone());
        }
    }
    platforms
}

/// The CLI a platform's groups route, named as the user knows it. Unknown
/// platforms are shown capitalized rather than as their raw lowercase id.
/// `×0.50 · 99.2%` — a group's rate multiplier, 24h availability, and a
/// translated status word when it is not healthy. Empty when nothing is
/// known. Availability formatting matches the Model Plaza's.
fn group_status_suffix(
    group: &sub2api::client::Group,
    status: Option<&sub2api::client::GroupStatusItem>,
) -> String {
    let mut parts = Vec::new();
    if group.rate_multiplier > 0.0 {
        parts.push(format!("\u{00d7}{:.2}", group.rate_multiplier));
    }
    if let Some(status) = status {
        if let Some(value) = status.availability_24h.filter(|value| value.is_finite()) {
            parts.push(if value >= 99.0 {
                format!("{value:.2}%")
            } else {
                format!("{value:.1}%")
            });
        }
        // Never color alone in a menu row — say it.
        match status.effective_status() {
            "degraded" => parts.push(tr!("plaza.status_degraded")),
            "down" => parts.push(tr!("plaza.status_down")),
            _ => {}
        }
    }
    parts.join(" \u{00b7} ")
}

fn platform_display_name(platform: &str) -> String {
    match platform {
        "anthropic" => "Claude Code".to_owned(),
        "openai" => "Codex".to_owned(),
        "grok" => "Grok".to_owned(),
        "gemini" => "Gemini".to_owned(),
        other => {
            let mut characters = other.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => other.to_owned(),
            }
        }
    }
}

/// Heading above a group of rows.
fn section_title(theme: Theme, title: &str, detail: &str) -> Div {
    div()
        .mt(px(6.0))
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(sp(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .child(
            div()
                .mt(px(3.0))
                .text_size(sp(12.0))
                .text_color(theme.text_ghost)
                .child(detail.to_owned()),
        )
}

/// Colour for a balance figure: at or below $1 the next sizeable request
/// fails (red); below $5 it is time to top up (amber).
fn balance_color(balance: f64, theme: Theme) -> Hsla {
    if balance <= 1.0 {
        theme.danger
    } else if balance < 5.0 {
        theme.warning
    } else {
        theme.text_secondary
    }
}

/// One settings row: title, detail, and an optional action button.
fn cloud_card(
    theme: Theme,
    id: &'static str,
    title: &str,
    detail: String,
    action: Option<(String, CloudAction)>,
    pending: bool,
    cx: &mut Context<Waku>,
) -> Div {
    let row = div()
        .w_full()
        .px(px(20.0))
        .py(px(16.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .mt(px(5.0))
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(theme.text_secondary)
                        .child(detail),
                ),
        );

    let Some((label, action)) = action else {
        return row;
    };
    row.child(
        div()
            .id(id)
            .tab_index(0)
            .h(px(29.0))
            .px(px(11.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .opacity(if pending { 0.55 } else { 1.0 })
            .child(label.to_owned())
            .on_click(cx.listener(move |this, _, _, cx| {
                if pending {
                    return;
                }
                match action {
                    CloudAction::SignIn => this.start_cloud_sign_in(cx),
                    CloudAction::SignOut => this.sign_out_cloud(cx),
                    CloudAction::SetRouting(enabled) => {
                        this.set_cloud_routing_enabled(enabled, cx)
                    }
                    CloudAction::TopUp => this.open_cloud_pay_modal(cx),
                    CloudAction::CopyReferral => {
                        if let Some(referral) = this.cloud_account.referral.clone() {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                referral.referral_link,
                            ));
                            this.show_toast(tr!("cloud.invite_copied"));
                        }
                    }
                }
            })),
    )
}
