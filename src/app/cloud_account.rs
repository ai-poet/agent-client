//! Settings → Cloud Account.
//!
//! Fork addition. All the logic lives in the `sub2api` crate, which has no
//! GPUI dependency and is unit-tested on its own; this file is the view and the
//! plumbing that runs that logic off the UI thread.
//!
//! Publishing routing configuration goes through the persisted state rather
//! than straight to the daemon: `PersistedState` owns the `extra` map that
//! carries unknown daemon settings, and every later `save()` re-emits it. A
//! direct `update_settings` call would be silently undone by the next save.

use std::time::Duration;

use super::*;

/// How long the loopback listener waits for the browser before giving up.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

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
    /// Gateway pricing per model.
    pub catalog: Vec<sub2api::client::ModelCatalogItem>,
    /// Referral code and share link.
    pub referral: Option<sub2api::client::ReferralInfo>,
    /// A group switch or redemption is in flight.
    pub busy: bool,
    /// The welcome-screen sign-in card was dismissed this session.
    ///
    /// Deliberately not persisted: signing in is the product's whole value,
    /// so the nudge may return on the next launch — but within a session,
    /// "later" means later.
    pub onboarding_dismissed: bool,
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
        let Some(credentials) = sub2api::Credentials::load() else {
            return;
        };
        self.cloud_account.routing_enabled =
            sub2api::gateway::load().is_some_and(|config| config.enabled);
        self.cloud_account.credentials = Some(credentials);
        self.refresh_cloud_account(cx);
        self.load_cloud_details(cx);
    }

    /// Fold a background task's renewed session into the in-memory one.
    ///
    /// Only the token fields are taken. The background clone was made before
    /// the task ran, so adopting it wholesale would silently roll back
    /// anything the user did meanwhile — picking a group rebinds `api_key`,
    /// and a balance poll finishing a moment later must not undo that.
    fn adopt_cloud_tokens(&mut self, renewed: sub2api::Credentials) {
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
                        this.publish_cloud_gateway();
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
        self.publish_cloud_gateway();
        cx.notify();
    }

    /// Fetch the things that change rarely: groups, pricing, referral.
    ///
    /// Kept out of [`Self::refresh_cloud_account`], which runs every five
    /// minutes for the balance — the catalog is large and static enough that
    /// re-pulling it on that cadence would be waste.
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
                        client.model_catalog(token).unwrap_or_default().items,
                        client.referral_info(token).ok(),
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Ok((credentials, groups, catalog, referral)) = loaded else {
                    // A failed renewal here is not worth a banner: the balance
                    // poll reports the same problem, and this data is optional.
                    return;
                };
                this.adopt_cloud_tokens(credentials);
                this.cloud_account.groups = groups;
                this.cloud_account.catalog = catalog;
                this.cloud_account.referral = referral;
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
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.cloud_account.busy {
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
                    sub2api::bind_group_for_platform(&mut credentials, &platform, group_id)?;
                    anyhow::Ok(credentials)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.busy = false;
                match result {
                    Ok(renewed) => {
                        this.cloud_account.credentials = Some(renewed);
                        this.publish_cloud_gateway();
                    }
                    Err(error) => this.cloud_account.error = Some(format!("{error:#}")),
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
                        if let Some(balance) = redeemed.new_balance
                            && let Some(user) = this.cloud_account.user.as_mut()
                        {
                            user.balance = balance;
                        }
                        let message = if redeemed.message.is_empty() {
                            tr!("cloud.redeemed", value = format!("{:.2}", redeemed.value))
                        } else {
                            redeemed.message
                        };
                        this.show_toast(message);
                    }
                    Err(error) => this.cloud_account.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the hosted top-up page in the user's browser.
    pub(super) fn open_cloud_top_up(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        // The pay page takes a bare language tag, so `zh-CN` becomes `zh`;
        // it falls back to its own default for anything it does not know.
        let locale = self.state.language.locale();
        let language = locale.split('-').next().unwrap_or(locale);
        let url = sub2api::Client::new(credentials.endpoint.clone())
            .top_up_url(&credentials.access_token, language);
        cx.open_url(&url);

        // Payment completes in the browser and the service has no way to call
        // back into a desktop app, so there is nothing to await. Poll for a
        // couple of minutes instead: `open_url` returns the instant the browser
        // launches, so refreshing right here would only ever re-read the same
        // pre-payment figure, and the five-minute tick is long enough that the
        // user would assume the top-up failed.
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

    /// Turn gateway routing on or off without signing out.
    pub(super) fn set_cloud_routing_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.cloud_account.routing_enabled = enabled;
        self.publish_cloud_gateway();
        cx.notify();
    }

    /// Publish routing configuration into daemon settings.
    ///
    /// Existing sessions keep the environment they launched with; the change
    /// applies to the next agent that starts.
    fn publish_cloud_gateway(&mut self) {
        let mut settings = self.state.daemon_settings();
        match self.cloud_account.credentials.as_ref() {
            Some(credentials) => {
                let config =
                    sub2api::gateway_config_from(credentials, self.cloud_account.routing_enabled);
                if let Err(error) = config.write_into(&mut settings.extra) {
                    self.show_toast(format!("{error:#}"));
                    return;
                }
            }
            None => sub2api::GatewayConfig::remove_from(&mut settings.extra),
        }
        self.state.apply_daemon_settings(settings);
        self.save();
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
            page = page
                .child(self.render_cloud_groups(theme, cx))
                .child(self.render_cloud_redeem(theme, cx))
                .child(self.render_cloud_referral(theme, cx))
                .child(self.render_cloud_catalog(theme));
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
            rows = rows.child(
                div()
                    .mt(px(8.0))
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_secondary)
                    .child(platform_display_name(&platform)),
            );
            for group in std::iter::once(None).chain(
                self.cloud_account
                    .groups
                    .iter()
                    .filter(|group| group.platform == platform)
                    .map(|group| Some(group.clone())),
            ) {
                let (id, name, detail) = match &group {
                    Some(group) => (
                        Some(group.id),
                        group.name.clone(),
                        if group.rate_multiplier > 0.0 {
                            format!("\u{00d7}{:.2}", group.rate_multiplier)
                        } else {
                            String::new()
                        },
                    ),
                    None => (
                        None,
                        tr!("cloud.group_default"),
                        tr!("cloud.group_default_detail"),
                    ),
                };
                let active = bound == id;
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
                                    .text_size(sp(12.0))
                                    .text_color(theme.accent)
                                    .child(tr!("cloud.group_active")),
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

    /// Redeem-code field.
    fn render_cloud_redeem(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let busy = self.cloud_account.busy;
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(section_title(
                theme,
                &tr!("cloud.redeem_title"),
                &tr!("cloud.redeem_detail"),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        TextField::new("cloud-redeem-field", self.cloud_redeem_input.clone())
                            .flex_1()
                            .max_w(px(430.0)),
                    )
                    .child(
                        div()
                            .id("cloud-redeem-submit")
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
                            .opacity(if busy { 0.55 } else { 1.0 })
                            .child(tr!("cloud.redeem_action"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.redeem_cloud_code(cx);
                            })),
                    ),
            )
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

    /// Gateway pricing per model.
    fn render_cloud_catalog(&self, theme: Theme) -> Div {
        if self.cloud_account.catalog.is_empty() {
            return div();
        }
        let mut list = div().flex().flex_col().gap(px(6.0)).child(section_title(
            theme,
            &tr!("cloud.pricing_title"),
            &tr!("cloud.pricing_detail"),
        ));
        for item in &self.cloud_account.catalog {
            let name = if item.display_name.is_empty() {
                item.model.clone()
            } else {
                item.display_name.clone()
            };
            let price = match (
                item.effective_pricing_usd.input_per_mtok_usd,
                item.effective_pricing_usd.output_per_mtok_usd,
            ) {
                (Some(input), Some(output)) => tr!(
                    "cloud.price_in_out",
                    input = format!("{input:.2}"),
                    output = format!("{output:.2}")
                ),
                (Some(input), None) => tr!("cloud.price_in", input = format!("{input:.2}")),
                (None, Some(output)) => tr!("cloud.price_out", output = format!("{output:.2}")),
                // Per-request models have no per-token price; showing 0.00
                // would read as free.
                (None, None) => match item.effective_pricing_usd.per_request_usd {
                    Some(per_request) => {
                        tr!("cloud.price_per_request", price = format!("{per_request:.4}"))
                    }
                    None => tr!("cloud.price_unknown"),
                },
            };
            list = list.child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .py(px(10.0))
                    .rounded(px(11.0))
                    .bg(theme.raised)
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
                            .child(div().text_size(sp(12.8)).text_color(theme.text).child(name))
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_ghost)
                                    .truncate()
                                    .child(if item.best_group.name.is_empty() {
                                        item.platform.clone()
                                    } else {
                                        format!("{}  ·  {}", item.platform, item.best_group.name)
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_secondary)
                                    .child(price),
                            )
                            .when_some(
                                item.comparison
                                    .savings_percent
                                    .filter(|_| item.comparison.is_cheaper_than_official),
                                |element, savings| {
                                    element.child(
                                        div()
                                            .text_size(sp(12.0))
                                            .text_color(theme.success)
                                            .child(format!("-{savings:.0}%")),
                                    )
                                },
                            ),
                    ),
            );
        }
        list
    }
}

impl Waku {
    /// Sign-in call-to-action for the welcome screen.
    ///
    /// `None` once signed in or dismissed. The primary button starts the
    /// browser flow directly — sending a first-run user through Settings to
    /// find the same button is a needless detour.
    pub(super) fn render_cloud_onboarding_card(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.cloud_account.credentials.is_some() || self.cloud_account.onboarding_dismissed {
            return None;
        }
        let theme = Theme::current(cx);
        let pending = self.cloud_account.pending;

        Some(
            div()
                .mt(px(28.0))
                .max_w(px(420.0))
                .w_full()
                .px(px(20.0))
                .py(px(18.0))
                .rounded(px(14.0))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .text_size(sp(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(tr!(
                            "cloud.onboarding_title",
                            name = sub2api::brand::DISPLAY_NAME
                        )),
                )
                .child(
                    div()
                        .mt(px(6.0))
                        .text_center()
                        .text_size(sp(12.5))
                        .line_height(sp(19.0))
                        .text_color(theme.text_tertiary)
                        .child(tr!("cloud.onboarding_detail")),
                )
                .child(
                    div()
                        .mt(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .id("cloud-onboarding-sign-in")
                                .tab_index(0)
                                .focus_visible(|style| {
                                    style.border_1().border_color(theme.accent)
                                })
                                .h(px(32.0))
                                .px(px(16.0))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_default()
                                .bg(theme.inverse)
                                .text_color(theme.on_inverse)
                                .text_size(sp(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .opacity(if pending { 0.55 } else { 1.0 })
                                .child(if pending {
                                    tr!("cloud.onboarding_waiting")
                                } else {
                                    tr!("cloud.sign_in")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_cloud_sign_in(cx);
                                })),
                        )
                        .child(
                            div()
                                .id("cloud-onboarding-later")
                                .tab_index(0)
                                .h(px(32.0))
                                .px(px(12.0))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_default()
                                .text_size(sp(12.5))
                                .text_color(theme.text_tertiary)
                                .hover(|style| style.bg(theme.overlay))
                                .child(tr!("cloud.onboarding_later"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cloud_account.onboarding_dismissed = true;
                                    cx.notify();
                                })),
                        ),
                )
                .when_some(self.cloud_account.error.clone(), |card, error| {
                    card.child(
                        div()
                            .mt(px(10.0))
                            .text_center()
                            .text_size(sp(12.0))
                            .text_color(theme.text_tertiary)
                            .child(error),
                    )
                }),
        )
    }

    /// Account chip for the sidebar footer: the always-visible way in.
    ///
    /// Signed out it invites and opens the account page. Signed in it opens a
    /// menu: balance, per-CLI group switching, the model catalog, and the
    /// hosted usage history.
    pub(super) fn render_cloud_footer_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let signed_in = self.cloud_account.credentials.is_some();
        let label = match self.cloud_account.user.as_ref() {
            Some(user) => {
                let identity = if user.email.is_empty() {
                    user.username.clone()
                } else {
                    user.email.clone()
                };
                format!("{identity}  ·  ${:.2}", user.balance)
            }
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
        let handle = self.menu_handle("cloud-account-menu", cx);
        let weak = cx.entity().downgrade();
        let balance = self.cloud_account.user.as_ref().map(|user| user.balance);
        let groups = self.cloud_account.groups.clone();
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
        let endpoint = self
            .cloud_account
            .credentials
            .as_ref()
            .map(|credentials| credentials.endpoint.clone())
            .unwrap_or_default();

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
                    items.push(MenuItem::Separator);
                }

                // One submenu per CLI: pick the group its traffic routes
                // through, or drop back to the account default.
                for (platform, bound) in &bindings {
                    let platform_groups: Vec<sub2api::client::Group> = groups
                        .iter()
                        .filter(|group| &group.platform == platform)
                        .cloned()
                        .collect();
                    if platform_groups.is_empty() {
                        continue;
                    }
                    let current = bound
                        .and_then(|id| {
                            platform_groups
                                .iter()
                                .find(|group| group.id == id)
                                .map(|group| group.name.clone())
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
                            let default_platform = submenu_platform.clone();
                            let default_weak = submenu_weak.clone();
                            entries.push(
                                MenuItem::new(tr!("cloud.group_default"), move |_, cx| {
                                    let platform = default_platform.clone();
                                    let _ = default_weak.update(cx, |this, cx| {
                                        this.select_cloud_group(platform, None, cx);
                                    });
                                })
                                .selected(bound.is_none()),
                            );
                            for group in &platform_groups {
                                let entry_platform = submenu_platform.clone();
                                let entry_weak = submenu_weak.clone();
                                let group_id = group.id;
                                entries.push(
                                    MenuItem::new(group.name.clone(), move |_, cx| {
                                        let platform = entry_platform.clone();
                                        let _ = entry_weak.update(cx, |this, cx| {
                                            this.select_cloud_group(
                                                platform,
                                                Some(group_id),
                                                cx,
                                            );
                                        });
                                    })
                                    .selected(bound == Some(group.id)),
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
                        this.open_settings_page(SettingsPage::CloudAccount, cx);
                    });
                }));
                // Usage history lives in the hosted console; recreating its
                // filters and tables in the desktop would be a slow copy of a
                // page one click away.
                let usage_endpoint = endpoint.clone();
                items.push(MenuItem::new(tr!("cloud.menu_usage"), move |_, cx| {
                    if !usage_endpoint.is_empty() {
                        cx.open_url(&format!("{usage_endpoint}/usage"));
                    }
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

    /// Balance chip for the status strip above the composer.
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
                .text_color(balance_color(balance, theme))
                .hover(|style| style.bg(theme.raised))
                .child(format!("${balance:.2}"))
                // Opens the account page rather than the browser: a stray
                // click on a always-visible chip should not launch a payment
                // page.
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_settings_page(SettingsPage::CloudAccount, cx);
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

/// The CLI a platform's groups route, named as the user knows it.
fn platform_display_name(platform: &str) -> String {
    match platform {
        "anthropic" => "Claude Code".to_owned(),
        "openai" => "Codex".to_owned(),
        other => other.to_owned(),
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

/// Colour for a balance figure, matching the old client's header badge:
/// exhausted is an error, nearly exhausted is a warning.
fn balance_color(balance: f64, theme: Theme) -> Hsla {
    if balance <= 0.0 {
        theme.danger
    } else if balance < 1.0 {
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
                    CloudAction::TopUp => this.open_cloud_top_up(cx),
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
