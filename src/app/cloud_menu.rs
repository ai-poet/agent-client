//! The cloud account's always-visible ways in: the sidebar footer chip with
//! its account menu, and the balance badge.
//!
//! Fork addition.

use super::cloud_account::{balance_color, platform_display_name};
use super::cloud_groups::{cloud_lane, cloud_lanes, group_status_suffix};
use super::*;

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
                    this.load_plans_if_needed(false, cx);
                });
            }
        });
        let weak = cx.entity().downgrade();
        let balance = self.cloud_account.user.as_ref().map(|user| user.balance);
        let plans_visible = self.plans_entry_visible();
        let subscription_lines: Vec<String> = self
            .cloud_account
            .subscriptions
            .iter()
            .flatten()
            .map(super::cloud_subscriptions::subscription_menu_line)
            .collect();
        let groups = self.cloud_account.groups.clone();
        let statuses = self.cloud_account.group_status.clone();
        let lanes: std::collections::HashMap<i64, String> = groups
            .iter()
            .map(|group| (group.id, cloud_lane(group, &self.model_plaza.items)))
            .collect();
        let bindings: Vec<(String, Option<i64>)> = cloud_lanes(&groups, &self.model_plaza.items)
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
                    if plans_visible {
                        let plans_weak = weak.clone();
                        items.push(
                            MenuItem::new(tr!("plans.menu_item"), move |_, cx| {
                                let _ = plans_weak.update(cx, |this, cx| {
                                    this.open_settings_page(SettingsPage::Plans, cx);
                                });
                            })
                            .icon("icons/zap.svg"),
                        );
                    }
                    items.push(MenuItem::Separator);
                }
                // What the user has paid for by the period, and how far into
                // it they are: read-only, the account page has the detail.
                if !subscription_lines.is_empty() {
                    for line in &subscription_lines {
                        items.push(MenuItem::Header(SharedString::from(line.clone())));
                    }
                    items.push(MenuItem::Separator);
                }

                // One submenu per CLI: pick the group its traffic routes
                // through, or drop back to the account default.
                for (platform, bound) in &bindings {
                    // (id, plain name, menu label with rate + 24h availability).
                    let platform_groups: Vec<(i64, String, String)> = groups
                        .iter()
                        .filter(|group| lanes.get(&group.id) == Some(platform))
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
