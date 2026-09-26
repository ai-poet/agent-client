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
//!
//! The rest of the account lives beside it: group lanes and switching in
//! `cloud_groups.rs`, group health and automatic failover in
//! `cloud_failover.rs`, the service domains in `cloud_origins.rs`, the
//! footer chip, account menu and balance badge in `cloud_menu.rs`, and
//! subscriptions and model routing in `cloud_subscriptions.rs`.

use std::time::{Duration, Instant};

use sub2api::model_routing::DOMESTIC_LANE;

use super::*;

/// How long the loopback listener waits for the browser before giving up.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// How long subscription usage read with the balance stays fresh. The balance
/// refresh runs after every settled turn; this keeps it from reading usage on
/// every one.
const SUBSCRIPTIONS_TTL: Duration = Duration::from_secs(60);

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
    pub origin_test: Option<super::cloud_origins::OriginTest>,
    /// Bumped per measurement; a result from a superseded run is discarded.
    pub origin_generation: u64,
    /// The once-per-run check of the chosen domain has been made.
    pub origin_checked: bool,
    /// Field for adding a domain, built on first use — creating a
    /// `TextInput` needs a `Window`, which render does not have.
    pub origin_input: Option<Entity<TextInput>>,
    /// The account's active subscriptions, once read. `None` until the first
    /// model-routing refresh answers, or when the deployment has none to
    /// report.
    pub subscriptions: Option<Vec<sub2api::client::SubscriptionProgress>>,
    /// Last subscription read on the balance cadence, for its TTL guard: a
    /// failed turn is judged by how far into its windows the account is.
    pub subscriptions_at: Option<Instant>,
    /// A model-routing refresh is in flight.
    pub routes_refreshing: bool,
    /// Something asked for another refresh while one was in flight.
    pub routes_stale: bool,
    /// The model a one-model route lookup is running for
    /// ([`sub2api::route_one_model`]). Only one at a time, and never beside a
    /// full refresh, so the two cannot both mint a key for the same group.
    pub route_lookup: Option<String>,
    /// Models a lookup found no route for this run; they are sent as they
    /// are rather than looked up again on every message. Cleared by a full
    /// refresh.
    pub route_misses: std::collections::HashSet<String>,
    /// Messages held back until their model has a key, by session.
    pub pending_route_sends: Vec<(Uuid, super::ComposerSubmission)>,
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
        // The built-in agent's model list is this account's catalog; fetch
        // it now rather than the first time the Model Plaza page is opened.
        self.refresh_native_catalog(false, cx);
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
        // Usage rides along behind its own TTL: what a failed turn's paywall
        // prompt reads when the driver lost the gateway's code.
        let read_subscriptions = self
            .cloud_account
            .subscriptions_at
            .is_none_or(|at| at.elapsed() >= SUBSCRIPTIONS_TTL);
        if read_subscriptions {
            self.cloud_account.subscriptions_at = Some(Instant::now());
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    sub2api::refresh_if_needed(&mut credentials)?;
                    let client = sub2api::Client::new(credentials.endpoint.clone());
                    let user = client.me(&credentials.access_token)?;
                    let subscriptions = read_subscriptions
                        .then(|| client.subscription_progress(&credentials.access_token).ok())
                        .flatten();
                    anyhow::Ok((credentials, user, subscriptions))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.pending = false;
                match fetched {
                    Ok((credentials, user, subscriptions)) => {
                        this.adopt_cloud_tokens(credentials);
                        this.cloud_account.user = Some(user);
                        this.cloud_account.error = None;
                        if let Some(subscriptions) = subscriptions {
                            let spent_before = this.exhausted_subscriptions();
                            this.cloud_account.subscriptions = Some(subscriptions);
                            // A subscription that just ran out (or reset)
                            // moves its models to pay-as-you-go (or back).
                            if this.exhausted_subscriptions() != spent_before {
                                this.refresh_model_routes(cx);
                                this.sync_native_models();
                            }
                        }
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
                        // A new account reaches a different set of models.
                        this.refresh_native_catalog(true, cx);
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
        self.cloud_account.subscriptions = None;
        self.cloud_account.subscriptions_at = None;
        self.cloud_account.routing_enabled = false;
        self.cloud_account.error = None;
        self.reset_plans();
        self.apply_cloud_routing();
        // The catalog was this account's; the built-in agent drops back to
        // its fallback list until someone signs in again.
        self.clear_model_plaza();
        self.sync_native_models();
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
                // Subscriptions come and go with the groups; so does which
                // group each of the built-in agent's models goes through.
                this.refresh_model_routes(cx);
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
                .child(self.render_cloud_subscriptions(theme, cx))
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

/// The CLI a platform's groups route, named as the user knows it. Unknown
/// platforms are shown capitalized rather than as their raw lowercase id.
pub(super) fn platform_display_name(platform: &str) -> String {
    match platform {
        "anthropic" => "Claude Code".to_owned(),
        "openai" => "Codex".to_owned(),
        "grok" => "Grok".to_owned(),
        "gemini" => "Gemini".to_owned(),
        DOMESTIC_LANE => tr!("cloud.lane_domestic"),
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
pub(super) fn section_title(theme: Theme, title: &str, detail: &str) -> Div {
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
pub(super) fn balance_color(balance: f64, theme: Theme) -> Hsla {
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
