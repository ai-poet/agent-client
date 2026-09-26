//! Settings → Plans: the subscription plans on sale, bought in-app.
//!
//! Fork addition. A plan is sold by the pay service (`/pay/api/subscription-
//! plans`) and grants, or extends, a subscription to one gateway group; the
//! purchase itself runs through the native pay sheet ([`super::cloud_pay`]).
//! This page lists the plans, marks the ones the account already holds with
//! their expiry and spend, and turns their button into "renew".
//!
//! A gateway without the pay service answers the plan list with a 502 or an
//! HTML page; the page then says plans are not offered here, and the entry
//! points elsewhere (account menu, top-up sheet, subscription cards) hide.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gpui::{Font, StyledText, TextRun};
use sub2api::client::SubscriptionProgress;
use sub2api::pay::{SubscriptionPlan, ValidityUnit};

use super::*;

/// How long a fetched plan list stays fresh before a re-render reloads it.
const PLANS_TTL: Duration = Duration::from_secs(120);

/// How many of the models a held plan routes its card names.
const LISTED_MODELS: usize = 6;

/// After a purchase in the hosted pay center, how often and how long to look
/// for the new subscription.
const PURCHASE_POLL_INTERVAL: Duration = Duration::from_secs(15);
const PURCHASE_POLL_ATTEMPTS: usize = 12;

/// Whether this gateway sells plans at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PlansAvailability {
    /// Not asked yet, or the last attempt failed for another reason.
    #[default]
    Unknown,
    Available,
    /// No pay service behind the gateway.
    Unavailable,
}

/// View state for the Plans page.
#[derive(Default)]
pub(super) struct PlansState {
    pub items: Vec<SubscriptionPlan>,
    pub availability: PlansAvailability,
    pub loading: bool,
    pub error: Option<String>,
    loaded_at: Option<Instant>,
    /// Render schedules loads; this keeps it from scheduling twice per frame.
    load_scheduled: Cell<bool>,
    /// Bumped per load and on sign-out; an answer for an older one is dropped.
    generation: u64,
}

/// What a plan's card offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CardState {
    Buy,
    /// The account holds this plan's group; `days` until it lapses, when known.
    Renew { days: Option<i64> },
}

/// A subscription window a plan caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LimitWindow {
    Daily,
    Weekly,
    Monthly,
}

impl Waku {
    /// Whether plan entry points should show: the list loaded and has
    /// something in it.
    pub(super) fn plans_offered(&self) -> bool {
        self.plans.availability == PlansAvailability::Available && !self.plans.items.is_empty()
    }

    /// Whether a menu entry to the page belongs: hidden once the gateway is
    /// known to sell none, shown while that is still unknown (the page then
    /// finds out).
    pub(super) fn plans_entry_visible(&self) -> bool {
        match self.plans.availability {
            PlansAvailability::Unavailable => false,
            PlansAvailability::Available => !self.plans.items.is_empty(),
            PlansAvailability::Unknown => true,
        }
    }

    /// Forget the plan list with the account it was loaded for.
    pub(super) fn reset_plans(&mut self) {
        self.plans.items.clear();
        self.plans.availability = PlansAvailability::Unknown;
        self.plans.error = None;
        self.plans.loaded_at = None;
        self.plans.loading = false;
        self.plans.generation = self.plans.generation.wrapping_add(1);
    }

    /// Fetch the plans, and with them the account's subscriptions so the
    /// "current plan" marks are fresh, when stale.
    pub(super) fn load_plans_if_needed(&mut self, force: bool, cx: &mut Context<Self>) {
        self.plans.load_scheduled.set(false);
        if self.plans.loading {
            return;
        }
        if !force && self.plans.loaded_at.is_some_and(|at| at.elapsed() < PLANS_TTL) {
            return;
        }
        let Some((client, credentials)) = self.pay_session() else {
            return;
        };
        self.plans.loading = true;
        self.plans.generation = self.plans.generation.wrapping_add(1);
        let generation = self.plans.generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let (credentials, fetched) = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let fetched = sub2api::pay::session_token(&mut credentials).and_then(|token| {
                        let plans = client.list_plans(&token)?;
                        // Decoration: an answer without it keeps the old list.
                        let subscriptions = sub2api::Client::new(credentials.endpoint.clone())
                            .subscription_progress(&token)
                            .ok();
                        anyhow::Ok((plans, subscriptions))
                    });
                    (credentials, fetched)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.plans.generation != generation {
                    return;
                }
                this.plans.loading = false;
                this.plans.loaded_at = Some(Instant::now());
                this.adopt_pay_tokens(credentials);
                match fetched {
                    Ok((plans, subscriptions)) => {
                        this.plans.items = plans;
                        this.plans.availability = PlansAvailability::Available;
                        this.plans.error = None;
                        if let Some(subscriptions) = subscriptions {
                            this.cloud_account.subscriptions = Some(subscriptions);
                        }
                    }
                    Err(error) if sub2api::session_ended(&error) => this.end_cloud_session(cx),
                    Err(error) if sub2api::pay::plans_unavailable(&error) => {
                        this.plans.items.clear();
                        this.plans.availability = PlansAvailability::Unavailable;
                        this.plans.error = None;
                    }
                    Err(error) => this.plans.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Load the plans (when stale) from a render, which cannot mutate. The
    /// flag stops a render from scheduling again while one is pending.
    pub(super) fn schedule_plans_load(&self, cx: &mut Context<Self>) {
        if self.cloud_account.credentials.is_none() || self.plans.load_scheduled.replace(true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.load_plans_if_needed(false, cx);
            });
        })
        .detach();
    }

    /// The plans that renew `group_id`'s subscription.
    pub(super) fn plans_for_group(&self, group_id: i64) -> Vec<&SubscriptionPlan> {
        self.plans
            .items
            .iter()
            .filter(|plan| plan.group_id == group_id)
            .collect()
    }

    /// Renew the subscription to `group_id`: straight to the sheet when one
    /// plan sells it, to this page when there is a choice.
    pub(super) fn renew_group(&mut self, group_id: i64, cx: &mut Context<Self>) {
        let plans = self.plans_for_group(group_id);
        if let [plan] = plans.as_slice() {
            let plan = (*plan).clone();
            self.open_plan_purchase(plan, cx);
        } else {
            self.open_settings_page(SettingsPage::Plans, cx);
        }
    }

    /// A plan order settled: pick the subscription up everywhere it shows.
    pub(super) fn activate_purchased_plan(&mut self, plan: &SubscriptionPlan, cx: &mut Context<Self>) {
        self.show_toast(tr!("plans.activated_toast", plan = plan.name.clone()));
        self.pick_up_new_subscription(cx);
    }

    /// Everything a new subscription changes, re-read: the balance and user,
    /// the groups (a subscription group appears only once held) and with
    /// them the CLI bindings, per-model routing and the group's key, the
    /// built-in agent's catalog, and this page's "current" marks.
    fn pick_up_new_subscription(&mut self, cx: &mut Context<Self>) {
        self.refresh_cloud_account(cx);
        self.load_cloud_details(cx);
        self.refresh_native_catalog(true, cx);
        self.load_plans_if_needed(true, cx);
    }

    /// A plan is being bought out of sight, in the hosted pay center: look
    /// for the subscription to appear (or its expiry to move) for a few
    /// minutes, and pick it up when it does.
    pub(super) fn poll_purchased_plan(&mut self, cx: &mut Context<Self>) {
        let before = subscription_fingerprint(self.cloud_account.subscriptions.as_deref());
        cx.spawn(async move |this, cx| {
            for _ in 0..PURCHASE_POLL_ATTEMPTS {
                cx.background_executor().timer(PURCHASE_POLL_INTERVAL).await;
                let done = this
                    .update(cx, |this, cx| {
                        if this.cloud_account.credentials.is_none() {
                            return true;
                        }
                        let now =
                            subscription_fingerprint(this.cloud_account.subscriptions.as_deref());
                        if now != before {
                            this.show_toast(tr!("plans.activated_toast_generic"));
                            this.pick_up_new_subscription(cx);
                            return true;
                        }
                        // Reads the subscriptions again for the next look.
                        this.load_plans_if_needed(true, cx);
                        false
                    })
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn render_plans_settings(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);

        if self.cloud_account.credentials.is_none() {
            return notice_card(
                theme,
                tr!("plans.sign_in_hint", name = sub2api::brand::DISPLAY_NAME),
            )
            .mt(px(15.0))
            .into_any_element();
        }

        self.schedule_plans_load(cx);

        let mut page = div()
            .mt(px(15.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(sp(12.5))
                    .line_height(sp(18.0))
                    .text_color(theme.text_secondary)
                    .child(tr!("plans.intro")),
            );

        if let Some(error) = &self.plans.error {
            page = page.child(
                notice_card(theme, error.clone()).child(
                    div().mt(px(10.0)).child(super::providers_page::card_button(
                        theme,
                        "plans-retry".into(),
                        tr!("plans.retry"),
                        false,
                        self.plans.loading,
                        cx,
                        |this, _, cx| this.load_plans_if_needed(true, cx),
                    )),
                ),
            );
        }

        match self.plans.availability {
            PlansAvailability::Unavailable => {
                return page
                    .child(
                        notice_card(theme, tr!("plans.unavailable")).child(
                            div().mt(px(10.0)).child(super::providers_page::card_button(
                                theme,
                                "plans-top-up".into(),
                                tr!("cloud.top_up"),
                                false,
                                false,
                                cx,
                                |this, _, cx| this.open_cloud_pay_modal(cx),
                            )),
                        ),
                    )
                    .into_any_element();
            }
            _ if self.plans.items.is_empty() => {
                let message = if self.plans.loading || self.plans.loaded_at.is_none() {
                    tr!("plans.loading")
                } else if self.plans.error.is_some() {
                    return page.into_any_element();
                } else {
                    tr!("plans.empty")
                };
                return page
                    .child(
                        div()
                            .text_size(sp(12.5))
                            .text_color(theme.text_ghost)
                            .child(message),
                    )
                    .into_any_element();
            }
            _ => {}
        }

        let subscriptions = self.cloud_account.subscriptions.as_deref().unwrap_or(&[]);
        let routes = self
            .cloud_account
            .credentials
            .as_ref()
            .map(|credentials| credentials.model_routes.clone())
            .unwrap_or_default();
        let font = window.text_style().font();
        let mut grid = div().w_full().flex().flex_wrap().gap(px(12.0));
        for plan in &self.plans.items {
            grid = grid.child(self.plan_card(plan, subscriptions, &routes, theme, font.clone(), cx));
        }
        page.child(grid).into_any_element()
    }

    fn plan_card(
        &self,
        plan: &SubscriptionPlan,
        subscriptions: &[SubscriptionProgress],
        routes: &BTreeMap<String, i64>,
        theme: Theme,
        font: Font,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = card_state(plan, subscriptions);
        let held = matches!(state, CardState::Renew { .. });

        let mut header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(sp(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(plan.name.clone()),
            );
        // A plan for the Chinese models is sold on an `openai` group; filed
        // by platform it read as Codex's, and "use for Codex" broke GPT.
        let lane = plan
            .platform
            .as_deref()
            .filter(|platform| !platform.is_empty())
            .map(|platform| {
                sub2api::model_routing::group_lane(plan.group_id, platform, &self.model_plaza.items)
            });
        if let Some(platform) = lane.as_deref() {
            header = header.child(
                div()
                    .flex_none()
                    .text_size(sp(11.5))
                    .text_color(theme.text_ghost)
                    .child(tr!(
                        "plans.for_cli",
                        cli = super::cloud_account::platform_display_name(platform)
                    )),
            );
        }

        let mut card = div()
            .flex_basis(px(340.0))
            .flex_grow_1()
            .min_w(px(0.0))
            .px(px(16.0))
            .py(px(14.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .border_1()
            .border_color(if held { theme.accent } else { theme.raised })
            .flex()
            .flex_col()
            .gap(px(9.0))
            .child(header);

        if let CardState::Renew { days } = state {
            // Said in words; the accent border is only a second cue.
            let mut badge = tr!("plans.current");
            if let Some(days) = days {
                badge = format!(
                    "{badge} \u{00b7} {}",
                    super::cloud_subscriptions::expiry_label(days)
                );
            }
            card = card.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(if days.is_some_and(|days| days <= 3) {
                        theme.warning
                    } else {
                        theme.success
                    })
                    .child(badge),
            );
        }

        let blurb = plan
            .description
            .as_deref()
            .or(plan.product_name.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if let Some(blurb) = blurb {
            card = card.child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.text_secondary)
                    .child(blurb.to_owned()),
            );
        }

        // Price, validity, and the discount — in words as well as struck through.
        let mut price = div()
            .flex()
            .flex_wrap()
            .items_baseline()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(sp(20.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(super::cloud_pay::format_cny(plan.price)),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_secondary)
                    .child(validity_label(plan)),
            );
        if let Some(original) = plan.discounted_from() {
            let original = super::cloud_pay::format_cny(original);
            let struck = StyledText::new(original.clone()).with_runs(vec![TextRun {
                len: original.len(),
                font,
                color: theme.text_ghost,
                background_color: None,
                underline: None,
                strikethrough: Some(gpui::StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(theme.text_ghost),
                }),
            }]);
            price = price.child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(4.0))
                    .text_size(sp(11.5))
                    .text_color(theme.text_ghost)
                    .child(tr!("plans.original_price_label"))
                    .child(struck),
            );
        }
        card = card.child(price);

        let mut facts = vec![limits_line(plan)];
        if let Some(rate) = plan
            .rate_multiplier
            .filter(|rate| *rate > 0.0 && (rate - 1.0).abs() > 1e-9)
        {
            facts.push(tr!("plans.rate", rate = format!("\u{00d7}{rate:.2}")));
        }
        card = card.child(
            div()
                .text_size(sp(12.0))
                .text_color(theme.text_secondary)
                .child(facts.join(" \u{00b7} ")),
        );

        // A held plan shows its spend, the account page's meters.
        if let Some(subscription) = subscriptions
            .iter()
            .find(|subscription| subscription.group_id() == plan.group_id)
        {
            for (label, window) in super::cloud_subscriptions::subscription_windows(subscription) {
                card = card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .text_size(sp(11.5))
                                .text_color(theme.text_secondary)
                                .child(label)
                                .child(format!(
                                    "${:.2} / ${:.2}",
                                    window.used_usd, window.limit_usd
                                )),
                        )
                        .child(super::usage_meter::meter_bar(&theme, window.percentage)),
                );
            }
        }

        if !plan.features.is_empty() {
            let mut features = div().flex().flex_col().gap(px(3.0));
            for feature in &plan.features {
                features = features.child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .text_size(sp(12.0))
                        .line_height(sp(17.0))
                        .text_color(theme.text_secondary)
                        .child(div().flex_none().text_color(theme.text_ghost).child("\u{2022}"))
                        .child(div().flex_1().min_w_0().child(feature.clone())),
                );
            }
            card = card.child(features);
        }

        if let Some(models) = models_line(plan, held, routes) {
            card = card.child(
                div()
                    .text_size(sp(11.5))
                    .line_height(sp(16.0))
                    .text_color(theme.text_ghost)
                    .child(models),
            );
        }

        let mut actions = div().mt(px(2.0)).flex().flex_wrap().gap(px(8.0));
        let buy_plan = plan.clone();
        actions = actions.child(super::providers_page::card_button(
            theme,
            SharedString::from(format!("plans-buy-{}", plan.id)),
            if held {
                tr!("plans.renew")
            } else {
                tr!("plans.buy")
            },
            true,
            false,
            cx,
            move |this, _, cx| this.open_plan_purchase(buy_plan.clone(), cx),
        ));
        // A held plan for a CLI — or for the Chinese models — routed through
        // another group sits idle for those models; offer to point it here.
        // Until the catalog lands a Chinese-models plan reads as `openai`:
        // no Codex offer before then.
        let catalog_ready = !self.model_plaza.items.is_empty();
        if held
            && let Some(platform) = lane.as_deref().filter(|platform| {
                *platform == "anthropic"
                    || (*platform == "openai" && catalog_ready)
                    || *platform == sub2api::model_routing::DOMESTIC_LANE
            })
        {
            let bound = self
                .cloud_account
                .credentials
                .as_ref()
                .and_then(|credentials| sub2api::bound_group_for_platform(credentials, platform));
            let cli = super::cloud_account::platform_display_name(platform);
            if bound == Some(plan.group_id) {
                actions = actions.child(
                    div()
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(tr!("plans.in_use", cli = cli)),
                );
            } else {
                let platform = platform.to_owned();
                let group_id = plan.group_id;
                actions = actions.child(super::providers_page::card_button(
                    theme,
                    SharedString::from(format!("plans-use-{}", plan.id)),
                    tr!("plans.use_for_cli", cli = cli),
                    false,
                    self.cloud_account.busy,
                    cx,
                    move |this, _, cx| this.select_cloud_group(platform.clone(), Some(group_id), cx),
                ));
            }
        }
        card.child(actions)
    }
}

/// A quiet card holding one message.
fn notice_card(theme: Theme, message: String) -> Div {
    div()
        .w_full()
        .px(px(20.0))
        .py(px(16.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .child(
            div()
                .text_size(sp(12.5))
                .line_height(sp(18.0))
                .text_color(theme.text_secondary)
                .child(message),
        )
}

/// Which subscriptions exist and until when — what a purchase changes.
fn subscription_fingerprint(subscriptions: Option<&[SubscriptionProgress]>) -> Vec<(i64, String)> {
    let mut fingerprint: Vec<(i64, String)> = subscriptions
        .unwrap_or_default()
        .iter()
        .map(|subscription| {
            (
                subscription.group_id(),
                subscription.subscription.expires_at.clone(),
            )
        })
        .collect();
    fingerprint.sort();
    fingerprint
}

/// Buy, or renew what the account already holds.
pub(super) fn card_state(plan: &SubscriptionPlan, subscriptions: &[SubscriptionProgress]) -> CardState {
    match subscriptions
        .iter()
        .find(|subscription| subscription.group_id() == plan.group_id)
    {
        Some(subscription) => CardState::Renew {
            days: subscription
                .progress
                .as_ref()
                .map(|progress| progress.expires_in_days),
        },
        None => CardState::Buy,
    }
}

/// The windows `plan` caps, shortest first; an absent or zero cap is none.
pub(super) fn limit_figures(plan: &SubscriptionPlan) -> Vec<(LimitWindow, f64)> {
    let Some(limits) = plan.limits.as_ref() else {
        return Vec::new();
    };
    [
        (LimitWindow::Daily, limits.daily_limit_usd),
        (LimitWindow::Weekly, limits.weekly_limit_usd),
        (LimitWindow::Monthly, limits.monthly_limit_usd),
    ]
    .into_iter()
    .filter_map(|(window, limit)| limit.filter(|limit| *limit > 0.0).map(|limit| (window, limit)))
    .collect()
}

/// `每日 $10 · 每周 $50`, or "no usage cap".
pub(super) fn limits_line(plan: &SubscriptionPlan) -> String {
    let figures = limit_figures(plan);
    if figures.is_empty() {
        return tr!("plans.unlimited");
    }
    figures
        .into_iter()
        .map(|(window, limit)| {
            let amount = format_usd(limit);
            match window {
                LimitWindow::Daily => tr!("plans.limit_daily", amount = amount),
                LimitWindow::Weekly => tr!("plans.limit_weekly", amount = amount),
                LimitWindow::Monthly => tr!("plans.limit_monthly", amount = amount),
            }
        })
        .collect::<Vec<_>>()
        .join(" \u{00b7} ")
}

/// `/ 1 个月`, `/ 30 天`.
pub(super) fn validity_label(plan: &SubscriptionPlan) -> String {
    let count = plan.validity_days.max(1);
    match plan.validity_unit {
        ValidityUnit::Day => tr!("plans.validity_day", count = count),
        ValidityUnit::Week => tr!("plans.validity_week", count = count),
        ValidityUnit::Month => tr!("plans.validity_month", count = count),
    }
}

/// `$10`, `$10.50`.
fn format_usd(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("${value:.0}")
    } else {
        format!("${value:.2}")
    }
}

/// Which models the plan serves: what the account routes through it when
/// held, the group's model scopes otherwise (a group the account does not
/// hold is absent from its catalog).
fn models_line(plan: &SubscriptionPlan, held: bool, routes: &BTreeMap<String, i64>) -> Option<String> {
    if held {
        let mut models: Vec<&str> = routes
            .iter()
            .filter(|(_, group)| **group == plan.group_id)
            .map(|(model, _)| model.as_str())
            .collect();
        if !models.is_empty() {
            models.sort_unstable();
            return Some(if models.len() > LISTED_MODELS {
                tr!(
                    "cloud.subscription_models_more",
                    models = models[..LISTED_MODELS].join(", "),
                    count = (models.len() - LISTED_MODELS).to_string()
                )
            } else {
                tr!("cloud.subscription_models", models = models.join(", "))
            });
        }
    }
    if !plan.supported_model_scopes.is_empty() {
        return Some(tr!(
            "plans.scopes",
            scopes = plan.supported_model_scopes.join(", ")
        ));
    }
    plan.default_mapped_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| tr!("plans.default_model", model = model.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sub2api::client::{SubscriptionUsage, UserSubscription};
    use sub2api::pay::PlanLimits;

    fn plan(group_id: i64) -> SubscriptionPlan {
        SubscriptionPlan {
            id: format!("plan-{group_id}"),
            group_id,
            name: "Pro".into(),
            price: 29.9,
            validity_days: 1,
            validity_unit: ValidityUnit::Month,
            ..SubscriptionPlan::default()
        }
    }

    fn held(group_id: i64, days: Option<i64>) -> SubscriptionProgress {
        SubscriptionProgress {
            subscription: UserSubscription {
                group_id,
                expires_at: "2026-10-24T00:00:00Z".into(),
                ..UserSubscription::default()
            },
            progress: days.map(|days| SubscriptionUsage {
                expires_in_days: days,
                ..SubscriptionUsage::default()
            }),
        }
    }

    #[test]
    fn a_held_group_turns_buy_into_renew() {
        assert_eq!(card_state(&plan(7), &[]), CardState::Buy);
        assert_eq!(card_state(&plan(7), &[held(8, Some(3))]), CardState::Buy);
        assert_eq!(
            card_state(&plan(7), &[held(8, None), held(7, Some(12))]),
            CardState::Renew { days: Some(12) }
        );
        assert_eq!(card_state(&plan(7), &[held(7, None)]), CardState::Renew { days: None });
    }

    #[test]
    fn only_real_caps_count_as_limits() {
        let mut capped = plan(7);
        capped.limits = Some(PlanLimits {
            daily_limit_usd: Some(10.0),
            weekly_limit_usd: Some(0.0),
            monthly_limit_usd: Some(200.5),
        });
        assert_eq!(
            limit_figures(&capped),
            vec![(LimitWindow::Daily, 10.0), (LimitWindow::Monthly, 200.5)]
        );
        assert!(limit_figures(&plan(7)).is_empty());
        assert_eq!(format_usd(10.0), "$10");
        assert_eq!(format_usd(200.5), "$200.50");
    }

    #[test]
    fn a_purchase_shows_in_the_subscription_fingerprint() {
        let fingerprint = |held: Vec<SubscriptionProgress>| subscription_fingerprint(Some(&held));
        let before = fingerprint(vec![held(8, Some(3))]);
        // A new group, or the same group running longer, both differ.
        assert_ne!(before, fingerprint(vec![held(8, Some(3)), held(7, Some(30))]));
        let mut extended = held(8, Some(33));
        extended.subscription.expires_at = "2026-11-24T00:00:00Z".into();
        assert_ne!(before, fingerprint(vec![extended]));
        // Order does not matter; nothing held is empty.
        assert_eq!(
            fingerprint(vec![held(7, None), held(8, None)]),
            fingerprint(vec![held(8, None), held(7, None)])
        );
        assert!(subscription_fingerprint(None).is_empty());
    }

    #[test]
    fn held_plans_list_their_routed_models_and_others_their_scopes() {
        let mut routes = BTreeMap::new();
        routes.insert("claude-sonnet-5".to_owned(), 7);
        routes.insert("deepseek-v4".to_owned(), 9);
        let mut offered = plan(7);
        offered.supported_model_scopes = vec!["claude".into()];

        let held_line = models_line(&offered, true, &routes).expect("line");
        assert!(held_line.contains("claude-sonnet-5"));
        assert!(!held_line.contains("deepseek-v4"));

        let offered_line = models_line(&offered, false, &routes).expect("line");
        assert!(offered_line.contains("claude"));
        assert!(!offered_line.contains("claude-sonnet-5"));

        assert_eq!(models_line(&plan(7), false, &routes), None);
    }
}
