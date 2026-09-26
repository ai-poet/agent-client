//! The account's subscriptions, and the per-model routing they feed.
//!
//! A subscription is a group the user has paid for by the period: requests
//! made with a key of that group bill against its daily, weekly and monthly
//! limits instead of the balance. A gateway key belongs to exactly one
//! group, so which key a model goes out with decides whether it lands on a
//! group that serves it at all. The built-in agent therefore picks a group
//! per model ([`sub2api::model_routing`]): the CLI slot's group for the
//! families a CLI speaks for (Claude, GPT, Grok), a subscription group for
//! the rest while it has room, and a pay-as-you-go group once it has run out
//! or when there is none. A model a subscription and a pay-as-you-go group
//! both serve is also offered pinned to pay-as-you-go, as a second row.
//!
//! This module re-derives that routing whenever the catalog or the group
//! list lands, and shows the subscriptions — on Settings → Cloud Account
//! and in the account menu.

use std::collections::BTreeMap;

use super::*;

/// How many of the models routed through a subscription its card names
/// before summing up the rest.
const LISTED_MODELS: usize = 6;

/// How long a message may wait for its model's key before it goes out as
/// it is. A lookup is one or two requests; this only guards a stuck one.
const ROUTE_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// The model behind picker id `id` when a turn on it must first be routed:
/// a Chinese model the catalog lists with no route — or no key for its
/// route's group — that no earlier lookup already came up empty for.
///
/// Only the Chinese models: they are the ones the routing always places
/// when the catalog lists them, so a missing route means "not worked out
/// yet". Claude and GPT may be left to their CLI slot's key on purpose. A
/// model on the user's own endpoint carries no `::` and needs no gateway
/// key at all.
pub(super) fn native_route_need(
    id: &str,
    credentials: &sub2api::Credentials,
    catalog: &[sub2api::client::ModelCatalogItem],
    misses: &std::collections::HashSet<String>,
) -> Option<String> {
    if !id.contains("::") {
        return None;
    }
    let (_, model) = super::native_agent::native_route_parts(id);
    let model = model.trim();
    if !sub2api::model_routing::is_domestic_model(model) || misses.contains(model) {
        return None;
    }
    if !catalog.iter().any(|item| item.model.trim() == model) {
        return None;
    }
    let routed = credentials
        .model_routes
        .get(model)
        .is_some_and(|group| sub2api::key_for_group(credentials, *group).is_some());
    (!routed).then(|| model.to_owned())
}

impl Waku {
    /// Re-derive which group each of the built-in agent's models goes
    /// through, and with it the subscriptions the account holds.
    ///
    /// Needs the catalog: without it there is nothing to route by, and an
    /// empty answer would wipe routes that are still right. The catalog load
    /// calls this again when it lands. One refresh at a time; a request that
    /// arrives meanwhile runs once the first is done, against the newer data.
    pub(super) fn refresh_model_routes(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.model_plaza.items.is_empty() {
            return;
        }
        // One route writer at a time: a one-model lookup running now would
        // otherwise race this refresh to mint a key for the same group.
        if self.cloud_account.routes_refreshing || self.cloud_account.route_lookup.is_some() {
            self.cloud_account.routes_stale = true;
            return;
        }
        self.cloud_account.routes_refreshing = true;
        let catalog = self.model_plaza.items.clone();
        let groups = self.cloud_account.groups.clone();
        let origin = self
            .cloud_account
            .gateway_origin
            .origin()
            .unwrap_or_else(|| credentials.endpoint.clone());

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let refresh =
                        sub2api::refresh_model_routes(&mut credentials, &catalog, &groups, &origin)?;
                    anyhow::Ok((credentials, refresh))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.routes_refreshing = false;
                match result {
                    Ok((renewed, refresh)) => {
                        this.adopt_cloud_tokens(renewed);
                        if let Some(subscriptions) = refresh.subscriptions.clone() {
                            this.cloud_account.subscriptions = Some(subscriptions);
                        }
                        let changed = this
                            .cloud_account
                            .credentials
                            .as_mut()
                            .is_some_and(|credentials| {
                                let before = (
                                    credentials.group_keys.clone(),
                                    credentials.model_routes.clone(),
                                );
                                refresh.apply_to(credentials);
                                before
                                    != (
                                        credentials.group_keys.clone(),
                                        credentials.model_routes.clone(),
                                    )
                            });
                        if changed {
                            if let Some(credentials) = this.cloud_account.credentials.as_ref()
                                && let Err(error) = credentials.save()
                            {
                                this.show_toast(format!("{error:#}"));
                            }
                            // The built-in agent reads its keys from the
                            // routing the CLIs share; rewrite it.
                            this.apply_cloud_routing();
                            // A live session read its key when it started;
                            // re-applying its options makes it read the new
                            // table, so a subscription that ran out hands
                            // over from the next turn rather than the next
                            // session.
                            this.reapply_built_in_session_options(cx);
                        }
                        // Every model was routed afresh; one a lookup found
                        // nothing for may have a group now.
                        this.cloud_account.route_misses.clear();
                        this.sync_native_models();
                    }
                    Err(error) if sub2api::session_ended(&error) => this.end_cloud_session(cx),
                    // No toast: nothing was user-initiated, and the previous
                    // routes stay in force. The next catalog load retries.
                    Err(error) => eprintln!("warning: model routing not refreshed: {error:#}"),
                }
                if std::mem::take(&mut this.cloud_account.routes_stale) {
                    this.refresh_model_routes(cx);
                }
                // Messages that waited for this refresh go out now — routed,
                // or looked up one by one where it could not route them.
                this.drain_route_sends(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The Chinese model a built-in session's next turn needs a route for,
    /// or `None` when it has one (or cannot have one).
    pub(super) fn native_route_needed(&self, session_id: Uuid) -> Option<String> {
        if !self.cloud_account.routing_enabled {
            return None;
        }
        let credentials = self.cloud_account.credentials.as_ref()?;
        let session = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)?;
        if !session.provider.is_builtin() {
            return None;
        }
        let id = self.model_for_session(session)?;
        native_route_need(
            id,
            credentials,
            &self.model_plaza.items,
            &self.cloud_account.route_misses,
        )
    }

    /// Make sure a built-in session's model has a key before its turn goes
    /// out; returns whether the turn has to wait for one.
    ///
    /// Right after signing in, the full route refresh has not finished — it
    /// waits for the catalog, the subscriptions and a key for every group —
    /// and a turn sent then went out with the general key, which the gateway
    /// answered with "no account in this group supports the model". This
    /// routes the one model at once instead: the group the routing rules
    /// pick for it, with the key the account already has there, or a new one
    /// ([`sub2api::route_one_model`]). Only one lookup runs at a time, never
    /// beside a full refresh; a turn that finds either running waits for it.
    pub(super) fn ensure_native_model_route(
        &mut self,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(model) = self.native_route_needed(session_id) else {
            return false;
        };
        if self.cloud_account.routes_refreshing || self.cloud_account.route_lookup.is_some() {
            return true;
        }
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return false;
        };
        self.cloud_account.route_lookup = Some(model.clone());
        let catalog = self.model_plaza.items.clone();
        let groups = self.cloud_account.groups.clone();
        let lookup_model = model.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let group = sub2api::route_one_model(
                        &mut credentials,
                        &catalog,
                        &groups,
                        &lookup_model,
                    )?;
                    anyhow::Ok((credentials, group))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_account.route_lookup = None;
                match result {
                    Ok((found, Some(group))) => {
                        this.adopt_cloud_tokens(found.clone());
                        let saved = this
                            .cloud_account
                            .credentials
                            .as_mut()
                            .map(|live| {
                                sub2api::merge_model_route(&found, &model, group, live);
                                live.save()
                            });
                        if let Some(Err(error)) = saved {
                            this.show_toast(format!("{error:#}"));
                        }
                        // The routing writer files the new key; the running
                        // session re-reads it before the held turn goes out.
                        this.apply_cloud_routing();
                        this.reapply_built_in_session_options(cx);
                        this.sync_native_models();
                    }
                    Ok((found, None)) => {
                        this.adopt_cloud_tokens(found);
                        this.cloud_account.route_misses.insert(model.clone());
                    }
                    Err(error) if sub2api::session_ended(&error) => this.end_cloud_session(cx),
                    Err(error) => {
                        eprintln!("warning: no route for {model}: {error:#}");
                        this.cloud_account.route_misses.insert(model.clone());
                    }
                }
                if std::mem::take(&mut this.cloud_account.routes_stale) {
                    this.refresh_model_routes(cx);
                }
                this.drain_route_sends(cx);
                cx.notify();
            });
        })
        .detach();
        true
    }

    /// Hold a message until its model has a key. Released when the lookup
    /// or refresh it waits for ends — or after [`ROUTE_WAIT`] regardless, so
    /// a stuck request never swallows it.
    pub(super) fn hold_for_route(
        &mut self,
        session_id: Uuid,
        submission: super::ComposerSubmission,
        cx: &mut Context<Self>,
    ) {
        self.cloud_account
            .pending_route_sends
            .push((session_id, submission));
        self.show_toast(tr!("native.preparing_route"));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ROUTE_WAIT).await;
            let _ = this.update(cx, |this, cx| this.release_route_sends(cx));
        })
        .detach();
        cx.notify();
    }

    /// Send what was held for a route. A message whose model still has
    /// none starts (or waits on) another lookup; one that found nothing is
    /// in `route_misses` and goes out as it is.
    pub(super) fn drain_route_sends(&mut self, cx: &mut Context<Self>) {
        for (session_id, submission) in
            std::mem::take(&mut self.cloud_account.pending_route_sends)
        {
            self.submit_submission_for_session(session_id, submission, cx);
        }
    }

    /// The wait ran out: send every held message as it is.
    fn release_route_sends(&mut self, cx: &mut Context<Self>) {
        let waiting: Vec<String> = self
            .cloud_account
            .pending_route_sends
            .iter()
            .filter_map(|(session_id, _)| self.native_route_needed(*session_id))
            .collect();
        self.cloud_account.route_misses.extend(waiting);
        self.drain_route_sends(cx);
    }

    /// Push the current options — and with them the current key table — to
    /// every built-in agent session that has a live runtime.
    fn reapply_built_in_session_options(&mut self, cx: &mut Context<Self>) {
        let live: Vec<Uuid> = self
            .state
            .sessions
            .iter()
            .filter(|session| {
                session.provider.is_builtin() && self.runtimes.contains_key(&session.id)
            })
            .map(|session| session.id)
            .collect();
        for session_id in live {
            self.apply_session_options(session_id, cx);
        }
    }

    /// The subscriptions that can take nothing more right now, by group.
    pub(super) fn exhausted_subscriptions(&self) -> std::collections::BTreeSet<i64> {
        self.cloud_account
            .subscriptions
            .iter()
            .flatten()
            .filter(|subscription| subscription.is_exhausted())
            .map(|subscription| subscription.group_id())
            .collect()
    }

    /// What the built-in agent's picker needs to label a model with the
    /// group it goes through.
    pub(super) fn native_routing(&self) -> super::native_agent::NativeRouting {
        let routes = self
            .cloud_account
            .credentials
            .as_ref()
            .map(|credentials| credentials.model_routes.clone())
            .unwrap_or_default();
        let subscriptions = self
            .cloud_account
            .subscriptions
            .iter()
            .flatten()
            .map(|subscription| (subscription.group_id(), subscription.group_name()))
            .collect();
        let group_names = self
            .cloud_account
            .groups
            .iter()
            .map(|group| (group.id, group.name.clone()))
            .collect();
        super::native_agent::NativeRouting {
            routes,
            subscriptions,
            exhausted: self.exhausted_subscriptions(),
            group_names,
        }
    }

    /// Settings → Cloud Account: one card per active subscription — its
    /// group, how long it has left, spend against each limit, and which of
    /// the built-in agent's models go through it, with a way to renew when a
    /// plan sells the group. With none, a pointer to the plans when there are
    /// any to buy; otherwise nothing, since a heading over nothing would only
    /// suggest something is missing.
    pub(super) fn render_cloud_subscriptions(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        // Renew buttons and the empty state need to know what is on sale.
        self.schedule_plans_load(cx);
        let subscriptions = match self.cloud_account.subscriptions.as_ref() {
            Some(subscriptions) if !subscriptions.is_empty() => subscriptions,
            _ if self.plans_offered() => return self.render_no_subscription(theme, cx),
            _ => return div(),
        };
        let routes = self
            .cloud_account
            .credentials
            .as_ref()
            .map(|credentials| credentials.model_routes.clone())
            .unwrap_or_default();

        let mut section = div().flex().flex_col().gap(px(8.0)).child(super::cloud_account::section_title(
            theme,
            &tr!("cloud.subscriptions_title"),
            &tr!("cloud.subscriptions_detail"),
        ));
        for subscription in subscriptions {
            let mut card = subscription_card(theme, subscription, &routes);
            let group_id = subscription.group_id();
            if !self.plans_for_group(group_id).is_empty() {
                // About to lapse: the one card worth acting on, so its button
                // is the primary one.
                let lapsing = subscription
                    .progress
                    .as_ref()
                    .is_some_and(|progress| progress.expires_in_days <= 3);
                card = card.child(
                    div().flex().child(super::providers_page::card_button(
                        theme,
                        SharedString::from(format!("cloud-subscription-renew-{group_id}")),
                        tr!("cloud.subscription_renew"),
                        lapsing,
                        false,
                        cx,
                        move |this, _, cx| this.renew_group(group_id, cx),
                    )),
                );
            }
            section = section.child(card);
        }
        section
    }

    fn render_no_subscription(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        div().flex().flex_col().gap(px(8.0)).child(
            div()
                .w_full()
                .px(px(16.0))
                .py(px(12.0))
                .rounded(px(11.0))
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
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_size(sp(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(tr!("cloud.subscriptions_empty_title")),
                        )
                        .child(
                            div()
                                .text_size(sp(12.0))
                                .line_height(sp(17.0))
                                .text_color(theme.text_secondary)
                                .child(tr!("cloud.subscriptions_empty_detail")),
                        ),
                )
                .child(super::providers_page::card_button(
                    theme,
                    "cloud-subscriptions-browse".into(),
                    tr!("plans.browse_link"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.open_settings_page(SettingsPage::Plans, cx),
                )),
        )
    }
}

/// `订阅 Pro · 12 天后到期 · 今日 $1.20 / $10.00` — one line for the account
/// menu, with the tightest window's spend when there is one.
pub(super) fn subscription_menu_line(subscription: &sub2api::client::SubscriptionProgress) -> String {
    let mut parts = vec![tr!(
        "cloud.subscription_menu",
        group = subscription.group_name()
    )];
    if let Some(days) = subscription.progress.as_ref().map(|progress| progress.expires_in_days) {
        parts.push(expiry_label(days));
    }
    if let Some((label, window)) = subscription_windows(subscription).into_iter().next() {
        parts.push(format!(
            "{label} ${:.2} / ${:.2}",
            window.used_usd, window.limit_usd
        ));
    }
    parts.join(" \u{00b7} ")
}

pub(super) fn expiry_label(days: i64) -> String {
    match days {
        ..=0 => tr!("cloud.subscription_expires_today"),
        1 => tr!("cloud.subscription_expires_tomorrow"),
        days => tr!("cloud.subscription_expires_in", days = days.to_string()),
    }
}

/// The subscription's limits, shortest window first. A window the group
/// limits but that has not been used since it reset comes back from the
/// service without figures; it is shown at zero rather than dropped, so the
/// limit itself stays visible.
pub(super) fn subscription_windows(
    subscription: &sub2api::client::SubscriptionProgress,
) -> Vec<(String, sub2api::client::SubscriptionWindow)> {
    let group = subscription.subscription.group.as_ref();
    let progress = subscription.progress.as_ref();
    let unused = |limit: Option<f64>| {
        limit
            .filter(|limit| *limit > 0.0)
            .map(|limit| sub2api::client::SubscriptionWindow {
                limit_usd: limit,
                remaining_usd: limit,
                ..Default::default()
            })
    };
    [
        (
            tr!("cloud.subscription_daily"),
            progress.and_then(|progress| progress.daily.clone()),
            group.and_then(|group| group.daily_limit_usd),
        ),
        (
            tr!("cloud.subscription_weekly"),
            progress.and_then(|progress| progress.weekly.clone()),
            group.and_then(|group| group.weekly_limit_usd),
        ),
        (
            tr!("cloud.subscription_monthly"),
            progress.and_then(|progress| progress.monthly.clone()),
            group.and_then(|group| group.monthly_limit_usd),
        ),
    ]
    .into_iter()
    .filter_map(|(label, window, limit)| window.or_else(|| unused(limit)).map(|window| (label, window)))
    .collect()
}

fn subscription_card(
    theme: Theme,
    subscription: &sub2api::client::SubscriptionProgress,
    routes: &BTreeMap<String, i64>,
) -> Div {
    let days = subscription
        .progress
        .as_ref()
        .map(|progress| progress.expires_in_days);
    let platform = subscription.platform();
    let mut title = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(sp(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(subscription.group_name()),
        );
    if !platform.is_empty() {
        title = title.child(
            div()
                .flex_none()
                .text_size(sp(11.5))
                .text_color(theme.text_ghost)
                .child(platform),
        );
    }
    if let Some(days) = days {
        title = title.child(
            div()
                .flex_none()
                .text_size(sp(12.0))
                // A subscription about to lapse is the one thing on this card
                // worth acting on; never colour alone, the words say it too.
                .text_color(if days <= 3 { theme.warning } else { theme.text_secondary })
                .child(expiry_label(days)),
        );
    }

    let mut card = div()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .rounded(px(11.0))
        .bg(theme.raised)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(title);

    let windows = subscription_windows(subscription);
    if windows.is_empty() {
        card = card.child(
            div()
                .text_size(sp(12.0))
                .text_color(theme.text_ghost)
                .child(tr!("cloud.subscription_unlimited")),
        );
    }
    for (label, window) in windows {
        card = card.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_size(sp(12.0))
                        .child(div().text_color(theme.text_secondary).child(label))
                        .child(div().text_color(theme.text_secondary).child(format!(
                            "${:.2} / ${:.2}",
                            window.used_usd, window.limit_usd
                        ))),
                )
                .child(super::usage_meter::meter_bar(&theme, window.percentage)),
        );
    }

    let mut models: Vec<&str> = routes
        .iter()
        .filter(|(_, group)| **group == subscription.group_id())
        .map(|(model, _)| model.as_str())
        .collect();
    models.sort_unstable();
    let line = if models.is_empty() {
        tr!("cloud.subscription_no_models")
    } else if models.len() > LISTED_MODELS {
        tr!(
            "cloud.subscription_models_more",
            models = models[..LISTED_MODELS].join(", "),
            count = (models.len() - LISTED_MODELS).to_string()
        )
    } else {
        tr!("cloud.subscription_models", models = models.join(", "))
    };
    card.child(
        div()
            .text_size(sp(11.5))
            .line_height(sp(16.0))
            .text_color(theme.text_ghost)
            .child(line),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sub2api::client::{
        Group, GroupRef, ModelCatalogItem, SubscriptionProgress, SubscriptionUsage,
        SubscriptionWindow, UserSubscription,
    };

    fn subscription(progress: Option<SubscriptionUsage>, group: Group) -> SubscriptionProgress {
        SubscriptionProgress {
            subscription: UserSubscription {
                group_id: group.id,
                group: Some(group),
                ..UserSubscription::default()
            },
            progress,
        }
    }

    /// A window the group limits but that has not been touched since it
    /// reset still shows, at zero; a window it does not limit does not.
    #[test]
    fn an_untouched_limit_still_shows_and_an_absent_one_does_not() {
        let group = Group {
            id: 20,
            name: "DeepSeek".into(),
            daily_limit_usd: Some(10.0),
            monthly_limit_usd: Some(200.0),
            ..Group::default()
        };
        let usage = SubscriptionUsage {
            monthly: Some(SubscriptionWindow {
                limit_usd: 200.0,
                used_usd: 50.0,
                percentage: 25.0,
                ..SubscriptionWindow::default()
            }),
            ..SubscriptionUsage::default()
        };
        let windows = subscription_windows(&subscription(Some(usage), group));
        let figures: Vec<(f64, f64)> = windows
            .iter()
            .map(|(_, window)| (window.used_usd, window.limit_usd))
            .collect();
        assert_eq!(figures, [(0.0, 10.0), (50.0, 200.0)]);
    }

    /// Bug 1: right after sign-in a pay-as-you-go-only account's DeepSeek
    /// turn went out on the general key — a Grok group — before the full
    /// refresh had routed it. Such a turn now waits for its model's key.
    #[test]
    fn a_chinese_model_without_a_keyed_route_is_looked_up_first() {
        let catalog = [ModelCatalogItem {
            model: "deepseek-v4.1-flash".into(),
            platform: "openai".into(),
            best_group: GroupRef {
                id: 15,
                ..GroupRef::default()
            },
            ..ModelCatalogItem::default()
        }];
        let none = HashSet::new();
        let fresh = sub2api::Credentials::default();
        let id = "deepseek::deepseek-v4.1-flash";
        assert_eq!(
            native_route_need(id, &fresh, &catalog, &none).as_deref(),
            Some("deepseek-v4.1-flash")
        );

        // Routed, but the group's key is not known yet: still looked up.
        let mut routed = fresh.clone();
        routed.model_routes.insert("deepseek-v4.1-flash".into(), 15);
        assert!(native_route_need(id, &routed, &catalog, &none).is_some());
        routed.group_keys.insert(15, "sk-payg".into());
        assert_eq!(native_route_need(id, &routed, &catalog, &none), None);

        // An id 0.2.3 saved on its pay-as-you-go row is the same model.
        assert!(native_route_need("deepseek+payg::deepseek-v4.1-flash", &fresh, &catalog, &none).is_some());

        // A lookup that already came up empty is not repeated per message.
        let missed: HashSet<String> = ["deepseek-v4.1-flash".to_owned()].into();
        assert_eq!(native_route_need(id, &fresh, &catalog, &missed), None);

        // The user's own endpoint: no `::`, no gateway key involved.
        assert_eq!(native_route_need("deepseek-v4.1-flash", &fresh, &catalog, &none), None);
    }

    #[test]
    fn only_the_chinese_models_the_catalog_lists_wait_for_a_route() {
        let none = HashSet::new();
        let fresh = sub2api::Credentials::default();
        let gpt = [ModelCatalogItem {
            model: "gpt-5.6-sol".into(),
            ..ModelCatalogItem::default()
        }];
        // GPT and Claude may ride their CLI slot's key on purpose.
        assert_eq!(native_route_need("openai::gpt-5.6-sol", &fresh, &gpt, &none), None);
        // Not in the catalog: there is no group to look up.
        assert_eq!(native_route_need("deepseek::deepseek-v4.1-flash", &fresh, &gpt, &none), None);
    }
}
