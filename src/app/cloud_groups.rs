//! Settings → Cloud Account: the group picker's lanes and switching a lane's
//! group.
//!
//! Fork addition. A lane is a CLI's slot — Claude Code, Codex, the general
//! key — or the Chinese models' own ([`sub2api::model_routing::DOMESTIC_LANE`]),
//! whose groups are `openai` groups on the live gateway and so cannot be
//! filed by platform.

use sub2api::model_routing::DOMESTIC_LANE;

use super::cloud_account::{platform_display_name, section_title};
use super::*;

/// Whether a group switch was asked for or decided automatically. A manual
/// pick also records the new preference; an automatic one must not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SwitchOrigin {
    Manual,
    Auto,
}

impl Waku {
    /// Bind every lane that has groups but no selection yet to its first
    /// group. There is no "account default" choice: the server-side fallback
    /// is invisible to the user, so an explicit group keeps the routing (and
    /// the switcher menus) honest. Runs after each group fetch and each
    /// catalog load, which also migrates accounts signed in before a rule
    /// existed.
    ///
    /// The Chinese models' lane picks a subscription the account holds over
    /// pay-as-you-go, and follows the user's own earlier choice where there
    /// was one: before that lane existed, the way to reach those models was
    /// to point Codex (or the general key) at one of their groups — which
    /// broke every GPT request. Such a slot is moved back to its own lane's
    /// first group, and the group it held becomes the Chinese models' pick.
    pub(super) fn ensure_cloud_group_bindings(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.cloud_account.busy {
            return;
        }
        let pending = pending_group_bindings(
            &credentials,
            &self.cloud_account.groups,
            &self.model_plaza.items,
        );
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
                        // A catalog that landed while this ran was turned
                        // away by `busy`; what it makes bindable goes now.
                        // Each pass binds what it found, so this settles.
                        this.ensure_cloud_group_bindings(cx);
                        // The Chinese models' pick decides their routes.
                        this.refresh_model_routes(cx);
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
        // No CLI reads the Chinese models' slot: the built-in agent's live
        // sessions take it once their routes are refreshed, not at restart.
        let domestic = platform == DOMESTIC_LANE;
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
                        // The built-in agent's catalog changes with the group
                        // as well, and is not the daemon's to re-read.
                        this.refresh_native_catalog(true, cx);
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
                            this.show_toast(if domestic {
                                tr!("cloud.domestic_switched", group = name)
                            } else {
                                tr!("cloud.group_switched", group = name)
                            });
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

    /// Group picker, one section per CLI. Selecting a group rebinds that
    /// CLI's gateway key.
    pub(super) fn render_cloud_groups(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        if self.cloud_account.groups.is_empty() {
            return div();
        }
        let busy = self.cloud_account.busy;
        let mut rows = div().flex().flex_col().gap(px(6.0)).child(section_title(
            theme,
            &tr!("cloud.group_title"),
            &tr!("cloud.group_detail"),
        ));

        let catalog = &self.model_plaza.items;
        for platform in cloud_lanes(&self.cloud_account.groups, catalog) {
            let bound = self
                .cloud_account
                .credentials
                .as_ref()
                .and_then(|credentials| {
                    sub2api::bound_group_for_platform(credentials, &platform)
                });
            // The Chinese models' lane binds no CLI and falls back on its
            // own — a spent subscription hands over to pay-as-you-go — so it
            // has no failover switch, and says what picking a group means.
            let domestic = platform == DOMESTIC_LANE;
            let failover_on = !domestic && self.cloud_account.failover.enabled(&platform);
            let toggle_platform = platform.clone();
            let mut header = div()
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
                );
            if !domestic {
                header = header
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
                    ));
            }
            rows = rows.child(header);
            if domestic {
                rows = rows.child(
                    div()
                        .text_size(sp(11.5))
                        .line_height(sp(16.0))
                        .text_color(theme.text_ghost)
                        .child(tr!("cloud.lane_domestic_detail")),
                );
            }
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
                .filter(|group| cloud_lane(group, catalog) == platform)
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
}

/// The lane of the group picker a group sits in: its platform, or the
/// Chinese models' lane ([`sub2api::model_routing::group_lane`]) — the live
/// groups for DeepSeek, GLM and Kimi are `openai` groups, and filed by
/// platform they sat under Codex, where picking one broke every GPT request.
pub(super) fn cloud_lane(
    group: &sub2api::client::Group,
    catalog: &[sub2api::client::ModelCatalogItem],
) -> String {
    sub2api::model_routing::group_lane(group.id, &group.platform, catalog)
}

/// The lanes present in `groups`, in a stable, CLI-meaningful order: the two
/// first-class CLIs, the other platforms, and the Chinese models last.
pub(super) fn cloud_lanes(
    groups: &[sub2api::client::Group],
    catalog: &[sub2api::client::ModelCatalogItem],
) -> Vec<String> {
    let present: Vec<String> = groups
        .iter()
        .map(|group| cloud_lane(group, catalog))
        .filter(|lane| !lane.is_empty())
        .collect();
    let mut lanes: Vec<String> = Vec::new();
    for known in ["anthropic", "openai"] {
        if present.iter().any(|lane| lane == known) {
            lanes.push(known.to_owned());
        }
    }
    for lane in &present {
        if lane != DOMESTIC_LANE && !lanes.contains(lane) {
            lanes.push(lane.clone());
        }
    }
    if present.iter().any(|lane| lane == DOMESTIC_LANE) {
        lanes.push(DOMESTIC_LANE.to_owned());
    }
    lanes
}

/// What [`Waku::ensure_cloud_group_bindings`] binds, in order, as `(lane,
/// group)`: the Chinese models' lane first — so a CLI slot moved off one of
/// their groups hands it over — then every lane whose slot is empty, or holds
/// a group of the Chinese models it can never serve a CLI's requests through.
///
/// Without the catalog a group of the Chinese models reads as its platform
/// (`openai`), so the two lanes that tells apart wait for it.
pub(super) fn pending_group_bindings(
    credentials: &sub2api::Credentials,
    groups: &[sub2api::client::Group],
    catalog: &[sub2api::client::ModelCatalogItem],
) -> Vec<(String, i64)> {
    let catalog_ready = !catalog.is_empty();
    let lane_of = |id: i64| {
        groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| cloud_lane(group, catalog))
    };
    let domestic: Vec<&sub2api::client::Group> = groups
        .iter()
        .filter(|group| cloud_lane(group, catalog) == DOMESTIC_LANE)
        .collect();
    let mut pending = Vec::new();
    if catalog_ready && !domestic.is_empty() {
        let held = credentials
            .domestic_group_id
            .filter(|id| domestic.iter().any(|group| group.id == *id));
        if held.is_none() {
            let inherited = [credentials.codex_group_id, credentials.group_id]
                .into_iter()
                .flatten()
                .find(|id| domestic.iter().any(|group| group.id == *id));
            let pick = inherited
                .or_else(|| {
                    domestic
                        .iter()
                        .find(|group| group.is_subscription())
                        .map(|group| group.id)
                })
                .or_else(|| domestic.first().map(|group| group.id));
            if let Some(group) = pick {
                pending.push((DOMESTIC_LANE.to_owned(), group));
            }
        }
    }
    for lane in cloud_lanes(groups, catalog) {
        if lane == DOMESTIC_LANE || (!catalog_ready && lane == "openai") {
            continue;
        }
        let bound = sub2api::bound_group_for_platform(credentials, &lane);
        let holds_domestic =
            catalog_ready && bound.and_then(lane_of).as_deref() == Some(DOMESTIC_LANE);
        if bound.is_some() && !holds_domestic {
            continue;
        }
        if let Some(group) = groups.iter().find(|group| cloud_lane(group, catalog) == lane) {
            pending.push((lane, group.id));
        }
    }
    pending
}

/// `×0.50 · 99.2%` — a group's rate multiplier, 24h availability, and a
/// translated status word when it is not healthy. Empty when nothing is
/// known. Availability formatting matches the Model Plaza's.
pub(super) fn group_status_suffix(
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

#[cfg(test)]
mod tests {
    use super::*;
    use sub2api::client::{Group, GroupRef, ModelCatalogItem};

    fn group(id: i64, platform: &str, subscription: bool) -> Group {
        Group {
            id,
            name: format!("group {id}"),
            platform: platform.into(),
            subscription_type: if subscription { "subscription" } else { "standard" }.into(),
            ..Group::default()
        }
    }

    fn listed(model: &str, platform: &str, group: i64) -> ModelCatalogItem {
        ModelCatalogItem {
            model: model.into(),
            platform: platform.into(),
            best_group: GroupRef {
                id: group,
                ..GroupRef::default()
            },
            ..ModelCatalogItem::default()
        }
    }

    /// The live shape: Claude Code, Codex, and the Chinese models'
    /// subscription (14) and pay-as-you-go (15) groups — both of them
    /// `openai` groups, listed after Codex.
    fn live() -> (Vec<Group>, Vec<ModelCatalogItem>) {
        let groups = vec![
            group(1, "anthropic", false),
            group(2, "openai", false),
            group(14, "openai", true),
            group(15, "openai", false),
        ];
        let catalog = vec![
            listed("claude-sonnet-5", "anthropic", 1),
            listed("gpt-5.6-sol", "openai", 2),
            listed("deepseek-v4.1-flash", "openai", 14),
            listed("kimi-k3", "openai", 14),
            listed("deepseek-v4.1-flash", "openai", 15),
        ];
        (groups, catalog)
    }

    #[test]
    fn the_chinese_models_groups_leave_the_codex_lane() {
        let (groups, catalog) = live();
        assert_eq!(cloud_lanes(&groups, &catalog), ["anthropic", "openai", DOMESTIC_LANE]);
        let lanes: Vec<String> = groups.iter().map(|group| cloud_lane(group, &catalog)).collect();
        assert_eq!(lanes, ["anthropic", "openai", DOMESTIC_LANE, DOMESTIC_LANE]);
    }

    /// Bug 2's account: Codex pointed at the pay-as-you-go group of the
    /// Chinese models, so every GPT request 404'd. The group moves to their
    /// own slot — what the user meant by picking it — and Codex gets Codex.
    #[test]
    fn a_codex_slot_on_a_chinese_models_group_hands_it_over() {
        let (groups, catalog) = live();
        let credentials = sub2api::Credentials {
            claude_group_id: Some(1),
            codex_group_id: Some(15),
            ..sub2api::Credentials::default()
        };
        assert_eq!(
            pending_group_bindings(&credentials, &groups, &catalog),
            [(DOMESTIC_LANE.to_owned(), 15), ("openai".to_owned(), 2)]
        );
    }

    #[test]
    fn a_fresh_account_starts_the_chinese_models_on_its_subscription() {
        let (groups, catalog) = live();
        assert_eq!(
            pending_group_bindings(&sub2api::Credentials::default(), &groups, &catalog),
            [
                (DOMESTIC_LANE.to_owned(), 14),
                ("anthropic".to_owned(), 1),
                ("openai".to_owned(), 2),
            ]
        );
    }

    #[test]
    fn a_chinese_models_pick_stays_until_its_group_is_gone() {
        let (mut groups, catalog) = live();
        let credentials = sub2api::Credentials {
            claude_group_id: Some(1),
            codex_group_id: Some(2),
            domestic_group_id: Some(14),
            ..sub2api::Credentials::default()
        };
        assert!(pending_group_bindings(&credentials, &groups, &catalog).is_empty());
        // The subscription ran out and `/groups/available` stopped listing it.
        groups.retain(|group| group.id != 14);
        assert_eq!(
            pending_group_bindings(&credentials, &groups, &catalog),
            [(DOMESTIC_LANE.to_owned(), 15)]
        );
    }

    /// Before the catalog lands the Chinese models' groups read as `openai`;
    /// binding by that would put one of them on Codex again.
    #[test]
    fn without_the_catalog_neither_codex_nor_the_chinese_models_are_bound() {
        let (groups, _) = live();
        assert_eq!(
            pending_group_bindings(&sub2api::Credentials::default(), &groups, &[]),
            [("anthropic".to_owned(), 1)]
        );
        let on_the_chinese_group = sub2api::Credentials {
            claude_group_id: Some(1),
            codex_group_id: Some(15),
            ..sub2api::Credentials::default()
        };
        assert!(pending_group_bindings(&on_the_chinese_group, &groups, &[]).is_empty());
    }
}
