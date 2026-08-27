//! Settings → Model Plaza: the gateway catalog, natively.
//!
//! Fork addition, aligned with the service's own web catalog
//! (`ModelCatalogView.vue`): summary figures, a group filter with counts, a
//! dynamic platform filter (whatever platforms the catalog actually carries
//! — nothing is hidden), a billing-mode filter, multi-keyword search, and a
//! card per model showing both price columns, capability badges (prompt
//! caching, long context, tiered pricing), expandable tier schedules and
//! peer groups. Group health from `/group-status` rides on each card — the
//! one thing the desktop shows that the web does not.

use std::time::{Duration, Instant};

use gpui::{ElementId, Font, StyledText, TextRun};
use sub2api::client::{GroupStatusItem, ModelCatalogItem, PriceInterval};

use super::*;

/// How long a fetched catalog stays fresh before a re-render reloads it.
const PLAZA_TTL: Duration = Duration::from_secs(60);

/// Which group's models the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PlazaSelection {
    /// Every group — the web catalog's default.
    #[default]
    All,
    Group(i64),
}

/// View state for the Model Plaza page.
#[derive(Default)]
pub(super) struct ModelPlazaState {
    /// Platform filter; `None` = all platforms.
    pub platform: Option<String>,
    /// Billing-mode filter; `None` = all modes.
    pub billing: Option<&'static str>,
    pub selection: PlazaSelection,
    /// Which model card's detail section is open, keyed `group_id:model`.
    pub expanded: Option<String>,
    pub items: Vec<ModelCatalogItem>,
    pub summary: Option<sub2api::client::CatalogSummary>,
    pub statuses: Vec<GroupStatusItem>,
    pub loading: bool,
    pub error: Option<String>,
    loaded_at: Option<Instant>,
    /// Render schedules loads; this keeps it from scheduling twice per frame.
    load_scheduled: std::cell::Cell<bool>,
}

/// The web catalog's search: every whitespace-separated keyword must appear
/// somewhere in model, display name, platform, or group name/id.
pub(super) fn matches_model_search(item: &ModelCatalogItem, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        item.model, item.display_name, item.platform, item.best_group.name, item.best_group.id
    )
    .to_lowercase();
    query
        .split_whitespace()
        .all(|keyword| haystack.contains(keyword))
}

/// Billing mode with the service's default: empty means `token`.
pub(super) fn billing_mode(item: &ModelCatalogItem) -> &str {
    if item.billing_mode.is_empty() {
        "token"
    } else {
        &item.billing_mode
    }
}

/// The figure the card leads with, per billing mode — the web catalog's
/// `getPrimaryPrice`.
pub(super) fn primary_price(item: &ModelCatalogItem) -> Option<f64> {
    let pricing = &item.effective_pricing_usd;
    match billing_mode(item) {
        "per_request" => pricing.per_request_usd,
        "image" => pricing.per_image_usd,
        _ => pricing.input_per_mtok_usd.or(pricing.output_per_mtok_usd),
    }
}

/// The web catalog's default ordering: cheapest primary price first, models
/// without a figure last, ties by name.
pub(super) fn sort_by_effective_price(items: &mut [&ModelCatalogItem]) {
    items.sort_by(|a, b| {
        match (primary_price(a), primary_price(b)) {
            (Some(left), Some(right)) => left
                .partial_cmp(&right)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.display_name.cmp(&b.display_name))
    });
}

/// `99.17%` / `98.4%` — two decimals only near the top.
fn format_availability(value: Option<f64>) -> String {
    match value {
        Some(value) if value.is_finite() => {
            if value >= 99.0 {
                format!("{value:.2}%")
            } else {
                format!("{value:.1}%")
            }
        }
        _ => "-".to_owned(),
    }
}

/// A price figure. Sub-cent prices keep four decimals so they do not render
/// as $0.00.
fn format_price(value: Option<f64>) -> String {
    match value {
        Some(value) if value.is_finite() => {
            if value != 0.0 && value.abs() < 0.01 {
                format!("${value:.4}")
            } else {
                format!("${value:.2}")
            }
        }
        _ => "--".to_owned(),
    }
}

/// `200000` → `200K`, `1048576` → `1.0M` — token thresholds and ranges.
pub(super) fn format_token_count(value: i64) -> String {
    if value >= 1_000_000 {
        let millions = value as f64 / 1_000_000.0;
        if millions.fract() == 0.0 {
            format!("{millions:.0}M")
        } else {
            format!("{millions:.1}M")
        }
    } else if value >= 1_000 {
        format!("{}K", value / 1_000)
    } else {
        value.to_string()
    }
}

/// `0–200K` / `>200K` — one tier's token range.
pub(super) fn format_interval_range(interval: &PriceInterval) -> String {
    match interval.max_tokens {
        Some(max) => format!(
            "{}\u{2013}{}",
            format_token_count(interval.min_tokens),
            format_token_count(max)
        ),
        None => format!(">{}", format_token_count(interval.min_tokens)),
    }
}

/// The status vocabulary, translated and tinted. Never color alone.
fn status_presentation(status: &str, theme: &Theme) -> (String, Hsla) {
    match status {
        "up" => (tr!("plaza.status_up"), theme.success),
        "degraded" => (tr!("plaza.status_degraded"), theme.warning),
        "down" => (tr!("plaza.status_down"), theme.danger),
        _ => (tr!("plaza.status_unknown"), theme.text_ghost),
    }
}

/// Vendor labels and badge colors, matching the service's own catalog.
fn platform_badge(platform: &str, theme: &Theme) -> (String, Hsla) {
    match platform.trim().to_lowercase().as_str() {
        "anthropic" => ("Anthropic".to_owned(), gpui::rgb(0xD97757).into()),
        "openai" => ("OpenAI".to_owned(), gpui::rgb(0x10A37F).into()),
        "grok" => ("Grok".to_owned(), gpui::rgb(0x64748B).into()),
        "gemini" => ("Gemini".to_owned(), gpui::rgb(0x4285F4).into()),
        "antigravity" => ("Antigravity".to_owned(), gpui::rgb(0x8B5CF6).into()),
        "" => ("API".to_owned(), theme.text_secondary),
        other => {
            let mut characters = other.chars();
            let label = match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => other.to_owned(),
            };
            (label, theme.text_secondary)
        }
    }
}

fn billing_mode_label(mode: &str) -> String {
    match mode {
        "per_request" => tr!("plaza.billing_per_request"),
        "image" => tr!("plaza.billing_image"),
        _ => tr!("plaza.billing_token"),
    }
}

impl Waku {
    /// Fetch the catalog and group health when stale.
    pub(super) fn load_model_plaza_if_needed(&mut self, force: bool, cx: &mut Context<Self>) {
        self.model_plaza.load_scheduled.set(false);
        if self.model_plaza.loading {
            return;
        }
        if !force
            && self
                .model_plaza
                .loaded_at
                .is_some_and(|at| at.elapsed() < PLAZA_TTL)
        {
            return;
        }
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        self.model_plaza.loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let client = sub2api::authenticated(&mut credentials)?;
                    let token = credentials.access_token.clone();
                    let catalog = client.model_catalog(&token)?;
                    // Health is decoration: a deployment without the probe
                    // endpoint must not blank the whole page.
                    let statuses = client.group_statuses(&token).unwrap_or_default();
                    anyhow::Ok((credentials, catalog, statuses))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.model_plaza.loading = false;
                this.model_plaza.loaded_at = Some(Instant::now());
                match fetched {
                    Ok((credentials, catalog, statuses)) => {
                        this.adopt_cloud_tokens(credentials);
                        this.model_plaza.items = catalog.items;
                        this.model_plaza.summary = catalog.summary;
                        this.model_plaza.statuses = statuses;
                        this.model_plaza.error = None;
                    }
                    Err(error) => this.model_plaza.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_model_plaza_settings(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                        .child(tr!("plaza.sign_in_hint", name = sub2api::brand::DISPLAY_NAME)),
                )
                .into_any_element();
        }

        // Render triggers the fetch; the flag stops it from re-scheduling on
        // every frame while the previous spawn is still in flight.
        if !self.model_plaza.load_scheduled.replace(true) {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.load_model_plaza_if_needed(false, cx);
                });
            })
            .detach();
        }

        let mut page = div().mt(px(15.0)).w_full().flex().flex_col().gap(px(12.0));

        if let Some(summary) = &self.model_plaza.summary {
            page = page.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(12.0))
                    .text_size(sp(12.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("plaza.summary_models", count = summary.total_models))
                    .child(tr!("plaza.summary_token", count = summary.token_models))
                    .child(tr!("plaza.summary_non_token", count = summary.non_token_models))
                    .child(tr!(
                        "plaza.summary_best_savings",
                        percent = format!("{:.1}", summary.max_savings_percent)
                    )),
            );
        }

        // Platform filter: "all" plus whatever the catalog actually carries,
        // so a platform added service-side shows up without an app update.
        let mut platforms: Vec<String> = self
            .model_plaza
            .items
            .iter()
            .map(|item| item.platform.trim().to_lowercase())
            .filter(|platform| !platform.is_empty())
            .collect();
        platforms.sort();
        platforms.dedup();

        let mut platform_row = div().flex().flex_wrap().items_center().gap(px(6.0));
        platform_row = platform_row.child(self.plaza_pill(
            "plaza-platform-all",
            tr!("plaza.all_platforms"),
            self.model_plaza.platform.is_none(),
            theme,
            cx.listener(|this, _, _, cx| {
                this.model_plaza.platform = None;
                cx.notify();
            }),
        ));
        for platform in &platforms {
            let (label, _) = platform_badge(platform, &theme);
            let selected = self.model_plaza.platform.as_deref() == Some(platform.as_str());
            let value = platform.clone();
            platform_row = platform_row.child(self.plaza_pill(
                SharedString::from(format!("plaza-platform-{platform}")),
                label,
                selected,
                theme,
                cx.listener(move |this, _, _, cx| {
                    this.model_plaza.platform = Some(value.clone());
                    cx.notify();
                }),
            ));
        }
        if self.model_plaza.loading {
            platform_row = platform_row.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("plaza.loading")),
            );
        }
        page = page.child(platform_row);

        // Billing-mode filter, the web catalog's second dropdown.
        let mut billing_row = div().flex().items_center().gap(px(6.0));
        billing_row = billing_row.child(self.plaza_pill(
            "plaza-billing-all",
            tr!("plaza.billing_all"),
            self.model_plaza.billing.is_none(),
            theme,
            cx.listener(|this, _, _, cx| {
                this.model_plaza.billing = None;
                cx.notify();
            }),
        ));
        for mode in ["token", "per_request", "image"] {
            let selected = self.model_plaza.billing == Some(mode);
            billing_row = billing_row.child(self.plaza_pill(
                SharedString::from(format!("plaza-billing-{mode}")),
                billing_mode_label(mode),
                selected,
                theme,
                cx.listener(move |this, _, _, cx| {
                    this.model_plaza.billing = Some(mode);
                    cx.notify();
                }),
            ));
        }
        page = page.child(billing_row);

        // Group selector + search.
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(self.render_plaza_group_selector(theme, cx))
                .child(
                    TextField::new("plaza-search-field", self.plaza_search_input.clone()).flex_1(),
                ),
        );

        if let Some(error) = &self.model_plaza.error {
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

        // Filter and sort, the web catalog's semantics.
        let query = self
            .plaza_search_input
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let mut visible: Vec<&ModelCatalogItem> = self
            .model_plaza
            .items
            .iter()
            .filter(|item| matches_model_search(item, &query))
            .filter(|item| match self.model_plaza.platform.as_deref() {
                Some(platform) => item.platform.trim().eq_ignore_ascii_case(platform),
                None => true,
            })
            .filter(|item| match self.model_plaza.billing {
                Some(mode) => billing_mode(item) == mode,
                None => true,
            })
            .filter(|item| match self.model_plaza.selection {
                PlazaSelection::All => true,
                PlazaSelection::Group(id) => item.best_group.id == id,
            })
            .collect();
        sort_by_effective_price(&mut visible);

        page = page.child(
            div()
                .text_size(sp(11.5))
                .text_color(theme.text_ghost)
                .child(tr!(
                    "plaza.filter_result",
                    visible = visible.len(),
                    total = self.model_plaza.items.len()
                )),
        );

        let font = window.text_style().font();
        let status_of = |id: i64| {
            self.model_plaza
                .statuses
                .iter()
                .find(|status| status.group_id == id)
        };
        if visible.is_empty() && !self.model_plaza.loading {
            page = page.child(
                div()
                    .text_size(sp(12.5))
                    .text_color(theme.text_ghost)
                    .child(if query.is_empty() {
                        tr!("plaza.no_models")
                    } else {
                        tr!("plaza.no_search_match")
                    }),
            );
        }
        for item in visible {
            page = page.child(self.plaza_model_card(
                item,
                status_of(item.best_group.id),
                theme,
                font.clone(),
                cx,
            ));
        }

        page.into_any_element()
    }

    /// One filter pill.
    fn plaza_pill(
        &self,
        id: impl Into<ElementId>,
        label: String,
        selected: bool,
        theme: Theme,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
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
            .text_color(if selected { theme.text } else { theme.text_secondary })
            .child(label)
            .on_click(on_click)
    }

    /// Dropdown listing all groups (with model counts) plus the all scope.
    fn render_plaza_group_selector(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        // Group tabs from best_group, like the web catalog.
        let mut counts: Vec<(i64, String, usize)> = Vec::new();
        for item in &self.model_plaza.items {
            match counts.iter_mut().find(|(id, ..)| *id == item.best_group.id) {
                Some((_, _, count)) => *count += 1,
                None => counts.push((item.best_group.id, item.best_group.name.clone(), 1)),
            }
        }
        counts.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        let total = self.model_plaza.items.len();
        let selection = self.model_plaza.selection;
        let current = match selection {
            PlazaSelection::Group(id) => counts
                .iter()
                .find(|(candidate, ..)| *candidate == id)
                .map(|(_, name, _)| name.clone())
                .unwrap_or_else(|| tr!("plaza.all_groups")),
            PlazaSelection::All => tr!("plaza.all_groups"),
        };

        let trigger = div()
            .id("plaza-group-trigger")
            .tab_index(0)
            .h(px(29.0))
            .px(px(11.0))
            .min_w(px(160.0))
            .max_w(px(260.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .cursor_default()
            .hover(|style| style.bg(theme.overlay))
            .text_size(sp(12.5))
            .text_color(theme.text)
            .child(div().min_w_0().truncate().child(current))
            .child(icon("icons/chevron-down.svg", 13.0, theme.text_secondary));

        let handle = self.menu_handle("plaza-group-menu", cx);
        let weak = cx.entity().downgrade();
        dropdown_menu(
            trigger,
            "plaza-group-menu",
            &handle,
            MenuAlign::BelowLeft,
            move |_| {
                let mut items = Vec::new();
                let all_weak = weak.clone();
                items.push(
                    MenuItem::new(
                        format!("{}  ({total})", tr!("plaza.all_groups")),
                        move |_, cx| {
                            let _ = all_weak.update(cx, |this, cx| {
                                this.model_plaza.selection = PlazaSelection::All;
                                cx.notify();
                            });
                        },
                    )
                    .selected(selection == PlazaSelection::All),
                );
                for (id, name, count) in &counts {
                    let id = *id;
                    let entry_weak = weak.clone();
                    items.push(
                        MenuItem::new(format!("{name}  ({count})"), move |_, cx| {
                            let _ = entry_weak.update(cx, |this, cx| {
                                this.model_plaza.selection = PlazaSelection::Group(id);
                                cx.notify();
                            });
                        })
                        .selected(selection == PlazaSelection::Group(id)),
                    );
                }
                items
            },
        )
    }

    /// One catalog card, the web catalog's layout with group health added.
    fn plaza_model_card(
        &self,
        item: &ModelCatalogItem,
        status: Option<&GroupStatusItem>,
        theme: Theme,
        font: Font,
        cx: &mut Context<Self>,
    ) -> Div {
        let (badge_label, badge_color) = platform_badge(&item.platform, &theme);
        let mode = billing_mode(item).to_owned();
        let name = if item.display_name.is_empty() {
            item.model.clone()
        } else {
            item.display_name.clone()
        };
        let cheaper = item.comparison.is_cheaper_than_official;
        let card_key = format!("{}:{}", item.best_group.id, item.model);
        let expanded = self.model_plaza.expanded.as_deref() == Some(card_key.as_str());

        let mut card = div()
            .w_full()
            .px(px(16.0))
            .py(px(13.0))
            .rounded(px(11.0))
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(8.0));

        // Pills: platform, billing mode, group.
        card = card.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                .child(plaza_tag(badge_label, badge_color, &theme))
                .child(plaza_tag(
                    billing_mode_label(&mode),
                    theme.text_secondary,
                    &theme,
                ))
                .child(plaza_tag(
                    item.best_group.name.clone(),
                    theme.accent,
                    &theme,
                )),
        );

        // Name + rate line, primary price on the right.
        let mut subtitle = format!("\u{00d7}{:.2}", item.best_group.rate_multiplier);
        subtitle.push_str(&format!(
            " \u{00b7} {}",
            if item.best_group.rate_source == "user_override" {
                tr!("plaza.rate_user_override")
            } else {
                tr!("plaza.rate_group_default")
            }
        ));
        if item.available_group_count > 1 {
            subtitle.push_str(&format!(
                " \u{00b7} {}",
                tr!("plaza.group_count", count = item.available_group_count)
            ));
        }
        card = card.child(
            div()
                .flex()
                .items_start()
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
                                .min_w_0()
                                .truncate()
                                .text_size(sp(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(name.clone()),
                        )
                        .when(name != item.model, |column| {
                            column.child(
                                div()
                                    .text_size(sp(11.0))
                                    .text_color(theme.text_ghost)
                                    .truncate()
                                    .child(item.model.clone()),
                            )
                        })
                        .child(
                            div()
                                .mt(px(2.0))
                                .text_size(sp(11.5))
                                .text_color(theme.text_ghost)
                                .child(subtitle),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .child(
                            div()
                                .text_size(sp(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if cheaper { theme.success } else { theme.text })
                                .child(format_price(primary_price(item))),
                        )
                        .child(
                            div()
                                .text_size(sp(10.5))
                                .text_color(theme.text_ghost)
                                .child(match mode.as_str() {
                                    "per_request" => tr!("plaza.unit_per_request"),
                                    "image" => tr!("plaza.unit_per_image"),
                                    _ => tr!("plaza.unit_per_mtok"),
                                }),
                        ),
                ),
        );

        // Health of the group the price belongs to.
        if let Some(status) = status {
            let (label, color) = status_presentation(status.effective_status(), &theme);
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(color))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .child(label),
                );
            if let Some(latency) = status.latency_ms {
                row = row.child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(tr!("plaza.latency", ms = format!("{latency:.0}"))),
                );
            }
            row = row.child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_ghost)
                    .child(format!(
                        "24h {} \u{00b7} 7d {}",
                        format_availability(status.availability_24h),
                        format_availability(status.availability_7d)
                    )),
            );
            card = card.child(row);
        }

        // Price rows: official struck through beside effective, per mode.
        let mut pricing = div().flex().flex_col().gap(px(3.0));
        let rows: Vec<(String, Option<f64>, Option<f64>)> = match mode.as_str() {
            "per_request" => vec![(
                tr!("plaza.per_request"),
                item.official_pricing.per_request_usd,
                item.effective_pricing_usd.per_request_usd,
            )],
            "image" => vec![(
                tr!("plaza.per_image"),
                item.official_pricing.per_image_usd,
                item.effective_pricing_usd.per_image_usd,
            )],
            _ => {
                let mut rows = vec![
                    (
                        tr!("plaza.input"),
                        item.official_pricing.input_per_mtok_usd,
                        item.effective_pricing_usd.input_per_mtok_usd,
                    ),
                    (
                        tr!("plaza.output"),
                        item.official_pricing.output_per_mtok_usd,
                        item.effective_pricing_usd.output_per_mtok_usd,
                    ),
                ];
                if item.effective_pricing_usd.cache_write_per_mtok_usd.is_some() {
                    rows.push((
                        tr!("plaza.cache_write"),
                        item.official_pricing.cache_write_per_mtok_usd,
                        item.effective_pricing_usd.cache_write_per_mtok_usd,
                    ));
                }
                if item.effective_pricing_usd.cache_read_per_mtok_usd.is_some() {
                    rows.push((
                        tr!("plaza.cache_read"),
                        item.official_pricing.cache_read_per_mtok_usd,
                        item.effective_pricing_usd.cache_read_per_mtok_usd,
                    ));
                }
                rows
            }
        };
        for (label, official, effective) in rows {
            pricing = pricing.child(price_row(label, official, effective, cheaper, &theme, font.clone()));
        }
        if let Some(savings) = item.comparison.savings_percent {
            pricing = pricing.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.text_ghost)
                            .child(tr!("plaza.savings")),
                    )
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if cheaper { theme.success } else { theme.text_ghost })
                            .child(format!("{savings:.1}%")),
                    ),
            );
        }
        card = card.child(pricing);

        // Capability badges, the web catalog's row.
        let mut badges = div().flex().flex_wrap().gap(px(6.0));
        let mut any_badge = false;
        if item.pricing_details.supports_prompt_caching {
            badges = badges.child(plaza_tag(tr!("plaza.prompt_caching"), theme.success, &theme));
            any_badge = true;
        }
        if item.pricing_details.has_long_context_multiplier {
            badges = badges.child(plaza_tag(
                tr!(
                    "plaza.long_context",
                    threshold = format_token_count(item.pricing_details.long_context_input_threshold)
                ),
                theme.warning,
                &theme,
            ));
            any_badge = true;
        }
        if !item.pricing_details.intervals.is_empty() {
            badges = badges.child(plaza_tag(
                tr!(
                    "plaza.tiered_pricing",
                    count = item.pricing_details.intervals.len()
                ),
                theme.text_secondary,
                &theme,
            ));
            any_badge = true;
        }
        if item.official_pricing.has_reference {
            badges = badges.child(plaza_tag(tr!("plaza.official_reference"), theme.accent, &theme));
            any_badge = true;
        }
        if any_badge {
            card = card.child(badges);
        }

        // Expandable details: tier schedule and peer groups.
        let has_details =
            !item.pricing_details.intervals.is_empty() || !item.other_groups.is_empty();
        if has_details {
            let toggle_key = card_key.clone();
            card = card.child(
                div()
                    .id(SharedString::from(format!("plaza-expand-{card_key}")))
                    .tab_index(0)
                    .text_size(sp(11.5))
                    .text_color(theme.text_secondary)
                    .cursor_default()
                    .hover(|style| style.text_color(theme.text))
                    .child(if expanded {
                        tr!("plaza.collapse_details")
                    } else {
                        tr!("plaza.expand_details")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model_plaza.expanded =
                            (this.model_plaza.expanded.as_deref() != Some(toggle_key.as_str()))
                                .then(|| toggle_key.clone());
                        cx.notify();
                    })),
            );
        }
        if expanded {
            if !item.pricing_details.intervals.is_empty() {
                let mut section = div().flex().flex_col().gap(px(4.0)).child(
                    div()
                        .text_size(sp(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_ghost)
                        .child(tr!("plaza.intervals_title")),
                );
                for interval in &item.pricing_details.intervals {
                    let tier = if interval.tier_label.is_empty() {
                        tr!("plaza.interval_default")
                    } else {
                        interval.tier_label.clone()
                    };
                    let mut details: Vec<String> = Vec::new();
                    match mode.as_str() {
                        "per_request" => {
                            if interval.per_request_usd.is_some() {
                                details.push(format!(
                                    "{} {}",
                                    tr!("plaza.per_request"),
                                    format_price(interval.per_request_usd)
                                ));
                            }
                        }
                        "image" => {
                            if interval.per_image_usd.is_some() {
                                details.push(format!(
                                    "{} {}",
                                    tr!("plaza.per_image"),
                                    format_price(interval.per_image_usd)
                                ));
                            }
                        }
                        _ => {
                            for (label, value) in [
                                (tr!("plaza.input"), interval.input_per_mtok_usd),
                                (tr!("plaza.output"), interval.output_per_mtok_usd),
                                (tr!("plaza.cache_write"), interval.cache_write_per_mtok_usd),
                                (tr!("plaza.cache_read"), interval.cache_read_per_mtok_usd),
                            ] {
                                if value.is_some() {
                                    details.push(format!("{label} {}", format_price(value)));
                                }
                            }
                        }
                    }
                    section = section.child(
                        div()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .bg(theme.overlay)
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(sp(11.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.text)
                                            .child(format_interval_range(interval)),
                                    )
                                    .child(
                                        div()
                                            .text_size(sp(10.5))
                                            .text_color(theme.text_ghost)
                                            .child(tier),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(sp(11.0))
                                    .text_color(theme.text_secondary)
                                    .child(details.join("  \u{00b7}  ")),
                            ),
                    );
                }
                card = card.child(section);
            }
            if !item.other_groups.is_empty() {
                let mut section = div().flex().flex_col().gap(px(4.0)).child(
                    div()
                        .text_size(sp(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_ghost)
                        .child(tr!("plaza.also_available")),
                );
                for companion in &item.other_groups {
                    let peer_price = match mode.as_str() {
                        "per_request" => companion.effective_pricing_usd.per_request_usd,
                        "image" => companion.effective_pricing_usd.per_image_usd,
                        _ => companion
                            .effective_pricing_usd
                            .input_per_mtok_usd
                            .or(companion.effective_pricing_usd.output_per_mtok_usd),
                    };
                    section = section.child(
                        div()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .bg(theme.overlay)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text)
                                    .child(format!(
                                        "{} \u{00b7} \u{00d7}{:.2}",
                                        companion.group.name, companion.group.rate_multiplier
                                    )),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(sp(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text_secondary)
                                    .child(format_price(peer_price)),
                            ),
                    );
                }
                card = card.child(section);
            }
        }

        card
    }
}

/// `label    $official̶ → $effective` — the official figure struck through.
fn price_row(
    label: String,
    official: Option<f64>,
    effective: Option<f64>,
    cheaper: bool,
    theme: &Theme,
    font: Font,
) -> Div {
    let official_text = format_price(official);
    let struck = StyledText::new(official_text.clone()).with_runs(vec![TextRun {
        len: official_text.len(),
        font,
        color: theme.text_ghost,
        background_color: None,
        underline: None,
        strikethrough: Some(gpui::StrikethroughStyle {
            thickness: px(1.0),
            color: Some(theme.text_ghost),
        }),
    }]);
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(sp(11.5))
                .text_color(theme.text_ghost)
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(div().text_size(sp(11.5)).child(struck))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child("\u{2192}"),
                )
                .child(
                    div()
                        .text_size(sp(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if cheaper { theme.success } else { theme.text })
                        .child(format_price(effective)),
                ),
        )
}

fn plaza_tag(label: String, tint: Hsla, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(tint.opacity(0.3))
        .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(tint))
        .child(
            div()
                .text_size(sp(11.0))
                .text_color(theme.text)
                .child(label),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sub2api::client::{GroupRef, Price};

    fn item(model: &str, platform: &str, mode: &str, input: Option<f64>) -> ModelCatalogItem {
        ModelCatalogItem {
            model: model.to_owned(),
            display_name: model.to_uppercase(),
            platform: platform.to_owned(),
            billing_mode: mode.to_owned(),
            best_group: GroupRef {
                id: 1,
                name: "Fast".to_owned(),
                rate_multiplier: 0.5,
                rate_source: "group_default".to_owned(),
            },
            effective_pricing_usd: Price {
                input_per_mtok_usd: input,
                per_request_usd: (mode == "per_request").then_some(0.01),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn search_matches_every_keyword_across_the_web_haystack() {
        let entry = item("claude-sonnet-4", "anthropic", "", Some(1.0));
        assert!(matches_model_search(&entry, ""));
        assert!(matches_model_search(&entry, "sonnet"));
        // Multi-keyword: all must match, order-free — the web behavior.
        assert!(matches_model_search(&entry, "anthropic sonnet"));
        assert!(matches_model_search(&entry, "fast claude"));
        assert!(!matches_model_search(&entry, "sonnet gpt"));
    }

    #[test]
    fn sorting_puts_cheapest_first_and_unpriced_last() {
        let expensive = item("m-expensive", "openai", "", Some(9.0));
        let cheap = item("m-cheap", "openai", "", Some(1.0));
        let unpriced = item("m-none", "openai", "", None);
        let per_request = item("m-req", "openai", "per_request", None);
        let mut visible = vec![&unpriced, &expensive, &per_request, &cheap];
        sort_by_effective_price(&mut visible);
        let order: Vec<&str> = visible.iter().map(|item| item.model.as_str()).collect();
        // per_request's primary price is its per-request figure (0.01).
        assert_eq!(order, vec!["m-req", "m-cheap", "m-expensive", "m-none"]);
    }

    #[test]
    fn token_counts_and_ranges_format_like_the_web() {
        assert_eq!(format_token_count(200_000), "200K");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(500), "500");
        let bounded = PriceInterval {
            min_tokens: 0,
            max_tokens: Some(200_000),
            ..Default::default()
        };
        assert_eq!(format_interval_range(&bounded), "0\u{2013}200K");
        let open = PriceInterval {
            min_tokens: 200_000,
            max_tokens: None,
            ..Default::default()
        };
        assert_eq!(format_interval_range(&open), ">200K");
    }

    #[test]
    fn billing_mode_defaults_to_token() {
        assert_eq!(billing_mode(&item("m", "openai", "", None)), "token");
        assert_eq!(
            billing_mode(&item("m", "openai", "per_request", None)),
            "per_request"
        );
    }
}
