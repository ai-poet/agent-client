//! Settings → Usage: the managed account's request history, natively.
//!
//! Fork addition, a faithful port of the Electron client's cloud usage
//! section: period pills (today / week / month), the aggregate stat cards, and
//! a card per logged request with its token, cache, cost, and latency
//! breakdown. Data comes from `/usage/stats` and `/usage`.

use std::time::{Duration, Instant};

use super::*;

/// How long fetched usage stays fresh before a re-render triggers a reload.
const USAGE_TTL: Duration = Duration::from_secs(30);

/// Log rows per page, the web console's default.
const USAGE_PAGE_SIZE: u32 = 20;

/// View state for the usage page.
#[derive(Default)]
pub(super) struct CloudUsageState {
    /// `today` / `week` / `month`, as the API spells them.
    pub period: Option<&'static str>,
    pub stats: Option<sub2api::client::UsageStats>,
    pub logs: Vec<sub2api::client::UsageLog>,
    /// Total matching log rows, for the pagination footer.
    pub total: i64,
    /// 1-based page of the log.
    pub page: u32,
    /// Server-side filters, matching the web console's.
    pub model_filter: Option<String>,
    pub group_filter: Option<i64>,
    /// Every model id seen so far, feeding the filter dropdown.
    pub seen_models: Vec<String>,
    pub error: Option<String>,
    pub loading: bool,
    loaded_at: Option<Instant>,
    /// Render schedules loads; this keeps it from scheduling twice per frame.
    load_scheduled: std::cell::Cell<bool>,
}

impl CloudUsageState {
    fn period(&self) -> &'static str {
        self.period.unwrap_or("today")
    }

    fn page(&self) -> u32 {
        self.page.max(1)
    }

    fn page_count(&self) -> u32 {
        ((self.total.max(0) as u32).div_ceil(USAGE_PAGE_SIZE)).max(1)
    }
}

impl Waku {
    /// Fetch stats and the first page of logs when stale.
    pub(super) fn load_cloud_usage_if_needed(&mut self, force: bool, cx: &mut Context<Self>) {
        self.cloud_usage.load_scheduled.set(false);
        if self.cloud_usage.loading {
            return;
        }
        if !force
            && self
                .cloud_usage
                .loaded_at
                .is_some_and(|at| at.elapsed() < USAGE_TTL)
        {
            return;
        }
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let period = self.cloud_usage.period();
        let query = sub2api::client::UsageLogQuery {
            page: self.cloud_usage.page(),
            page_size: USAGE_PAGE_SIZE,
            model: self.cloud_usage.model_filter.clone(),
            group_id: self.cloud_usage.group_filter,
        };
        self.cloud_usage.loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let client = sub2api::authenticated(&mut credentials)?;
                    let token = credentials.access_token.clone();
                    let stats = client.usage_stats(&token, period)?;
                    let logs = client.usage_logs(&token, &query)?;
                    anyhow::Ok((credentials, stats, logs))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_usage.loading = false;
                this.cloud_usage.loaded_at = Some(Instant::now());
                match fetched {
                    Ok((credentials, stats, logs)) => {
                        this.adopt_cloud_tokens(credentials);
                        this.cloud_usage.stats = Some(stats);
                        this.cloud_usage.total = logs.total;
                        // The filter dropdown grows as models are seen; a
                        // filtered page must not shrink it again.
                        for log in &logs.items {
                            if !log.model.is_empty()
                                && !this.cloud_usage.seen_models.contains(&log.model)
                            {
                                this.cloud_usage.seen_models.push(log.model.clone());
                            }
                        }
                        this.cloud_usage.seen_models.sort();
                        this.cloud_usage.logs = logs.items;
                        this.cloud_usage.error = None;
                    }
                    Err(error) => this.cloud_usage.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Switch the aggregate period and reload.
    pub(super) fn set_cloud_usage_period(&mut self, period: &'static str, cx: &mut Context<Self>) {
        if self.cloud_usage.period() == period {
            return;
        }
        self.cloud_usage.period = Some(period);
        self.load_cloud_usage_if_needed(true, cx);
        cx.notify();
    }

    /// Change a log filter and reload from the first page.
    pub(super) fn set_cloud_usage_filters(
        &mut self,
        model: Option<String>,
        group: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.cloud_usage.model_filter = model;
        self.cloud_usage.group_filter = group;
        self.cloud_usage.page = 1;
        self.load_cloud_usage_if_needed(true, cx);
        cx.notify();
    }

    /// Move to another log page.
    pub(super) fn set_cloud_usage_page(&mut self, page: u32, cx: &mut Context<Self>) {
        let page = page.clamp(1, self.cloud_usage.page_count());
        if page == self.cloud_usage.page() {
            return;
        }
        self.cloud_usage.page = page;
        self.load_cloud_usage_if_needed(true, cx);
        cx.notify();
    }

    pub(super) fn render_cloud_usage_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);

        if self.cloud_account.credentials.is_none() {
            return div()
                .mt(px(15.0))
                .w_full()
                .px(px(20.0))
                .py(px(16.0))
                .rounded(px(13.0))
                .bg(theme.raised)
                .child(
                    div()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("cloud.not_signed_in")),
                )
                .into_any_element();
        }

        // Render triggers the fetch; the flag stops it from re-scheduling on
        // every frame while the previous spawn is still in flight.
        if !self.cloud_usage.load_scheduled.replace(true) {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.load_cloud_usage_if_needed(false, cx);
                });
            })
            .detach();
        }

        let mut page = div().mt(px(15.0)).w_full().flex().flex_col().gap(px(12.0));

        // Period pills, exactly the old client's three.
        let current = self.cloud_usage.period();
        let mut pills = div().flex().items_center().gap(px(6.0));
        for (period, label) in [
            ("today", tr!("cloud_usage.today")),
            ("week", tr!("cloud_usage.week")),
            ("month", tr!("cloud_usage.month")),
        ] {
            let selected = current == period;
            pills = pills.child(
                div()
                    .id(SharedString::from(format!("usage-period-{period}")))
                    .tab_index(0)
                    .h(px(26.0))
                    .px(px(12.0))
                    .rounded_full()
                    .border_1()
                    .border_color(if selected {
                        theme.accent
                    } else {
                        theme.border_strong
                    })
                    .when(selected, |pill| pill.bg(theme.overlay))
                    .flex()
                    .items_center()
                    .cursor_default()
                    .text_size(sp(12.0))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_secondary
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_cloud_usage_period(period, cx);
                    })),
            );
        }
        if self.cloud_usage.loading {
            pills = pills.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("cloud_usage.loading")),
            );
        }
        page = page.child(pills);

        // Server-side log filters, the web console's model and group
        // dropdowns.
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(self.render_usage_model_filter(theme, cx))
                .child(self.render_usage_group_filter(theme, cx)),
        );

        // Aggregate cards: requests / tokens / cache / cost.
        if let Some(stats) = &self.cloud_usage.stats {
            let cards = [
                (
                    tr!("cloud_usage.requests"),
                    format_count(stats.total_requests),
                    Some(tr!(
                        "cloud_usage.avg_latency",
                        ms = format!("{:.0}", stats.average_duration_ms)
                    )),
                ),
                (
                    tr!("cloud_usage.tokens"),
                    format_tokens(stats.total_tokens),
                    Some(format!(
                        "\u{2191}{} \u{2193}{}",
                        format_tokens(stats.total_input_tokens),
                        format_tokens(stats.total_output_tokens)
                    )),
                ),
                (
                    tr!("cloud_usage.cache"),
                    format_tokens(stats.total_cache_tokens),
                    None,
                ),
                (
                    tr!("cloud_usage.cost"),
                    format!("${:.4}", stats.total_actual_cost),
                    Some(tr!(
                        "cloud_usage.standard_cost",
                        cost = format!("${:.4}", stats.total_cost)
                    )),
                ),
            ];
            let mut row = div().flex().flex_wrap().gap(px(8.0));
            for (label, value, hint) in cards {
                row = row.child(
                    div()
                        .flex_1()
                        .min_w(px(120.0))
                        .px(px(14.0))
                        .py(px(12.0))
                        .rounded(px(11.0))
                        .bg(theme.raised)
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(sp(11.5))
                                .text_color(theme.text_ghost)
                                .child(label),
                        )
                        .child(
                            div()
                                .mt(px(3.0))
                                .text_size(sp(15.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(value),
                        )
                        .when_some(hint, |card, hint| {
                            card.child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(sp(11.0))
                                    .text_color(theme.text_ghost)
                                    .child(hint),
                            )
                        }),
                );
            }
            page = page.child(row);
        }

        // The request log, one card per call.
        if !self.cloud_usage.logs.is_empty() {
            page = page.child(
                div()
                    .mt(px(6.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("cloud_usage.recent")),
                    )
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.text_ghost)
                            .child(tr!(
                                "cloud_usage.total_records",
                                total = self.cloud_usage.total
                            )),
                    ),
            );
            for log in &self.cloud_usage.logs {
                page = page.child(usage_log_card(theme, log));
            }
            page = page.child(self.render_usage_pagination(theme, cx));
        } else if !self.cloud_usage.loading && self.cloud_usage.stats.is_some() {
            page = page.child(
                div()
                    .text_size(sp(12.5))
                    .text_color(theme.text_ghost)
                    .child(tr!("cloud_usage.empty")),
            );
        }

        if let Some(error) = &self.cloud_usage.error {
            page = page.child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .rounded(px(11.0))
                    .bg(theme.raised)
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_secondary)
                            .child(error.clone()),
                    ),
            );
        }

        page.into_any_element()
    }

    /// Model filter dropdown: all models plus every model id seen so far.
    fn render_usage_model_filter(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = self
            .cloud_usage
            .model_filter
            .clone()
            .unwrap_or_else(|| tr!("cloud_usage.all_models"));
        let trigger = usage_filter_trigger("usage-model-trigger", current, theme);
        let handle = self.menu_handle("usage-model-menu", cx);
        let weak = cx.entity().downgrade();
        let models = self.cloud_usage.seen_models.clone();
        let selected = self.cloud_usage.model_filter.clone();
        let group = self.cloud_usage.group_filter;
        dropdown_menu(
            trigger,
            "usage-model-menu",
            &handle,
            MenuAlign::BelowLeft,
            move |_| {
                let mut items = Vec::new();
                let all_weak = weak.clone();
                items.push(
                    MenuItem::new(tr!("cloud_usage.all_models"), move |_, cx| {
                        let _ = all_weak.update(cx, |this, cx| {
                            this.set_cloud_usage_filters(None, this.cloud_usage.group_filter, cx);
                        });
                    })
                    .selected(selected.is_none()),
                );
                for model in &models {
                    let entry_weak = weak.clone();
                    let value = model.clone();
                    items.push(
                        MenuItem::new(model.clone(), move |_, cx| {
                            let value = value.clone();
                            let _ = entry_weak.update(cx, |this, cx| {
                                this.set_cloud_usage_filters(Some(value), group, cx);
                            });
                        })
                        .selected(selected.as_deref() == Some(model.as_str())),
                    );
                }
                items
            },
        )
    }

    /// Group filter dropdown, from the account's own group list.
    fn render_usage_group_filter(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let groups: Vec<(i64, String)> = self
            .cloud_account
            .groups
            .iter()
            .map(|group| (group.id, group.name.clone()))
            .collect();
        let current = self
            .cloud_usage
            .group_filter
            .and_then(|id| {
                groups
                    .iter()
                    .find(|(candidate, _)| *candidate == id)
                    .map(|(_, name)| name.clone())
            })
            .unwrap_or_else(|| tr!("cloud_usage.all_groups"));
        let trigger = usage_filter_trigger("usage-group-trigger", current, theme);
        let handle = self.menu_handle("usage-group-menu", cx);
        let weak = cx.entity().downgrade();
        let selected = self.cloud_usage.group_filter;
        dropdown_menu(
            trigger,
            "usage-group-menu",
            &handle,
            MenuAlign::BelowLeft,
            move |_| {
                let mut items = Vec::new();
                let all_weak = weak.clone();
                items.push(
                    MenuItem::new(tr!("cloud_usage.all_groups"), move |_, cx| {
                        let _ = all_weak.update(cx, |this, cx| {
                            let model = this.cloud_usage.model_filter.clone();
                            this.set_cloud_usage_filters(model, None, cx);
                        });
                    })
                    .selected(selected.is_none()),
                );
                for (id, name) in &groups {
                    let id = *id;
                    let entry_weak = weak.clone();
                    items.push(
                        MenuItem::new(name.clone(), move |_, cx| {
                            let _ = entry_weak.update(cx, |this, cx| {
                                let model = this.cloud_usage.model_filter.clone();
                                this.set_cloud_usage_filters(model, Some(id), cx);
                            });
                        })
                        .selected(selected == Some(id)),
                    );
                }
                items
            },
        )
    }

    /// `‹  第 X / Y 页  ›` with the total alongside.
    fn render_usage_pagination(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let page = self.cloud_usage.page();
        let pages = self.cloud_usage.page_count();
        let busy = self.cloud_usage.loading;
        let arrow = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .tab_index(0)
                .w(px(28.0))
                .h(px(26.0))
                .rounded(px(7.0))
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .text_size(sp(13.0))
                .text_color(if enabled { theme.text_secondary } else { theme.text_ghost })
                .when(enabled, |button| button.hover(|style| style.bg(theme.overlay)))
                .child(label)
        };
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(
                arrow("usage-prev-page", "\u{2039}", page > 1 && !busy).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.set_cloud_usage_page(page.saturating_sub(1), cx);
                    },
                )),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_secondary)
                    .child(tr!("cloud_usage.page_of", page = page, pages = pages)),
            )
            .child(
                arrow("usage-next-page", "\u{203a}", page < pages && !busy).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.set_cloud_usage_page(page + 1, cx);
                    },
                )),
            )
    }
}

/// A compact dropdown trigger for the log filters.
fn usage_filter_trigger(id: &'static str, label: String, theme: Theme) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .h(px(27.0))
        .px(px(10.0))
        .min_w(px(120.0))
        .max_w(px(220.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(6.0))
        .cursor_default()
        .hover(|style| style.bg(theme.overlay))
        .text_size(sp(12.0))
        .text_color(theme.text)
        .child(div().min_w_0().truncate().child(label))
        .child(icon("icons/chevron-down.svg", 12.0, theme.text_secondary))
}

/// One request, in the old client's card layout: model and time up top,
/// badges, then a metrics grid.
fn usage_log_card(theme: Theme, log: &sub2api::client::UsageLog) -> Div {
    let cache_tokens = log.cache_creation_tokens + log.cache_read_tokens;
    let mut badges = div().flex().flex_wrap().items_center().gap(px(6.0));
    if let Some(group) = &log.group
        && !group.name.is_empty()
    {
        badges = badges.child(usage_badge(theme, group.name.clone(), true));
    }
    if let Some(request_type) = log.request_type.as_deref().filter(|value| !value.is_empty()) {
        badges = badges.child(usage_badge(theme, request_type.to_owned(), false));
    } else if log.stream == Some(true) {
        badges = badges.child(usage_badge(theme, "stream".to_owned(), false));
    }
    // The billed rate and the long-context surcharge, the web table's
    // columns most relevant to "why did this cost that".
    if log.rate_multiplier > 0.0 && (log.rate_multiplier - 1.0).abs() > f64::EPSILON {
        badges = badges.child(usage_badge(
            theme,
            format!("\u{00d7}{:.2}", log.rate_multiplier),
            false,
        ));
    }
    if log.long_context_billing_applied {
        badges = badges.child(usage_badge(theme, tr!("cloud_usage.long_context"), false));
    }

    div()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .rounded(px(11.0))
        .bg(theme.raised)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(sp(12.8))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .truncate()
                                .child(log.model.clone()),
                        )
                        .child(
                            div()
                                .mt(px(1.0))
                                .text_size(sp(11.0))
                                .text_color(theme.text_ghost)
                                .truncate()
                                .child(format_log_time(&log.created_at)),
                        ),
                )
                .child(badges),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(14.0))
                .child(usage_metric(
                    theme,
                    tr!("cloud_usage.tokens"),
                    format_tokens(log.input_tokens + log.output_tokens),
                    format!(
                        "\u{2191}{} \u{2193}{}",
                        format_tokens(log.input_tokens),
                        format_tokens(log.output_tokens)
                    ),
                ))
                .child(usage_metric(
                    theme,
                    tr!("cloud_usage.cache"),
                    format_tokens(cache_tokens),
                    format!(
                        "+{} / {}",
                        format_tokens(log.cache_creation_tokens),
                        format_tokens(log.cache_read_tokens)
                    ),
                ))
                .child(usage_metric(
                    theme,
                    tr!("cloud_usage.cost"),
                    format!("${:.4}", log.actual_cost),
                    tr!(
                        "cloud_usage.standard_cost",
                        cost = format!("${:.4}", log.total_cost)
                    ),
                ))
                .child(usage_metric(
                    theme,
                    tr!("cloud_usage.latency"),
                    format!("{:.1}s", log.duration_ms as f64 / 1000.0),
                    match log.first_token_ms {
                        Some(first_token) => {
                            tr!("cloud_usage.first_token", ms = first_token)
                        }
                        None => String::new(),
                    },
                ))
                .when(log.image_count > 0, |row| {
                    row.child(usage_metric(
                        theme,
                        tr!("cloud_usage.images"),
                        log.image_count.to_string(),
                        String::new(),
                    ))
                }),
        )
}

fn usage_badge(theme: Theme, label: String, accent: bool) -> Div {
    div()
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(theme.overlay)
        .text_size(sp(10.5))
        .text_color(if accent { theme.accent } else { theme.text_secondary })
        .child(label)
}

fn usage_metric(theme: Theme, label: String, value: String, hint: String) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(sp(10.5))
                .text_color(theme.text_ghost)
                .child(label),
        )
        .child(
            div()
                .text_size(sp(12.0))
                .text_color(theme.text_secondary)
                .child(value),
        )
        .when(!hint.is_empty(), |cell| {
            cell.child(
                div()
                    .text_size(sp(10.5))
                    .text_color(theme.text_ghost)
                    .child(hint),
            )
        })
}

/// `12345` → `12.3K`, matching the old client's token formatting.
fn format_tokens(value: i64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn format_count(value: i64) -> String {
    format_tokens(value)
}

/// Trim an RFC 3339 timestamp to a readable local-ish form.
///
/// The service reports UTC timestamps; full timezone math is not worth a
/// dependency for a log line, so this shows the date and time as sent.
fn format_log_time(raw: &str) -> String {
    raw.replace('T', " ")
        .split('.')
        .next()
        .unwrap_or(raw)
        .trim_end_matches('Z')
        .to_owned()
}
