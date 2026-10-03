//! The model status page: how the gateway's groups are running, as the web
//! console's `/model-status` page shows it — four summary figures, a card
//! per monitored group with its last 24 probes, latency, availability and
//! fingerprint badges, and a details dialog with the history and the
//! stable-state events.
//!
//! Fork addition, opened from the sidebar row under "Images". This file is
//! the state, the loading and the 30-second refresh; the view is
//! `model_status_view.rs` and the requests are `sub2api::group_status`.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use sub2api::group_status::{
    self, GroupStatusEntry, GroupStatusEvent, GroupStatusRecord, HistoryBucket, HistoryPeriod,
};

use super::*;

/// The web page's refresh cadence.
const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Reopening the page within this reuses what is on screen.
const LIST_TTL: Duration = Duration::from_secs(30);

pub(super) struct ModelStatusState {
    /// The page holds the main column.
    pub(super) open: bool,
    pub(super) entries: Vec<GroupStatusEntry>,
    /// Each group's last probes, oldest first. Shared with the heartbeat
    /// cells' tooltips, which format them only on hover.
    pub(super) records: HashMap<i64, Rc<[GroupStatusRecord]>>,
    /// A first answer — data, refusal or failure — came back for this
    /// account; until then the page says it is loading.
    pub(super) loaded: bool,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    /// The administrator has not turned the page on.
    pub(super) feature_disabled: bool,
    /// Unix seconds of the last successful refresh.
    pub(super) updated_at: Option<i64>,
    attempted_at: Option<Instant>,
    /// Render schedules loads; this keeps it from scheduling twice per frame.
    load_scheduled: Cell<bool>,
    /// A poll loop lives only while its epoch is current.
    poll_epoch: u64,
    polling: bool,
    pub(super) details: Option<ModelStatusDetails>,
    details_request: u64,
    pub(super) details_focus: Option<FocusHandle>,
    pub(super) scroll: ScrollHandle,
    pub(super) scrollbar: Rc<ScrollbarState>,
    pub(super) details_scroll: ScrollHandle,
}

impl Default for ModelStatusState {
    fn default() -> Self {
        Self {
            open: false,
            entries: Vec::new(),
            records: HashMap::new(),
            loaded: false,
            loading: false,
            error: None,
            feature_disabled: false,
            updated_at: None,
            attempted_at: None,
            load_scheduled: Cell::new(false),
            poll_epoch: 0,
            polling: false,
            details: None,
            details_request: 0,
            details_focus: None,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
            details_scroll: ScrollHandle::new(),
        }
    }
}

/// The details dialog of one group.
pub(super) struct ModelStatusDetails {
    pub(super) group_id: i64,
    pub(super) period: HistoryPeriod,
    pub(super) history: Vec<HistoryBucket>,
    pub(super) events: Vec<GroupStatusEvent>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    /// Which load the dialog waits for, so a slow 7-day answer cannot land
    /// over a newer 24-hour one.
    request: u64,
}

impl Waku {
    // ── Main pages ───────────────────────────────────────────────────────

    /// A fork page (the image studio, the model status) holds the main
    /// column instead of a task.
    pub(super) fn main_page_open(&self) -> bool {
        self.image_studio.open || self.model_status.open
    }

    /// Leave whichever fork page holds the main column: going to a task, or
    /// starting one.
    pub(super) fn close_main_pages(&mut self) {
        self.image_studio.open = false;
        self.close_model_status();
    }

    /// The header's title while a fork page is open.
    pub(super) fn main_page_title(&self) -> Option<String> {
        if self.image_studio.open {
            Some(self.image_studio_title())
        } else if self.model_status.open {
            Some(tr!("model_status.title"))
        } else {
            None
        }
    }

    // ── Opening ──────────────────────────────────────────────────────────

    pub(super) fn open_model_status(&mut self, cx: &mut Context<Self>) {
        self.settings_page = None;
        self.image_studio.open = false;
        self.model_status.open = true;
        self.load_model_status_if_needed(false, false, cx);
        self.start_model_status_polling(cx);
        cx.notify();
    }

    /// Close the page and stop its refresh. What it showed is kept, so
    /// coming back draws at once.
    pub(super) fn close_model_status(&mut self) {
        let state = &mut self.model_status;
        state.open = false;
        state.details = None;
        state.polling = false;
        state.poll_epoch = state.poll_epoch.wrapping_add(1);
    }

    /// Forget what the page showed along with the account it came from.
    /// A load in flight belonged to that account and is dropped on arrival.
    pub(super) fn clear_model_status(&mut self) {
        let state = &mut self.model_status;
        state.entries.clear();
        state.records.clear();
        state.loaded = false;
        state.loading = false;
        state.error = None;
        state.feature_disabled = false;
        state.updated_at = None;
        state.attempted_at = None;
        state.load_scheduled.set(false);
        state.details = None;
    }

    /// Render's hook: load once there is an account and nothing on screen
    /// yet — the page may have opened before sign-in finished.
    pub(super) fn schedule_model_status_load(&self, cx: &mut Context<Self>) {
        let state = &self.model_status;
        if self.cloud_account.credentials.is_none() || state.loading || state.loaded {
            return;
        }
        if state.load_scheduled.replace(true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.load_model_status_if_needed(false, false, cx);
            });
        })
        .detach();
    }

    // ── Loading ──────────────────────────────────────────────────────────

    /// Fetch the groups and their last probes. `force` skips the freshness
    /// check (refresh, polling); `manual` is the refresh button, which says
    /// so when it fails.
    pub(super) fn load_model_status_if_needed(
        &mut self,
        force: bool,
        manual: bool,
        cx: &mut Context<Self>,
    ) {
        self.model_status.load_scheduled.set(false);
        if self.model_status.loading {
            return;
        }
        if !force
            && self
                .model_status
                .attempted_at
                .is_some_and(|at| at.elapsed() < LIST_TTL)
        {
            return;
        }
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        self.model_status.loading = true;
        cx.notify();

        let session = credentials.session_id.clone();
        cx.spawn(async move |this, cx| {
            let (credentials, fetched) = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let fetched = sub2api::authenticated(&mut credentials).and_then(|client| {
                        group_status::load_board(&client, &credentials.access_token)
                    });
                    (credentials, fetched)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.cloud_session_is(&session) {
                    return;
                }
                if let Err(error) = &fetched
                    && sub2api::session_ended(error)
                {
                    this.model_status.loading = false;
                    this.end_cloud_session(cx);
                    return;
                }
                this.adopt_cloud_tokens(credentials);
                let state = &mut this.model_status;
                state.loading = false;
                state.loaded = true;
                state.attempted_at = Some(Instant::now());
                let mut toast = None;
                match fetched {
                    Ok(board) => {
                        state.records = board
                            .records
                            .into_iter()
                            .map(|(group_id, records)| (group_id, Rc::from(records)))
                            .collect();
                        state.entries = board.entries;
                        state.error = None;
                        state.feature_disabled = false;
                        state.updated_at = Some(sub2api::auth::now_unix());
                        let entries = &state.entries;
                        if state.details.as_ref().is_some_and(|details| {
                            !entries
                                .iter()
                                .any(|entry| entry.group_id() == details.group_id)
                        }) {
                            state.details = None;
                        }
                    }
                    Err(error) if group_status::feature_disabled(&error) => {
                        state.feature_disabled = true;
                        state.entries.clear();
                        state.records.clear();
                        state.details = None;
                        state.error = None;
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        if manual {
                            toast = Some(tr!(
                                "model_status.load_failed_toast",
                                error = message.clone()
                            ));
                        }
                        state.error = Some(message);
                    }
                }
                if let Some(toast) = toast {
                    this.show_toast(toast);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Refresh every 30 seconds while the page is open. The loop ends when
    /// the page closes (its epoch moves on); a settings page covering it
    /// only skips the round.
    fn start_model_status_polling(&mut self, cx: &mut Context<Self>) {
        if self.model_status.polling {
            return;
        }
        self.model_status.polling = true;
        let epoch = self.model_status.poll_epoch;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = this
                    .update(cx, |this, cx| {
                        let state = &this.model_status;
                        if !state.open || state.poll_epoch != epoch {
                            return false;
                        }
                        if this.settings_page.is_none() {
                            this.load_model_status_if_needed(true, false, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    // ── Details ──────────────────────────────────────────────────────────

    pub(super) fn open_model_status_details(
        &mut self,
        group_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if group_id <= 0 {
            self.show_toast(tr!("model_status.detail_load_failed"));
            return;
        }
        let focus = self
            .model_status
            .details_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        self.model_status.details = Some(ModelStatusDetails {
            group_id,
            period: HistoryPeriod::Day,
            history: Vec::new(),
            events: Vec::new(),
            loading: false,
            error: None,
            request: 0,
        });
        self.model_status
            .details_scroll
            .set_offset(gpui::Point::default());
        // The dialog sits on GPUI's deferred plane; focus it once it has
        // joined the dispatch tree, so the first Escape already closes it.
        let weak = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                let still_open = weak
                    .update(cx, |this, _| this.model_status.details.is_some())
                    .unwrap_or(false);
                if still_open {
                    window.focus(&focus, cx);
                }
            });
        });
        self.load_model_status_details(cx);
    }

    pub(super) fn set_model_status_period(
        &mut self,
        period: HistoryPeriod,
        cx: &mut Context<Self>,
    ) {
        let Some(details) = self.model_status.details.as_mut() else {
            return;
        };
        if details.period == period {
            return;
        }
        details.period = period;
        self.load_model_status_details(cx);
    }

    pub(super) fn close_model_status_details(&mut self, cx: &mut Context<Self>) {
        if self.model_status.details.take().is_some() {
            cx.notify();
        }
    }

    fn load_model_status_details(&mut self, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        self.model_status.details_request = self.model_status.details_request.wrapping_add(1);
        let request = self.model_status.details_request;
        let Some(details) = self.model_status.details.as_mut() else {
            return;
        };
        details.request = request;
        details.loading = true;
        details.error = None;
        let group_id = details.group_id;
        let period = details.period;
        cx.notify();

        let session = credentials.session_id.clone();
        cx.spawn(async move |this, cx| {
            let (credentials, fetched) = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    let fetched = sub2api::authenticated(&mut credentials).and_then(|client| {
                        group_status::load_details(
                            &client,
                            &credentials.access_token,
                            group_id,
                            period,
                        )
                    });
                    (credentials, fetched)
                })
                .await;
            let _ =
                this.update(cx, |this, cx| {
                    if !this.cloud_session_is(&session) {
                        return;
                    }
                    if let Err(error) = &fetched
                        && sub2api::session_ended(error)
                    {
                        this.end_cloud_session(cx);
                        return;
                    }
                    this.adopt_cloud_tokens(credentials);
                    let Some(details) = this.model_status.details.as_mut().filter(|details| {
                        details.group_id == group_id && details.request == request
                    }) else {
                        return;
                    };
                    details.loading = false;
                    match fetched {
                        Ok((history, events)) => {
                            details.history = history;
                            details.events = events;
                        }
                        Err(error) => {
                            details.history.clear();
                            details.events.clear();
                            details.error = Some(format!("{error:#}"));
                        }
                    }
                    cx.notify();
                });
        })
        .detach();
    }
}

/// "5 分钟前" for an RFC 3339 instant, measured from `now`. `None` for a
/// timestamp that does not parse or lies ahead — the web page says
/// nothing useful for those either.
pub(super) fn time_ago(raw: &str, now: i64) -> Option<String> {
    let instant = chrono::DateTime::parse_from_rfc3339(raw.trim()).ok()?;
    let seconds = now.checked_sub(instant.timestamp())?;
    let seconds = u64::try_from(seconds).ok()?;
    Some(super::sidebar::format_time_ago(seconds))
}

/// An RFC 3339 instant in local time: `2026-10-03 12:00:05`. The raw text
/// when it does not parse.
pub(super) fn local_time(raw: &str) -> String {
    local_time_in(raw, &Local)
}

fn local_time_in<Tz: chrono::TimeZone>(raw: &str, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match chrono::DateTime::parse_from_rfc3339(raw.trim()) {
        Ok(instant) => instant
            .with_timezone(zone)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => raw.trim().to_owned(),
    }
}

/// Unix seconds in local time, for the "last refreshed" line.
pub(super) fn local_time_unix(seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|instant| {
            instant
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_timestamps_read_as_relative_time() {
        let now = 1_791_000_000;
        let instant = chrono::DateTime::<Utc>::from_timestamp(now - 125, 0)
            .unwrap()
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y-%m-%dT%H:%M:%S.123456789%:z")
            .to_string();
        assert_eq!(
            time_ago(&instant, now),
            Some(super::super::sidebar::format_time_ago(125))
        );
        assert_eq!(time_ago("not a time", now), None);
        // A clock running behind the server's is no "-5 minutes ago".
        let ahead = chrono::DateTime::<Utc>::from_timestamp(now + 300, 0)
            .unwrap()
            .to_rfc3339();
        assert_eq!(time_ago(&ahead, now), None);
    }

    #[test]
    fn local_times_convert_the_zone() {
        assert_eq!(
            local_time_in(
                "2026-10-03T04:00:05.5Z",
                &chrono::FixedOffset::east_opt(8 * 3600).unwrap()
            ),
            "2026-10-03 12:00:05"
        );
        assert_eq!(local_time_in(" junk ", &Utc), "junk");
    }
}
