//! The welcome screen's activity overview: how much, when, and with what.
//!
//! Fork addition, after dsh-claude-style's home panel (MIT, © Nwflower). That
//! panel folds token usage out of dsh's session logs; here the source is this
//! app's own history, which records every message of every provider but no
//! token counts. So the overview counts *messages* — what the person sent and
//! what the agents answered — and never tokens.
//!
//! The daemon reads the history into compact [`ActivityRecords`]
//! (`Command::LoadActivityOverview`, `waku_core::activity_overview::load`):
//! messages per local day, hour and model, and sessions per local day and
//! model. Everything the panel shows — each range's tiles, the model ranking,
//! the heat grid — is a cheap pure fold over those records ([`summarize`],
//! [`heat_grid`]), so switching a range never goes back to the daemon.
//!
//! The history is a ledger the daemon keeps beside the tasks, so deleting a
//! task or rewinding a conversation never takes activity away, and the
//! messages a fork copies over are not counted a second time.
//!
//! A message is attributed to its session's *current* model — the last one
//! known, once the session is deleted: the history keeps no per-message
//! model, and a session's model rarely changes.

use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Messages that share a local day, hour and model.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageBucket {
    pub day: NaiveDate,
    pub hour: u8,
    /// Index into [`ActivityRecords::models`].
    pub model: u32,
    pub messages: u32,
}

/// A session that holds at least one finished message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    /// Local day the session was created.
    pub day: NaiveDate,
    pub model: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRecords {
    /// Sorted by day, then hour, then model.
    pub buckets: Vec<MessageBucket>,
    pub sessions: Vec<SessionRecord>,
    /// Model ids, interned; `""` stands for a session that never named one.
    pub models: Vec<String>,
}

impl ActivityRecords {
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

/// Which stretch of history the tiles and the model ranking cover.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ActivityRange {
    #[default]
    All,
    Days30,
    Days7,
}

impl ActivityRange {
    /// The first local day inside the range (today counts as one).
    pub fn start(self, today: NaiveDate) -> Option<NaiveDate> {
        let days = match self {
            ActivityRange::All => return None,
            ActivityRange::Days30 => 29,
            ActivityRange::Days7 => 6,
        };
        today.checked_sub_days(Days::new(days))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelShare {
    /// `""` when the sessions never named a model.
    pub model: String,
    pub messages: u64,
    pub sessions: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivitySummary {
    pub sessions: u32,
    pub messages: u64,
    pub active_days: u32,
    /// The local hour with the most messages; ties go to the earlier hour.
    pub peak_hour: Option<u8>,
    /// The named model with the most sessions; ties go to the one with more
    /// messages, then to the name.
    pub favorite_model: Option<String>,
    /// The most consecutive active days inside the range.
    pub longest_streak: u32,
    /// Models by messages, most first.
    pub models: Vec<ModelShare>,
}

pub fn summarize(
    records: &ActivityRecords,
    range: ActivityRange,
    today: NaiveDate,
) -> ActivitySummary {
    let start = range.start(today);
    let inside = |day: NaiveDate| start.is_none_or(|start| day >= start) && day <= today;
    let model_count = records.models.len();
    let slot = |model: u32| (model as usize).min(model_count.saturating_sub(1));

    let mut summary = ActivitySummary::default();
    let mut hours = [0_u64; 24];
    let mut days: Vec<NaiveDate> = Vec::new();
    let mut model_messages = vec![0_u64; model_count];
    let mut model_sessions = vec![0_u32; model_count];
    for bucket in records.buckets.iter().filter(|bucket| inside(bucket.day)) {
        let messages = u64::from(bucket.messages);
        summary.messages += messages;
        hours[usize::from(bucket.hour.min(23))] += messages;
        if days.last() != Some(&bucket.day) {
            days.push(bucket.day);
        }
        if model_count > 0 {
            model_messages[slot(bucket.model)] += messages;
        }
    }
    for session in records.sessions.iter().filter(|session| inside(session.day)) {
        summary.sessions += 1;
        if model_count > 0 {
            model_sessions[slot(session.model)] += 1;
        }
    }

    // Buckets are sorted by day, so `days` is ascending; dedup guards a
    // producer that did not sort.
    days.sort_unstable();
    days.dedup();
    summary.active_days = days.len() as u32;
    summary.longest_streak = longest_streak(&days);
    summary.peak_hour = hours
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .max_by(|(left_hour, left), (right_hour, right)| {
            left.cmp(right).then(right_hour.cmp(left_hour))
        })
        .map(|(hour, _)| hour as u8);

    let mut models = records
        .models
        .iter()
        .enumerate()
        .filter(|(index, _)| model_messages[*index] > 0 || model_sessions[*index] > 0)
        .map(|(index, model)| ModelShare {
            model: model.clone(),
            messages: model_messages[index],
            sessions: model_sessions[index],
        })
        .collect::<Vec<_>>();
    summary.favorite_model = models
        .iter()
        .filter(|share| !share.model.is_empty() && share.sessions > 0)
        .max_by(|left, right| {
            left.sessions
                .cmp(&right.sessions)
                .then(left.messages.cmp(&right.messages))
                .then(right.model.cmp(&left.model))
        })
        .map(|share| share.model.clone());
    models.sort_by(|left, right| {
        right
            .messages
            .cmp(&left.messages)
            .then(right.sessions.cmp(&left.sessions))
            .then(left.model.cmp(&right.model))
    });
    summary.models = models;
    summary
}

fn longest_streak(days: &[NaiveDate]) -> u32 {
    let mut longest = 0;
    let mut current = 0;
    let mut previous: Option<NaiveDate> = None;
    for &day in days {
        current = match previous {
            Some(previous) if previous.succ_opt() == Some(day) => current + 1,
            _ => 1,
        };
        longest = longest.max(current);
        previous = Some(day);
    }
    longest
}

/// Messages per day for the last `weeks` weeks, one column per week starting
/// on Sunday, the last column ending with today.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeatGrid {
    /// The Sunday the first column starts on.
    pub start: NaiveDate,
    pub weeks: usize,
    /// Column-major: `cells[week * 7 + weekday]`, Sunday = 0. `None` for the
    /// days after today in the last column.
    pub cells: Vec<Option<u32>>,
    /// The busiest day in the grid.
    pub peak: u32,
}

impl HeatGrid {
    pub fn day(&self, week: usize, weekday: usize) -> NaiveDate {
        self.start + Days::new((week * 7 + weekday) as u64)
    }
}

pub fn heat_grid(records: &ActivityRecords, today: NaiveDate, weeks: usize) -> HeatGrid {
    let weeks = weeks.max(1);
    let this_sunday = today - Days::new(u64::from(today.weekday().num_days_from_sunday()));
    let start = this_sunday - Days::new(7 * (weeks as u64 - 1));
    let mut cells = vec![Some(0_u32); weeks * 7];
    for (index, cell) in cells.iter_mut().enumerate() {
        if start + Days::new(index as u64) > today {
            *cell = None;
        }
    }
    for bucket in &records.buckets {
        if bucket.day < start || bucket.day > today {
            continue;
        }
        let index = (bucket.day - start).num_days() as usize;
        if let Some(Some(count)) = cells.get_mut(index) {
            *count += bucket.messages;
        }
    }
    let peak = cells.iter().flatten().copied().max().unwrap_or(0);
    HeatGrid {
        start,
        weeks,
        cells,
        peak,
    }
}

/// 0 for an empty day, then 1–4 by the share of the busiest day.
pub fn heat_level(count: u32, peak: u32) -> u8 {
    if count == 0 || peak == 0 {
        return 0;
    }
    (u64::from(count) * 4)
        .div_ceil(u64::from(peak))
        .clamp(1, 4) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn bucket(day: NaiveDate, hour: u8, model: u32, messages: u32) -> MessageBucket {
        MessageBucket {
            day,
            hour,
            model,
            messages,
        }
    }

    fn records() -> ActivityRecords {
        ActivityRecords {
            models: vec!["".into(), "claude-opus-5-5".into(), "gpt-6-sol".into()],
            buckets: vec![
                bucket(day(2026, 3, 1), 9, 2, 40),
                bucket(day(2026, 9, 30), 15, 1, 6),
                bucket(day(2026, 10, 1), 15, 1, 4),
                bucket(day(2026, 10, 2), 9, 0, 3),
                bucket(day(2026, 10, 4), 15, 2, 2),
            ],
            sessions: vec![
                SessionRecord { day: day(2026, 3, 1), model: 2 },
                SessionRecord { day: day(2026, 9, 30), model: 1 },
                SessionRecord { day: day(2026, 10, 1), model: 1 },
                SessionRecord { day: day(2026, 10, 2), model: 0 },
                SessionRecord { day: day(2026, 10, 4), model: 2 },
            ],
        }
    }

    #[test]
    fn summary_covers_the_range_only() {
        let today = day(2026, 10, 4);
        let all = summarize(&records(), ActivityRange::All, today);
        assert_eq!(all.sessions, 5);
        assert_eq!(all.messages, 55);
        assert_eq!(all.active_days, 5);
        assert_eq!(all.peak_hour, Some(9));
        assert_eq!(all.longest_streak, 3);
        // Two sessions each; gpt-6-sol has more messages.
        assert_eq!(all.favorite_model.as_deref(), Some("gpt-6-sol"));
        assert_eq!(all.models[0].model, "gpt-6-sol");

        let week = summarize(&records(), ActivityRange::Days7, today);
        assert_eq!(week.sessions, 4);
        assert_eq!(week.messages, 15);
        assert_eq!(week.peak_hour, Some(15));
        assert_eq!(week.favorite_model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(week.longest_streak, 3);
    }

    #[test]
    fn an_unnamed_model_is_never_the_favorite() {
        let today = day(2026, 10, 4);
        let records = ActivityRecords {
            models: vec!["".into()],
            buckets: vec![bucket(today, 1, 0, 9)],
            sessions: vec![SessionRecord { day: today, model: 0 }],
        };
        assert_eq!(
            summarize(&records, ActivityRange::All, today).favorite_model,
            None
        );
    }

    #[test]
    fn peak_hour_ties_go_to_the_earlier_hour() {
        let today = day(2026, 10, 4);
        let records = ActivityRecords {
            models: vec!["m".into()],
            buckets: vec![bucket(today, 8, 0, 5), bucket(today, 20, 0, 5)],
            sessions: vec![],
        };
        assert_eq!(
            summarize(&records, ActivityRange::All, today).peak_hour,
            Some(8)
        );
    }

    #[test]
    fn a_malformed_model_index_does_not_panic() {
        let today = day(2026, 10, 4);
        let records = ActivityRecords {
            models: vec!["m".into()],
            buckets: vec![bucket(today, 30, 7, 1)],
            sessions: vec![SessionRecord { day: today, model: 9 }],
        };
        let summary = summarize(&records, ActivityRange::All, today);
        assert_eq!(summary.messages, 1);
        assert_eq!(summary.peak_hour, Some(23));
    }

    #[test]
    fn heat_grid_ends_today_and_starts_on_a_sunday() {
        // 2026-10-04 is a Sunday.
        let today = day(2026, 10, 4);
        let grid = heat_grid(&records(), today, 26);
        assert_eq!(grid.start.weekday().num_days_from_sunday(), 0);
        assert_eq!(grid.cells.len(), 26 * 7);
        assert_eq!(grid.day(25, 0), today);
        assert_eq!(grid.cells[25 * 7], Some(2));
        assert!(grid.cells[25 * 7 + 1..].iter().all(Option::is_none));
        // March 1 is outside 26 weeks; Sept 30 is the busiest day inside.
        assert_eq!(grid.peak, 6);
    }

    #[test]
    fn heat_levels_scale_to_the_busiest_day() {
        assert_eq!(heat_level(0, 10), 0);
        assert_eq!(heat_level(1, 10), 1);
        assert_eq!(heat_level(3, 10), 2);
        assert_eq!(heat_level(10, 10), 4);
        assert_eq!(heat_level(5, 0), 0);
    }

    #[test]
    fn records_travel_as_json() {
        let json = serde_json::to_string(&records()).unwrap();
        assert!(json.contains("\"day\":\"2026-09-30\""));
        let back: ActivityRecords = serde_json::from_str(&json).unwrap();
        assert_eq!(back.buckets, records().buckets);
    }
}
