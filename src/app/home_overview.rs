//! The welcome screen's activity overview.
//!
//! Fork addition, after dsh-claude-style's home panel (MIT, © Nwflower): six
//! tiles (sessions, messages, active days, peak hour, favorite model, longest
//! streak) over All / 30 days / 7 days, a 26-week heat grid of messages per
//! day, and a Models tab ranking the models by messages.
//!
//! The figures come from this app's own history: the daemon reads them out
//! of the task store (`Command::LoadActivityOverview`) whenever the welcome
//! screen shows and the last read is older than [`REFRESH_AFTER`]. Frames
//! only read the folded result; a range switch re-folds the records in
//! memory (`waku_protocol::activity_overview`).

use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use waku_protocol::activity_overview::{
    self as overview, ActivityRange, ActivityRecords, ActivitySummary, HeatGrid,
};

use super::*;

/// The welcome screen re-reads the history at most this often.
const REFRESH_AFTER: Duration = Duration::from_secs(30);
/// Weeks of history in the heat grid.
const HEAT_WEEKS: usize = 26;
const HEAT_CELL: f32 = 15.0;
const HEAT_GAP: f32 = 3.0;
/// Models shown before "Show N more".
const MODELS_FOLDED: usize = 6;
const PANEL_WIDTH: f32 = 520.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum OverviewTab {
    #[default]
    Overview,
    Models,
}

#[derive(Default)]
pub(super) struct HomeOverviewState {
    records: Option<Arc<ActivityRecords>>,
    loading: bool,
    loaded_at: Option<Instant>,
    tab: OverviewTab,
    range: ActivityRange,
    models_expanded: bool,
    /// Folded for `computed_for`; recomputed on a load or a range switch.
    summary: Option<ActivitySummary>,
    grid: Option<HeatGrid>,
    computed_for: Option<(ActivityRange, NaiveDate)>,
}

impl HomeOverviewState {
    fn recompute(&mut self, today: NaiveDate) {
        let Some(records) = &self.records else {
            return;
        };
        self.summary = Some(overview::summarize(records, self.range, today));
        self.grid = Some(overview::heat_grid(records, today, HEAT_WEEKS));
        self.computed_for = Some((self.range, today));
    }
}

impl Waku {
    /// From the window's render, while the welcome screen is up: start a
    /// background read when there is none yet or the last one is stale.
    /// Cheap when nothing is due — two field reads and a clock compare.
    pub(super) fn maybe_refresh_home_overview(&mut self, cx: &mut Context<Self>) {
        let state = &self.home_overview;
        if state.loading
            || state
                .loaded_at
                .is_some_and(|loaded| loaded.elapsed() < REFRESH_AFTER)
        {
            return;
        }
        self.home_overview.loading = true;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        waku_client::Command::LoadActivityOverview,
                    )? {
                        waku_client::ResponsePayload::ActivityOverview { records } => Ok(records),
                        _ => anyhow::bail!("the daemon returned an invalid activity response"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.home_overview;
                state.loading = false;
                state.loaded_at = Some(Instant::now());
                match loaded {
                    Ok(records) => {
                        state.records = Some(Arc::new(records));
                        state.recompute(Local::now().date_naive());
                        cx.notify();
                    }
                    // The welcome screen simply shows no overview.
                    Err(error) => eprintln!("warning: activity overview: {error}"),
                }
            });
        })
        .detach();
    }

    fn set_home_overview_range(&mut self, range: ActivityRange, cx: &mut Context<Self>) {
        if self.home_overview.range == range {
            return;
        }
        self.home_overview.range = range;
        self.home_overview.recompute(Local::now().date_naive());
        cx.notify();
    }

    fn set_home_overview_tab(&mut self, tab: OverviewTab, cx: &mut Context<Self>) {
        if self.home_overview.tab != tab {
            self.home_overview.tab = tab;
            cx.notify();
        }
    }

    /// The panel under the welcome headline; nothing until the history has
    /// been read, and nothing for a history without a single message.
    pub(super) fn render_home_overview(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = &self.home_overview;
        state.records.as_ref().filter(|records| !records.is_empty())?;
        let summary = state.summary.as_ref()?;
        let grid = state.grid.as_ref()?;
        let theme = Theme::current(cx);

        let body = match state.tab {
            OverviewTab::Overview => div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(self.render_overview_tiles(summary, &theme))
                .child(render_heat_grid(grid, &theme))
                .into_any_element(),
            OverviewTab::Models => self.render_overview_models(summary, &theme, cx),
        };
        Some(
            div()
                .id("home-overview")
                .mt(px(24.0))
                .w(px(PANEL_WIDTH))
                .max_w_full()
                .p(px(12.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(self.render_overview_head(&theme, cx))
                .child(body)
                .into_any_element(),
        )
    }

    fn render_overview_head(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &self.home_overview;
        let mut tabs = div().flex().items_center().gap(px(2.0));
        for (tab, label, id) in [
            (OverviewTab::Overview, tr!("home_overview.tab_overview"), "overview"),
            (OverviewTab::Models, tr!("home_overview.tab_models"), "models"),
        ] {
            tabs = tabs.child(
                overview_pill(id, label, state.tab == tab, theme).on_click(cx.listener(
                    move |this, _, _, cx| this.set_home_overview_tab(tab, cx),
                )),
            );
        }
        let mut ranges = div().flex().items_center().gap(px(2.0));
        for (range, label, id) in [
            (ActivityRange::All, tr!("home_overview.range_all"), "all"),
            (ActivityRange::Days30, tr!("home_overview.range_30d"), "30d"),
            (ActivityRange::Days7, tr!("home_overview.range_7d"), "7d"),
        ] {
            ranges = ranges.child(
                overview_pill(id, label, state.range == range, theme).on_click(cx.listener(
                    move |this, _, _, cx| this.set_home_overview_range(range, cx),
                )),
            );
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(tabs)
            .child(ranges)
    }

    fn render_overview_tiles(&self, summary: &ActivitySummary, theme: &Theme) -> impl IntoElement {
        let peak_hour = summary.peak_hour.map(hour_label);
        let streak = (summary.longest_streak > 0).then(|| {
            if summary.longest_streak == 1 {
                tr!("home_overview.streak_one")
            } else {
                tr!("home_overview.streak_days", count = summary.longest_streak)
            }
        });
        let tiles = [
            (
                tr!("home_overview.sessions"),
                Some(group_digits(u64::from(summary.sessions))),
                true,
            ),
            (
                tr!("home_overview.messages"),
                Some(group_digits(summary.messages)),
                true,
            ),
            (
                tr!("home_overview.active_days"),
                Some(group_digits(u64::from(summary.active_days))),
                true,
            ),
            (tr!("home_overview.peak_hour"), peak_hour, true),
            (
                tr!("home_overview.favorite_model"),
                summary.favorite_model.clone(),
                false,
            ),
            (tr!("home_overview.longest_streak"), streak, true),
        ];
        let mut rows = div().flex().flex_col().gap(px(6.0));
        for chunk in tiles.chunks(3) {
            let mut row = div().flex().gap(px(6.0));
            for (label, value, strong) in chunk {
                row = row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(px(10.0))
                        .py(px(7.0))
                        .rounded(px(8.0))
                        .bg(theme.overlay)
                        .child(
                            div()
                                .truncate()
                                .text_size(sp(11.5))
                                .line_height(sp(15.0))
                                .text_color(theme.text_tertiary)
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .mt(px(2.0))
                                .truncate()
                                .text_size(sp(13.5))
                                .line_height(sp(18.0))
                                .font_weight(if *strong {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .text_color(theme.text)
                                .child(value.clone().unwrap_or_else(|| "—".to_owned())),
                        ),
                );
            }
            rows = rows.child(row);
        }
        rows
    }

    fn render_overview_models(
        &self,
        summary: &ActivitySummary,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total: u64 = summary.models.iter().map(|share| share.messages).sum();
        if summary.models.is_empty() || total == 0 {
            return div()
                .py(px(18.0))
                .flex()
                .justify_center()
                .text_size(sp(12.5))
                .text_color(theme.text_tertiary)
                .child(tr!("home_overview.no_models"))
                .into_any_element();
        }
        let expanded = self.home_overview.models_expanded;
        let shown = if expanded {
            summary.models.len()
        } else {
            summary.models.len().min(MODELS_FOLDED)
        };

        // One bar split by share, in rank order.
        let mut bar = div()
            .h(px(8.0))
            .w_full()
            .rounded(px(4.0))
            .overflow_hidden()
            .flex()
            .bg(theme.overlay);
        for (rank, share) in summary.models.iter().enumerate() {
            let fraction = share.messages as f32 / total as f32;
            if fraction <= 0.0 {
                continue;
            }
            bar = bar.child(
                div()
                    .h_full()
                    .w(gpui::relative(fraction))
                    .bg(rank_color(theme, rank)),
            );
        }

        let mut list = div().flex().flex_col().gap(px(2.0));
        for (rank, share) in summary.models.iter().take(shown).enumerate() {
            let name = if share.model.is_empty() {
                tr!("home_overview.unnamed_model")
            } else {
                share.model.clone()
            };
            let percent = share.messages as f64 * 100.0 / total as f64;
            list = list.child(
                div()
                    .h(px(30.0))
                    .px(px(6.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(rank_color(theme, rank)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(sp(12.5))
                            .text_color(theme.text)
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(sp(11.5))
                            .text_color(theme.text_tertiary)
                            .child(tr!(
                                "home_overview.model_detail",
                                messages = group_digits(share.messages),
                                sessions = group_digits(u64::from(share.sessions))
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(48.0))
                            .flex()
                            .justify_end()
                            .text_size(sp(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_secondary)
                            .child(format!("{percent:.1}%")),
                    ),
            );
        }
        let hidden = summary.models.len().saturating_sub(MODELS_FOLDED);
        let toggle = (hidden > 0).then(|| {
            div()
                .id("home-overview-models-toggle")
                .tab_index(0)
                .mt(px(2.0))
                .h(px(24.0))
                .px(px(6.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .cursor_default()
                .text_size(sp(12.0))
                .text_color(theme.text_tertiary)
                .hover(|element| element.bg(theme.overlay).text_color(theme.text_secondary))
                .child(if expanded {
                    tr!("home_overview.show_less")
                } else {
                    tr!("home_overview.show_more", count = hidden)
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.home_overview.models_expanded = !this.home_overview.models_expanded;
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(bar)
            .child(list.children(toggle))
            .into_any_element()
    }
}

fn overview_pill(
    id: &'static str,
    label: String,
    selected: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("home-overview-{id}")))
        .tab_index(0)
        .h(px(22.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .cursor_default()
        .text_size(sp(12.0))
        .text_color(if selected {
            theme.text
        } else {
            theme.text_tertiary
        })
        .when(selected, |pill| pill.bg(theme.overlay_strong))
        .when(!selected, |pill| {
            pill.hover(|element| element.bg(theme.overlay).text_color(theme.text_secondary))
        })
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .child(label)
}

fn render_heat_grid(grid: &HeatGrid, theme: &Theme) -> impl IntoElement {
    let mut columns = div().flex().gap(px(HEAT_GAP));
    for week in 0..grid.weeks {
        let mut column = div().flex().flex_col().gap(px(HEAT_GAP));
        for weekday in 0..7 {
            let index = week * 7 + weekday;
            let cell = div().size(px(HEAT_CELL)).rounded(px(3.0));
            column = column.child(match grid.cells[index] {
                // A day still to come this week.
                None => cell.into_any_element(),
                Some(count) => {
                    let level = overview::heat_level(count, grid.peak);
                    let date = grid.day(week, weekday);
                    cell.id(SharedString::from(format!("heat-{index}")))
                        .bg(heat_color(theme, level))
                        .tooltip(Tooltip::text(day_tip(date, count)))
                        .into_any_element()
                }
            });
        }
        columns = columns.child(column);
    }
    let total: u64 = grid.cells.iter().flatten().map(|count| u64::from(*count)).sum();
    let mut legend = div()
        .flex()
        .items_center()
        .gap(px(3.0))
        .child(div().mr(px(3.0)).child(tr!("home_overview.less")));
    for level in 0..=4 {
        legend = legend.child(
            div()
                .size(px(10.0))
                .rounded(px(2.0))
                .bg(heat_color(theme, level)),
        );
    }
    legend = legend.child(div().ml(px(3.0)).child(tr!("home_overview.more")));
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(div().w_full().flex().justify_center().child(columns))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_size(sp(11.5))
                .text_color(theme.text_tertiary)
                .child(tr!("home_overview.heat_caption", count = group_digits(total)))
                .child(legend),
        )
}

fn heat_color(theme: &Theme, level: u8) -> Hsla {
    match level {
        0 => theme.text.opacity(0.07),
        1 => theme.gauge.opacity(0.3),
        2 => theme.gauge.opacity(0.5),
        3 => theme.gauge.opacity(0.75),
        _ => theme.gauge,
    }
}

fn rank_color(theme: &Theme, rank: usize) -> Hsla {
    const SHADES: [f32; 6] = [1.0, 0.78, 0.6, 0.46, 0.34, 0.24];
    match SHADES.get(rank) {
        Some(opacity) => theme.gauge.opacity(*opacity),
        None => theme.text_ghost,
    }
}

/// "3 PM" / "15 点" / "15 時".
fn hour_label(hour: u8) -> String {
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    tr!(
        "home_overview.hour",
        hour12 = hour12,
        hour24 = hour,
        meridiem = if hour < 12 { "AM" } else { "PM" }
    )
}

fn day_tip(date: NaiveDate, count: u32) -> String {
    let date = tr!(
        "home_overview.date",
        month = date.month(),
        month_name = date.format("%b").to_string(),
        day = date.day()
    );
    match count {
        0 => tr!("home_overview.day_tip_none", date = date),
        1 => tr!("home_overview.day_tip_one", date = date),
        count => tr!(
            "home_overview.day_tip",
            date = date,
            count = group_digits(u64::from(count))
        ),
    }
}

/// `13262` → `13,262`.
fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_group_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(13_262), "13,262");
        assert_eq!(group_digits(1_234_567), "1,234,567");
    }
}
