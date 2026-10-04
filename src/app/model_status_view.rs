//! What the model status page looks like — the web console's
//! `ModelStatusView.vue`, in GPUI: summary figures, a card per group, a
//! details dialog, and the fingerprint benchmark notes.
//!
//! Fork addition; the state and the requests are in `model_status.rs`.

use std::rc::Rc;

use gpui::{AnyView, relative};
use sub2api::group_status::{
    self, AstraCheckState, EventTone, FingerprintStatus, GroupStatusEntry, GroupStatusEvent,
    GroupStatusRecord, HistoryBucket, HistoryPeriod, Preview, RuntimeStatus,
};

use super::model_plaza::{platform_badge, plaza_tag};
use super::model_status::{local_time, local_time_unix, time_ago};
use super::providers_page::card_button;
use super::*;
use crate::ui::ActivationExt as _;

const PAGE_MAX_WIDTH: f32 = 984.0;

fn status_color(status: RuntimeStatus, theme: &Theme) -> Hsla {
    match status {
        RuntimeStatus::Up => theme.success,
        RuntimeStatus::Degraded => theme.warning,
        RuntimeStatus::Down => theme.danger,
        RuntimeStatus::Unknown => theme.text_ghost,
    }
}

fn status_label(status: RuntimeStatus) -> String {
    match status {
        RuntimeStatus::Up => tr!("model_status.status_up"),
        RuntimeStatus::Degraded => tr!("model_status.status_degraded"),
        RuntimeStatus::Down => tr!("model_status.status_down"),
        RuntimeStatus::Unknown => tr!("model_status.status_unknown"),
    }
}

fn fingerprint_color(status: FingerprintStatus, theme: &Theme) -> Hsla {
    match status {
        FingerprintStatus::Pass => theme.success,
        FingerprintStatus::Mismatch => theme.danger,
        FingerprintStatus::Suspect | FingerprintStatus::Insufficient => theme.warning,
        FingerprintStatus::Unknown => theme.text_ghost,
    }
}

fn fingerprint_label(status: FingerprintStatus) -> String {
    match status {
        FingerprintStatus::Pass => tr!("model_status.fingerprint_status_pass"),
        FingerprintStatus::Mismatch => tr!("model_status.fingerprint_status_mismatch"),
        FingerprintStatus::Suspect => tr!("model_status.fingerprint_status_suspect"),
        FingerprintStatus::Insufficient => tr!("model_status.fingerprint_status_insufficient"),
        FingerprintStatus::Unknown => tr!("model_status.fingerprint_status_unknown"),
    }
}

/// A fingerprint candidate's name, "another model" for the catch-all.
fn model_name(model: &str) -> String {
    if model.trim() == group_status::ASTRA_OTHER_MODEL {
        tr!("model_status.fingerprint_other_model")
    } else {
        group_status::model_label(model)
    }
}

/// One expected model's badge: its name, the verdict, and where a mismatch
/// points.
fn fingerprint_badge_text(state: &AstraCheckState) -> String {
    let model = if state.display_name.trim().is_empty() {
        model_name(&state.expected_model)
    } else {
        state.display_name.trim().to_owned()
    };
    match state.status() {
        FingerprintStatus::Mismatch if !state.winner.trim().is_empty() => tr!(
            "model_status.fingerprint_mismatch",
            model = model,
            winner = model_name(&state.winner)
        ),
        FingerprintStatus::Mismatch => {
            tr!("model_status.fingerprint_mismatch_no_winner", model = model)
        }
        FingerprintStatus::Pass => tr!("model_status.fingerprint_pass", model = model),
        FingerprintStatus::Suspect => tr!("model_status.fingerprint_suspect", model = model),
        FingerprintStatus::Insufficient => {
            tr!("model_status.fingerprint_insufficient", model = model)
        }
        FingerprintStatus::Unknown => tr!("model_status.fingerprint_pending", model = model),
    }
}

fn event_type_label(event_type: &str) -> String {
    match event_type {
        "up" => tr!("model_status.event_up"),
        "down" => tr!("model_status.event_down"),
        "astra_mismatch" => tr!("model_status.event_astra_mismatch"),
        "astra_recovered" => tr!("model_status.event_astra_recovered"),
        "modeltrace_mismatch" => tr!("model_status.event_modeltrace_mismatch"),
        "modeltrace_recovered" => tr!("model_status.event_modeltrace_recovered"),
        "sol_juice_mismatch" => tr!("model_status.event_sol_juice_mismatch"),
        "sol_juice_recovered" => tr!("model_status.event_sol_juice_recovered"),
        other => other.to_owned(),
    }
}

/// Which expected model a fingerprint event is about (and where a mismatch
/// points); nothing for other events or old ones that do not say.
fn fingerprint_event_text(event: &GroupStatusEvent) -> Option<String> {
    if !group_status::is_astra_check_event(&event.event_type) {
        return None;
    }
    let models = group_status::parse_astra_event_sub_status(&event.sub_status);
    if models.expected.is_empty() {
        return None;
    }
    let model = model_name(&models.expected);
    Some(
        if event.event_type == "astra_mismatch" && !models.winner.is_empty() {
            tr!(
                "model_status.fingerprint_event_mismatch",
                model = model,
                winner = model_name(&models.winner)
            )
        } else {
            model
        },
    )
}

/// An event's from/to state: pass/mismatch for fingerprint events, a
/// runtime status otherwise.
fn event_state(event: &GroupStatusEvent, raw: &str, theme: &Theme) -> (String, Hsla) {
    if group_status::is_fingerprint_event(&event.event_type) {
        let status = group_status::fingerprint_status(raw, "");
        (fingerprint_label(status), fingerprint_color(status, theme))
    } else {
        let status = RuntimeStatus::from_wire(raw);
        (status_label(status), status_color(status, theme))
    }
}

/// A tinted badge; the text always names the state, so colour is never the
/// only signal.
fn badge(label: String, tint: Hsla, theme: &Theme) -> Div {
    div()
        .flex_none()
        .px(px(8.0))
        .py(px(2.0))
        .rounded_full()
        .bg(if tint == theme.text_ghost {
            theme.overlay
        } else {
            tint.opacity(0.14)
        })
        .text_size(sp(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if tint == theme.text_ghost {
            theme.text_secondary
        } else {
            tint
        })
        .child(label)
}

fn preview_text(entry: &GroupStatusEntry) -> String {
    match entry.preview() {
        Preview::Text(text) => text,
        Preview::WaitingForProbe => tr!("model_status.waiting_for_probe"),
        Preview::NotAvailable => tr!("model_status.not_available"),
    }
}

/// The heartbeat cell's hover card.
fn record_lines(record: &GroupStatusRecord) -> Vec<SharedString> {
    let mut lines = vec![
        local_time(&record.observed_at).into(),
        status_label(RuntimeStatus::from_wire(&record.status)).into(),
        tr!(
            "model_status.latency_line",
            value = group_status::format_latency(record.latency_ms)
        )
        .into(),
        tr!(
            "model_status.http_code",
            code = record
                .http_code
                .map_or_else(|| "-".to_owned(), |code| code.to_string())
        )
        .into(),
    ];
    if !record.sub_status.is_empty() {
        lines.push(tr!("model_status.sub_status", value = record.sub_status.clone()).into());
    }
    lines
}

fn bucket_lines(bucket: &HistoryBucket) -> Vec<SharedString> {
    vec![
        format!(
            "{} - {}",
            local_time(&bucket.bucket_start),
            local_time(&bucket.bucket_end)
        )
        .into(),
        tr!(
            "model_status.bucket_availability",
            value = group_status::format_availability(Some(bucket.availability))
        )
        .into(),
        tr!("model_status.sample_count", count = bucket.total_count).into(),
        tr!(
            "model_status.avg_latency",
            value = group_status::format_latency(bucket.avg_latency_ms)
        )
        .into(),
    ]
}

/// A hover card of several lines — the web page's multi-line `title`.
/// (`ui::Tooltip` is a single line.)
struct StatusTooltip {
    lines: Vec<SharedString>,
}

impl StatusTooltip {
    fn build(lines: Vec<SharedString>, cx: &mut App) -> AnyView {
        cx.new(|_| Self { lines }).into()
    }
}

impl Render for StatusTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        div().pt(px(4.0)).pl(px(2.0)).child(
            div()
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.raised)
                .shadow_md()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .text_size(sp(12.0))
                .line_height(sp(16.0))
                .text_color(theme.text_secondary)
                .children(self.lines.iter().cloned()),
        )
    }
}

/// One of the four figures above the cards, and the probe metrics on them.
fn metric_tile(label: String, value: String, extra: Option<String>, theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w(px(150.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .bg(theme.overlay)
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(
            div()
                .text_size(sp(11.0))
                .text_color(theme.text_ghost)
                .child(label),
        )
        .child(
            div()
                .text_size(sp(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(value),
        )
        .when_some(extra, |tile, extra| {
            tile.child(
                div()
                    .text_size(sp(11.0))
                    .text_color(theme.text_ghost)
                    .child(extra),
            )
        })
}

fn section_box(theme: &Theme) -> Div {
    div()
        .w_full()
        .p(px(16.0))
        .rounded(px(14.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.raised)
        .flex()
        .flex_col()
        .gap(px(10.0))
}

impl Waku {
    // ── Sidebar ──────────────────────────────────────────────────────────

    /// The page's sidebar row, under "Images", marked while it is open.
    pub(super) fn render_sidebar_model_status(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let open = self.model_status.open;
        self.render_sidebar_action_row(
            "sidebar-model-status",
            "icons/server.svg",
            tr!("sidebar.model_status"),
            cx,
        )
        .when(open, |element| element.bg(theme.sidebar_item_background))
        .on_activation(cx, |this, _, cx| this.open_model_status(cx))
    }

    // ── Page ─────────────────────────────────────────────────────────────

    /// Everything under the header while the page is open.
    pub(super) fn render_model_status(&self, cx: &mut Context<Self>) -> AnyElement {
        self.schedule_model_status_load(cx);
        let theme = Theme::current(cx);
        div()
            .id("model-status")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .flex_col()
            .child(self.render_model_status_toolbar(theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(
                        div()
                            .id("model-status-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.model_status.scroll)
                            .px(px(20.0))
                            .pb(px(24.0))
                            .child(self.render_model_status_content(theme, cx)),
                    )
                    .child(scrollbar::vertical(
                        &self.model_status.scroll,
                        &self.model_status.scrollbar,
                    )),
            )
            .into_any_element()
    }

    fn render_model_status_toolbar(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = &self.model_status;
        let signed_in = self.cloud_account.credentials.is_some();
        let updated = state.updated_at.map_or_else(
            || tr!("model_status.not_available"),
            |at| {
                let ago = u64::try_from(sub2api::auth::now_unix() - at).unwrap_or(0);
                format!(
                    "{} ({})",
                    super::sidebar::format_time_ago(ago),
                    local_time_unix(at)
                )
            },
        );
        div().flex_none().px(px(20.0)).pb(px(8.0)).child(
            div()
                .w_full()
                .max_w(px(PAGE_MAX_WIDTH))
                .mx_auto()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .text_size(sp(12.0))
                        .text_color(theme.text_tertiary)
                        .child(tr!("model_status.auto_refresh"))
                        .when(signed_in, |column| {
                            column.child(
                                div()
                                    .text_color(theme.text_ghost)
                                    .child(tr!("model_status.last_updated", time = updated)),
                            )
                        }),
                )
                .child(div().flex_1())
                .children(self.render_cloud_balance_badge(cx))
                .when(signed_in, |row| {
                    row.child(card_button(
                        theme,
                        SharedString::from("model-status-refresh"),
                        if state.loading {
                            tr!("model_status.refreshing")
                        } else {
                            tr!("model_status.refresh")
                        },
                        false,
                        state.loading,
                        cx,
                        |this, _, cx| this.load_model_status_if_needed(true, true, cx),
                    ))
                }),
        )
    }

    fn render_model_status_content(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.model_status;
        let content = div()
            .w_full()
            .max_w(px(PAGE_MAX_WIDTH))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .pt(px(4.0));

        if self.cloud_account.credentials.is_none() {
            return content
                .child(self.render_model_status_empty(
                    theme,
                    tr!("model_status.signed_out_title"),
                    tr!("model_status.signed_out_body"),
                    Some(card_button(
                        theme,
                        SharedString::from("model-status-sign-in"),
                        tr!("model_status.sign_in"),
                        true,
                        false,
                        cx,
                        |this, _, cx| this.open_settings_page(SettingsPage::CloudAccount, cx),
                    )),
                ))
                .into_any_element();
        }
        if !state.loaded {
            return content
                .child(self.render_model_status_empty(
                    theme,
                    tr!("model_status.loading"),
                    tr!("model_status.description"),
                    None,
                ))
                .into_any_element();
        }
        if state.feature_disabled {
            return content
                .child(self.render_model_status_empty(
                    theme,
                    tr!("model_status.feature_disabled_title"),
                    tr!("model_status.feature_disabled_description"),
                    None,
                ))
                .into_any_element();
        }

        let counts = group_status::status_counts(&state.entries);
        let tile = |label: String, count: usize, color: Hsla| {
            div()
                .flex_1()
                .min_w(px(150.0))
                .px(px(16.0))
                .py(px(14.0))
                .rounded(px(14.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.raised)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(sp(12.0))
                        .text_color(theme.text_secondary)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(sp(24.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(count.to_string()),
                )
        };
        let mut content = content.child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(px(12.0))
                .child(tile(
                    tr!("model_status.total_groups"),
                    counts.total,
                    theme.text,
                ))
                .child(tile(
                    tr!("model_status.healthy_groups"),
                    counts.up,
                    theme.success,
                ))
                .child(tile(
                    tr!("model_status.degraded_groups"),
                    counts.degraded,
                    theme.warning,
                ))
                .child(tile(
                    tr!("model_status.down_groups"),
                    counts.down,
                    theme.danger,
                )),
        );

        if let Some(error) = &state.error {
            content = content.child(
                div()
                    .w_full()
                    .px(px(14.0))
                    .py(px(10.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme.warning.opacity(0.35))
                    .bg(theme.warning.opacity(0.08))
                    .text_size(sp(12.0))
                    .line_height(sp(18.0))
                    .text_color(theme.warning)
                    .child(tr!(
                        "model_status.load_failed_detail",
                        error = error.clone()
                    )),
            );
        }

        if state.entries.is_empty() {
            return content
                .child(self.render_model_status_empty(
                    theme,
                    tr!("model_status.empty_title"),
                    tr!("model_status.empty_description"),
                    None,
                ))
                .into_any_element();
        }

        let now = sub2api::auth::now_unix();
        for entry in &state.entries {
            content = content.child(self.render_model_status_card(entry, now, theme, cx));
        }
        if state
            .entries
            .iter()
            .any(GroupStatusEntry::has_fingerprint_checks)
        {
            content = content.child(self.render_benchmark_notes(theme, cx));
        }
        content.into_any_element()
    }

    fn render_model_status_empty(
        &self,
        theme: Theme,
        title: String,
        body: String,
        action: Option<Stateful<Div>>,
    ) -> impl IntoElement {
        div()
            .w_full()
            .pt(px(72.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .size(px(44.0))
                    .rounded(px(12.0))
                    .bg(theme.raised)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("icons/server.svg", 22.0, theme.text_tertiary)),
            )
            .child(
                div()
                    .text_size(sp(15.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(420.0))
                    .text_center()
                    .text_size(sp(12.5))
                    .text_color(theme.text_secondary)
                    .child(body),
            )
            .children(action)
    }

    /// The badges after a group's name: its status, then one per expected
    /// model of its fingerprint check.
    fn render_status_badges(&self, entry: &GroupStatusEntry, theme: &Theme) -> Vec<AnyElement> {
        let status = entry.status();
        let mut badges = vec![
            badge(
                if entry.waiting() {
                    tr!("model_status.waiting")
                } else {
                    status_label(status)
                },
                status_color(status, theme),
                theme,
            )
            .into_any_element(),
        ];
        if entry.summary.astra_check_enabled {
            let group_id = entry.group_id();
            for (index, state) in entry.summary.astra_check_states.iter().enumerate() {
                let checked = state
                    .checked_at
                    .as_deref()
                    .filter(|checked| !checked.trim().is_empty())
                    .map(local_time);
                let pill = badge(
                    fingerprint_badge_text(state),
                    fingerprint_color(state.status(), theme),
                    theme,
                );
                badges.push(match checked {
                    Some(checked) => div()
                        .id(SharedString::from(format!("ms-fp-{group_id}-{index}")))
                        .child(pill)
                        .tooltip(move |_, cx| {
                            StatusTooltip::build(vec![checked.clone().into()], cx)
                        })
                        .into_any_element(),
                    None => pill.into_any_element(),
                });
            }
        }
        badges
    }

    fn render_model_status_card(
        &self,
        entry: &GroupStatusEntry,
        now: i64,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let group_id = entry.group_id();
        let status = entry.status();
        let color = status_color(status, &theme);
        let (platform_label, platform_tint) = platform_badge(&entry.group.platform, &theme);
        let description = self.tx(entry.group.description.trim());
        let summary = &entry.summary;
        let observed = summary
            .observed_at
            .as_deref()
            .filter(|observed| !observed.trim().is_empty());

        // The heartbeat bar: the last 24 probes, oldest first, padded on the
        // left while a group has fewer.
        let records: Rc<[GroupStatusRecord]> = self
            .model_status
            .records
            .get(&group_id)
            .cloned()
            .unwrap_or_else(|| Rc::from(Vec::new()));
        let mut beats = div().w_full().flex().gap(px(3.0));
        for (slot, record) in group_status::heartbeat_slots(&records)
            .into_iter()
            .enumerate()
        {
            let tint = match record {
                Some(index) => {
                    let status = RuntimeStatus::from_wire(&records[index].status);
                    if status == RuntimeStatus::Unknown {
                        theme.overlay_strong
                    } else {
                        status_color(status, &theme)
                    }
                }
                None => theme.overlay_strong,
            };
            let records = records.clone();
            beats = beats.child(
                div()
                    .id(SharedString::from(format!("ms-beat-{group_id}-{slot}")))
                    .flex_1()
                    .min_w(px(4.0))
                    .h(px(24.0))
                    .rounded(px(4.0))
                    .bg(tint)
                    .tooltip(move |_, cx| {
                        let lines = match record {
                            Some(index) => record_lines(&records[index]),
                            None => vec![tr!("model_status.waiting_for_probe").into()],
                        };
                        StatusTooltip::build(lines, cx)
                    }),
            );
        }

        let latest_probe = match observed {
            Some(observed) => time_ago(observed, now).unwrap_or_else(|| local_time(observed)),
            None => tr!("model_status.waiting_for_probe"),
        };
        let updated = self.model_status.updated_at.map(|at| {
            let ago = u64::try_from(now - at).unwrap_or(0);
            format!(
                "{} ({})",
                super::sidebar::format_time_ago(ago),
                local_time_unix(at)
            )
        });

        div()
            .w_full()
            .p(px(16.0))
            .rounded(px(14.0))
            .border_1()
            .border_color(if status == RuntimeStatus::Unknown {
                theme.border
            } else {
                color.opacity(0.35)
            })
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(12.0))
            // Name, id, status and fingerprint badges.
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().size(px(10.0)).flex_none().rounded_full().bg(color))
                    .child(plaza_tag(platform_label.clone(), platform_tint, &theme))
                    .child(
                        div()
                            .text_size(sp(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(self.tx(&entry.display_name())),
                    )
                    .child(
                        div()
                            .px(px(7.0))
                            .py(px(1.0))
                            .rounded_full()
                            .bg(theme.overlay)
                            .text_size(sp(11.0))
                            .text_color(theme.text_ghost)
                            .child(format!("#{group_id}")),
                    )
                    .children(self.render_status_badges(entry, &theme)),
            )
            // Platform and description.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(sp(12.0))
                    .text_color(theme.text_ghost)
                    .child(div().flex_none().child(platform_label))
                    .child(div().flex_none().child("\u{2022}"))
                    .child(if description.is_empty() {
                        div()
                            .min_w_0()
                            .truncate()
                            .italic()
                            .child(tr!("model_status.no_description"))
                    } else {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text_secondary)
                            .child(description)
                    }),
            )
            // The heartbeat bar.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap(px(12.0))
                            .text_size(sp(11.0))
                            .text_color(theme.text_ghost)
                            .child(tr!("model_status.uptime_timeline"))
                            .child(tr!("model_status.recent_checks_hint")),
                    )
                    .child(beats),
            )
            // The latest result.
            .child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(if status == RuntimeStatus::Unknown {
                        theme.border
                    } else {
                        color.opacity(0.25)
                    })
                    .bg(if status == RuntimeStatus::Unknown {
                        theme.overlay
                    } else {
                        color.opacity(0.07)
                    })
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(sp(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_secondary)
                            .child(tr!("model_status.latest_result")),
                    )
                    .child(
                        div()
                            .text_size(sp(12.5))
                            .line_height(sp(19.0))
                            .text_color(theme.text)
                            .child(preview_text(entry)),
                    ),
            )
            // Probe metrics.
            .child(self.render_status_metrics(entry, latest_probe, &theme))
            // When, and the way into the details.
            .child(
                div()
                    .pt(px(10.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(8.0))
                            .text_size(sp(11.0))
                            .text_color(theme.text_ghost)
                            .child(observed.map_or_else(|| "\u{2014}".to_owned(), local_time))
                            .when_some(updated, |row, updated| {
                                row.child("\u{2022}")
                                    .child(tr!("model_status.last_updated", time = updated))
                            }),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("model-status-details-{group_id}")),
                        tr!("model_status.open_details"),
                        false,
                        false,
                        cx,
                        move |this, window, cx| {
                            this.open_model_status_details(group_id, window, cx)
                        },
                    )),
            )
    }

    fn render_status_metrics(
        &self,
        entry: &GroupStatusEntry,
        latest_probe: String,
        theme: &Theme,
    ) -> Div {
        let summary = &entry.summary;
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .child(metric_tile(
                tr!("model_status.latest_probe"),
                latest_probe,
                None,
                theme,
            ))
            .child(metric_tile(
                tr!("model_status.latest_latency"),
                group_status::format_latency(summary.latency_ms),
                summary.total_latency_ms.map(|total| {
                    tr!(
                        "model_status.total_latency",
                        value = group_status::format_latency(Some(total))
                    )
                }),
                theme,
            ))
            .child(metric_tile(
                tr!("model_status.availability_24h"),
                group_status::format_availability(entry.availability_24h),
                None,
                theme,
            ))
            .child(metric_tile(
                tr!("model_status.availability_7d"),
                group_status::format_availability(entry.availability_7d),
                None,
                theme,
            ))
    }

    /// Which benchmark checks which models, with links to the projects.
    fn render_benchmark_notes(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut families = div().w_full().flex().flex_wrap().gap(px(12.0));
        for family in group_status::FINGERPRINT_BENCHMARKS {
            let (platform, title) = match family.family {
                "gpt" => ("openai", tr!("model_status.benchmarks_family_gpt")),
                _ => ("anthropic", tr!("model_status.benchmarks_family_claude")),
            };
            let (_, tint) = platform_badge(platform, &theme);
            let mut sources = div().flex().flex_col().gap(px(12.0));
            for source in family.sources {
                let method = match source.method {
                    "meow" => tr!("model_status.benchmarks_method_meow"),
                    "modeltrace" => tr!("model_status.benchmarks_method_modeltrace"),
                    _ => tr!("model_status.benchmarks_method_sol_juice"),
                };
                let text = match source.key {
                    "gpt_meow" => tr!("model_status.benchmarks_source_gpt_meow"),
                    "gpt_modeltrace" => tr!("model_status.benchmarks_source_gpt_modeltrace"),
                    "gpt_juice" => tr!("model_status.benchmarks_source_gpt_juice"),
                    "claude_modeltrace" => tr!("model_status.benchmarks_source_claude_modeltrace"),
                    _ => tr!("model_status.benchmarks_source_claude_meow"),
                };
                let models = source
                    .models
                    .iter()
                    .map(|model| group_status::model_label(model))
                    .collect::<Vec<_>>()
                    .join(" / ");
                let mut links = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(14.0))
                    .gap_y(px(4.0))
                    .text_size(sp(11.0));
                if let Some(repo) = source.repo {
                    links = links.child(benchmark_link(
                        theme,
                        format!("ms-bench-repo-{}", source.key),
                        tr!(
                            "model_status.benchmarks_repo",
                            repo = group_status::repo_name(repo)
                        ),
                        repo,
                        cx,
                    ));
                }
                if let Some(site) = source.site {
                    links = links.child(benchmark_link(
                        theme,
                        format!("ms-bench-site-{}", source.key),
                        tr!("model_status.benchmarks_site"),
                        site,
                        cx,
                    ));
                }
                if let Some(license) = source.license {
                    links = links.child(
                        div()
                            .text_color(theme.text_ghost)
                            .child(tr!("model_status.benchmarks_license", license = license)),
                    );
                }
                if source.repo.is_none() {
                    links = links.child(
                        div()
                            .text_color(theme.text_ghost)
                            .child(tr!("model_status.benchmarks_self_implemented")),
                    );
                }
                sources = sources.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.0))
                                .child(badge(method, theme.text_ghost, &theme))
                                .child(
                                    div()
                                        .text_size(sp(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(models),
                                ),
                        )
                        .child(
                            div()
                                .text_size(sp(11.5))
                                .line_height(sp(17.0))
                                .text_color(theme.text_secondary)
                                .child(text),
                        )
                        .child(links),
                );
            }
            families = families.child(
                div()
                    .flex_1()
                    .min_w(px(300.0))
                    .p(px(14.0))
                    .rounded(px(12.0))
                    .bg(theme.overlay)
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(div().size(px(8.0)).rounded_full().bg(tint))
                            .child(
                                div()
                                    .text_size(sp(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child(title),
                            ),
                    )
                    .child(sources),
            );
        }

        section_box(&theme)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .child(div().pt(px(2.0)).child(icon(
                        "icons/info.svg",
                        15.0,
                        theme.text_tertiary,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .text_size(sp(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child(tr!("model_status.benchmarks_title")),
                            )
                            .child(
                                div()
                                    .text_size(sp(12.0))
                                    .line_height(sp(18.0))
                                    .text_color(theme.text_secondary)
                                    .child(tr!("model_status.benchmarks_description")),
                            ),
                    ),
            )
            .child(families)
            .child(
                div()
                    .text_size(sp(11.5))
                    .line_height(sp(17.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("model_status.benchmarks_disclaimer")),
            )
    }

    // ── Details dialog ───────────────────────────────────────────────────

    pub(super) fn render_model_status_details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = &self.model_status;
        if !state.open {
            return None;
        }
        let details = state.details.as_ref()?;
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.group_id() == details.group_id)?;
        let theme = Theme::current(cx);
        let group_id = details.group_id;
        let (platform_label, platform_tint) = platform_badge(&entry.group.platform, &theme);
        let observed = entry
            .summary
            .observed_at
            .as_deref()
            .filter(|observed| !observed.trim().is_empty());

        let mut body = div().flex().flex_col().gap(px(14.0));

        // Who, and how it stands.
        body = body.child(
            div()
                .p(px(14.0))
                .rounded(px(12.0))
                .bg(theme.raised)
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.0))
                        .child(plaza_tag(platform_label.clone(), platform_tint, &theme))
                        .child(
                            div()
                                .text_size(sp(12.0))
                                .text_color(theme.text_ghost)
                                .child(format!("{platform_label} \u{00b7} #{group_id}")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.0))
                        .children(self.render_status_badges(entry, &theme)),
                ),
        );
        body = body.child(self.render_status_metrics(
            entry,
            observed.map_or_else(|| tr!("model_status.waiting_for_probe"), local_time),
            &theme,
        ));

        if let Some(error) = &details.error {
            body = body.child(
                div()
                    .px(px(14.0))
                    .py(px(10.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme.warning.opacity(0.35))
                    .bg(theme.warning.opacity(0.08))
                    .text_size(sp(12.0))
                    .line_height(sp(18.0))
                    .text_color(theme.warning)
                    .child(tr!(
                        "model_status.detail_load_failed_detail",
                        error = error.clone()
                    )),
            );
        }

        body = body
            .child(self.render_status_history(
                details.period,
                details.loading,
                &details.history,
                theme,
                cx,
            ))
            .child(render_status_events(
                details.loading,
                &details.events,
                theme,
            ));

        // The latest result, in full.
        let color = status_color(entry.status(), &theme);
        body = body.child(
            div()
                .p(px(14.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(color.opacity(0.25))
                .bg(color.opacity(0.07))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(sp(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(tr!("model_status.latest_result")),
                )
                .child(
                    div()
                        .text_size(sp(12.5))
                        .line_height(sp(19.0))
                        .text_color(theme.text)
                        .child(preview_text(entry)),
                ),
        );

        let header = div()
            .h(px(50.0))
            .px(px(16.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .text_size(sp(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(icon("icons/server.svg", 15.0, theme.text))
                    .child(div().min_w_0().truncate().child(self.tx(&entry.display_name()))),
            )
            .child(
                div()
                    .id("model-status-details-close")
                    .tab_index(0)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .w(px(26.0))
                    .h(px(26.0))
                    .flex_none()
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_default()
                    .hover(|style| style.bg(theme.overlay))
                    .child(icon("icons/x.svg", 14.0, theme.text_secondary))
                    .on_activation(cx, |this, _, cx| this.close_model_status_details(cx)),
            );

        let mut card = div()
            .id("model-status-details-card")
            .w_full()
            .max_w(px(760.0))
            .max_h(px(720.0))
            .overflow_hidden()
            .rounded(px(18.0))
            .bg(theme.composer)
            .shadow_xl()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_model_status_details(cx);
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(
                div().flex_1().min_h(px(0.0)).relative().child(
                    div()
                        .id("model-status-details-body")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&state.details_scroll)
                        .px(px(16.0))
                        .pb(px(16.0))
                        .child(body),
                ),
            );
        if let Some(focus) = &state.details_focus {
            card = card.track_focus(focus);
        }

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        let layer = div()
            .id("model-status-details-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(scrim)
            .p(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_model_status_details(cx)),
            )
            .child(card);
        Some(gpui::deferred(layer).with_priority(4).into_any_element())
    }

    /// The availability history: a 24h/7d toggle and one bar per bucket.
    fn render_status_history(
        &self,
        period: HistoryPeriod,
        loading: bool,
        history: &[HistoryBucket],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let toggle =
            |id: &'static str, label: String, value: HistoryPeriod, cx: &mut Context<Self>| {
                let selected = period == value;
                div()
                    .id(id)
                    .tab_index(0)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .h(px(24.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .cursor_default()
                    .text_size(sp(12.0))
                    .when(selected, |button| {
                        button
                            .bg(theme.raised)
                            .shadow_sm()
                            .text_color(theme.text)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!selected, |button| button.text_color(theme.text_secondary))
                    .child(label)
                    .on_activation(cx, move |this, _, cx| {
                        this.set_model_status_period(value, cx)
                    })
            };

        let mut section = section_box(&theme).child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(sp(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(tr!("model_status.history_title")),
                        )
                        .child(
                            div()
                                .text_size(sp(12.0))
                                .text_color(theme.text_secondary)
                                .child(tr!("model_status.history_description")),
                        ),
                )
                .child(
                    div()
                        .p(px(3.0))
                        .rounded(px(8.0))
                        .bg(theme.overlay)
                        .flex()
                        .gap(px(2.0))
                        .child(toggle(
                            "model-status-period-24h",
                            tr!("model_status.period_24h"),
                            HistoryPeriod::Day,
                            cx,
                        ))
                        .child(toggle(
                            "model-status-period-7d",
                            tr!("model_status.period_7d"),
                            HistoryPeriod::Week,
                            cx,
                        )),
                ),
        );

        if loading {
            return section.child(quiet_placeholder(tr!("model_status.loading"), &theme));
        }
        if history.is_empty() {
            return section.child(quiet_placeholder(tr!("model_status.no_history"), &theme));
        }
        let mut bars = div()
            .w_full()
            .h(px(160.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .bg(theme.overlay)
            .flex()
            .items_end()
            .gap(px(2.0));
        for (index, bucket) in history.iter().enumerate() {
            let tint = status_color(bucket.bar_status(), &theme);
            let height = bucket.bar_height_percent() / 100.0;
            let lines = bucket_lines(bucket);
            bars = bars.child(
                div()
                    .id(SharedString::from(format!("ms-history-{index}")))
                    .flex_1()
                    .min_w(px(2.0))
                    .h_full()
                    .flex()
                    .items_end()
                    .child(
                        div()
                            .w_full()
                            .h(relative(height))
                            .rounded_tl(px(2.0))
                            .rounded_tr(px(2.0))
                            .bg(if bucket.bar_status() == RuntimeStatus::Unknown {
                                theme.overlay_strong
                            } else {
                                tint.opacity(0.85)
                            }),
                    )
                    .tooltip(move |_, cx| StatusTooltip::build(lines.clone(), cx)),
            );
        }
        section = section.child(bars).child(
            div()
                .flex()
                .justify_between()
                .text_size(sp(11.0))
                .text_color(theme.text_ghost)
                .child(history.first().map_or_else(
                    || "\u{2014}".to_owned(),
                    |bucket| local_time(&bucket.bucket_start),
                ))
                .child(history.last().map_or_else(
                    || "\u{2014}".to_owned(),
                    |bucket| local_time(&bucket.bucket_end),
                )),
        );
        section
    }
}

/// The recent stable-state events, newest first as the service sends them.
fn render_status_events(
    loading: bool,
    events: &[GroupStatusEvent],
    theme: Theme,
) -> impl IntoElement {
    let mut section = section_box(&theme).child(
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(sp(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(tr!("model_status.events_title")),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_secondary)
                    .child(tr!("model_status.events_description")),
            ),
    );
    if loading {
        return section.child(quiet_placeholder(tr!("model_status.loading"), &theme));
    }
    if events.is_empty() {
        return section.child(quiet_placeholder(tr!("model_status.no_events"), &theme));
    }
    let now = sub2api::auth::now_unix();
    for event in events {
        let tone = match group_status::event_tone(&event.event_type) {
            EventTone::Good => theme.success,
            EventTone::Bad => theme.danger,
            EventTone::Neutral => theme.text_ghost,
        };
        let (from_label, from_tint) = event_state(event, &event.from_status, &theme);
        let (to_label, to_tint) = event_state(event, &event.to_status, &theme);
        let mut meta = vec![
            tr!(
                "model_status.latency_line",
                value = group_status::format_latency(event.latency_ms)
            ),
            tr!(
                "model_status.http_code",
                code = event
                    .http_code
                    .map_or_else(|| "-".to_owned(), |code| code.to_string())
            ),
        ];
        if !event.sub_status.is_empty() {
            meta.push(tr!(
                "model_status.sub_status",
                value = event.sub_status.clone()
            ));
        }
        let error = group_status::sanitize_error_detail(&event.error_detail);
        section = section.child(
            div()
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.0))
                                .child(badge(event_type_label(&event.event_type), tone, &theme))
                                .when_some(fingerprint_event_text(event), |row, text| {
                                    row.child(
                                        div()
                                            .text_size(sp(11.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.text)
                                            .child(text),
                                    )
                                })
                                .when_some(time_ago(&event.observed_at, now), |row, ago| {
                                    row.child(
                                        div()
                                            .text_size(sp(11.5))
                                            .text_color(theme.text_ghost)
                                            .child(ago),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(sp(11.0))
                                .text_color(theme.text_ghost)
                                .child(local_time(&event.observed_at)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(badge(from_label, from_tint, &theme))
                        .child(icon("icons/arrow-right.svg", 11.0, theme.text_ghost))
                        .child(badge(to_label, to_tint, &theme)),
                )
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(meta.join(" \u{00b7} ")),
                )
                .when(!error.is_empty(), |card| {
                    card.child(
                        div()
                            .px(px(10.0))
                            .py(px(7.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme.danger.opacity(0.3))
                            .bg(theme.danger.opacity(0.08))
                            .text_size(sp(11.5))
                            .line_height(sp(17.0))
                            .text_color(theme.danger)
                            .child(error),
                    )
                }),
        );
    }
    section
}

/// "Nothing here" inside a section: a dashed box with one quiet line.
fn quiet_placeholder(text: String, theme: &Theme) -> Div {
    div()
        .w_full()
        .py(px(28.0))
        .rounded(px(10.0))
        .border_1()
        .border_dashed()
        .border_color(theme.border_strong)
        .flex()
        .justify_center()
        .text_size(sp(12.0))
        .text_color(theme.text_ghost)
        .child(text)
}

/// A link under a benchmark: opens the project in the browser.
fn benchmark_link(
    theme: Theme,
    id: String,
    label: String,
    url: &'static str,
    cx: &mut Context<Waku>,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .flex()
        .items_center()
        .gap(px(4.0))
        .cursor_pointer()
        .text_color(theme.accent)
        .hover(|style| style.underline())
        .child(icon("icons/external-link.svg", 11.0, theme.accent))
        .child(label)
        .on_activation(cx, move |_, _, cx| cx.open_url(url))
}
