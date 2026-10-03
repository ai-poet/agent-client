//! The gateway's group runtime status, for the "Model status" page.
//!
//! A port of the web console's `/model-status` page data layer
//! (`frontend/src/api/groupStatus.ts` and `frontend/src/utils/groupStatus.ts`):
//! the list of monitored groups with their latest probe, the last probes of
//! each group, its availability history, and its stable-state events. All
//! four endpoints take the user's session token.
//!
//! [`crate::client::GroupStatusItem`] stays the thin view failover reads;
//! this module holds everything the page shows.

use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;

use crate::client::{Client, null_to_default};

/// How many probes each group's heartbeat bar shows.
pub const HEARTBEAT_RECORDS: usize = 24;
/// How many events the details dialog lists.
pub const EVENT_LIMIT: usize = 20;
/// How much of a probe's answer or error the cards show.
pub const PREVIEW_CHARS: usize = 180;
/// The refusal reason when the administrator has not turned the page on.
pub const FEATURE_DISABLED_REASON: &str = "GROUP_STATUS_FEATURE_DISABLED";
/// The fingerprint candidate meaning "a model outside the benchmark".
pub const ASTRA_OTHER_MODEL: &str = "other_known_external";

/// Parallel record fetches while loading the board.
const RECORD_WORKERS: usize = 4;

/// The group a status belongs to. The gateway serializes it without json
/// tags, so its keys arrive PascalCase; the snake form is accepted too.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GroupInfo {
    #[serde(default, alias = "ID")]
    pub id: i64,
    #[serde(default, alias = "Name", deserialize_with = "null_to_default")]
    pub name: String,
    #[serde(default, alias = "Description", deserialize_with = "null_to_default")]
    pub description: String,
    #[serde(default, alias = "Platform", deserialize_with = "null_to_default")]
    pub platform: String,
}

impl GroupInfo {
    /// The group's name, or `#id` when it has none.
    pub fn display_name(&self, fallback_id: i64) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            format!("#{fallback_id}")
        } else {
            name.to_owned()
        }
    }
}

/// One expected model's fingerprint check on a group.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AstraCheckState {
    #[serde(default, deserialize_with = "null_to_default")]
    pub expected_model: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub display_name: String,
    /// `meow`, `modeltrace` or `sol_juice`.
    #[serde(default, deserialize_with = "null_to_default")]
    pub method: String,
    /// The latest run: `match`, `mismatch` or `insufficient`.
    #[serde(default, deserialize_with = "null_to_default")]
    pub verdict: String,
    /// The confirmed state: `pass` or `mismatch`.
    #[serde(default, deserialize_with = "null_to_default")]
    pub stable_status: String,
    /// The model a mismatch points at.
    #[serde(default, deserialize_with = "null_to_default")]
    pub winner: String,
    #[serde(default)]
    pub checked_at: Option<String>,
}

impl AstraCheckState {
    /// The badge state, as the web page's `normalizeAstraStateStatus`: the
    /// Juice reading keeps a confirmed verdict until another is confirmed,
    /// the other methods show their latest run.
    pub fn status(&self) -> FingerprintStatus {
        if self.method == "sol_juice" {
            match self.stable_status.as_str() {
                "pass" => return FingerprintStatus::Pass,
                "mismatch" => return FingerprintStatus::Mismatch,
                _ => {}
            }
        }
        fingerprint_status(&self.stable_status, &self.verdict)
    }
}

/// A group's latest probe and its fingerprint checks.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GroupStatusSummary {
    #[serde(default)]
    pub group_id: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub latest_status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub stable_status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub response_excerpt: String,
    /// Time to first token.
    #[serde(default)]
    pub latency_ms: Option<f64>,
    #[serde(default)]
    pub total_latency_ms: Option<f64>,
    #[serde(default)]
    pub http_code: Option<i64>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub sub_status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub error_detail: String,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub astra_check_enabled: bool,
    #[serde(default, deserialize_with = "null_to_default")]
    pub astra_check_states: Vec<AstraCheckState>,
}

/// One monitored group, as `GET /group-status` lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GroupStatusEntry {
    #[serde(default, deserialize_with = "null_to_default")]
    pub group: GroupInfo,
    #[serde(default, deserialize_with = "null_to_default")]
    pub summary: GroupStatusSummary,
    #[serde(default)]
    pub availability_24h: Option<f64>,
    #[serde(default)]
    pub availability_7d: Option<f64>,
}

/// What a card's "latest result" box says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Text(String),
    WaitingForProbe,
    NotAvailable,
}

impl GroupStatusEntry {
    pub fn group_id(&self) -> i64 {
        if self.group.id > 0 {
            self.group.id
        } else {
            self.summary.group_id
        }
    }

    pub fn display_name(&self) -> String {
        self.group.display_name(self.group_id())
    }

    /// The smoothed status, falling back to the latest probe.
    pub fn status(&self) -> RuntimeStatus {
        if !self.summary.stable_status.is_empty() {
            RuntimeStatus::from_wire(&self.summary.stable_status)
        } else {
            RuntimeStatus::from_wire(&self.summary.latest_status)
        }
    }

    /// No probe has run yet: the badge says "waiting" instead of a status.
    pub fn waiting(&self) -> bool {
        self.summary
            .observed_at
            .as_deref()
            .is_none_or(|observed| observed.trim().is_empty())
    }

    /// The latest error (with upstream addresses removed), else the answer's
    /// excerpt, else why there is nothing to show.
    pub fn preview(&self) -> Preview {
        let summary = &self.summary;
        if !summary.error_detail.trim().is_empty() {
            return Preview::Text(shorten_excerpt(
                &sanitize_error_detail(&summary.error_detail),
                PREVIEW_CHARS,
            ));
        }
        if !summary.response_excerpt.trim().is_empty() {
            return Preview::Text(shorten_excerpt(&summary.response_excerpt, PREVIEW_CHARS));
        }
        if self.waiting() {
            Preview::WaitingForProbe
        } else {
            Preview::NotAvailable
        }
    }

    pub fn has_fingerprint_checks(&self) -> bool {
        self.summary.astra_check_enabled
    }
}

/// One probe, for the heartbeat bar.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GroupStatusRecord {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub status: String,
    #[serde(default)]
    pub latency_ms: Option<f64>,
    #[serde(default)]
    pub http_code: Option<i64>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub sub_status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub observed_at: String,
}

/// A stable-state change: an outage, a recovery, a fingerprint verdict.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GroupStatusEvent {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub event_type: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub from_status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub to_status: String,
    #[serde(default)]
    pub latency_ms: Option<f64>,
    #[serde(default)]
    pub http_code: Option<i64>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub sub_status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub error_detail: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub observed_at: String,
}

/// One bar of the availability history: an hour (24h) or a day (7d).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct HistoryBucket {
    #[serde(default, deserialize_with = "null_to_default")]
    pub bucket_start: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub bucket_end: String,
    /// Percent.
    #[serde(default)]
    pub availability: f64,
    #[serde(default)]
    pub avg_latency_ms: Option<f64>,
    #[serde(default)]
    pub total_count: i64,
    #[serde(default)]
    pub down_count: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub latest_status: String,
}

impl HistoryBucket {
    /// The bar's colour: its latest status, else down if any probe was.
    pub fn bar_status(&self) -> RuntimeStatus {
        if !self.latest_status.is_empty() {
            RuntimeStatus::from_wire(&self.latest_status)
        } else if self.down_count > 0 {
            RuntimeStatus::Down
        } else {
            RuntimeStatus::Up
        }
    }

    /// The bar's height in percent: a stub for an empty bucket, at least a
    /// sliver otherwise so a bad hour is still visible.
    pub fn bar_height_percent(&self) -> f32 {
        if self.total_count == 0 {
            return 8.0;
        }
        js_round(self.availability).clamp(10.0, 100.0) as f32
    }
}

/// The history range the details dialog shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryPeriod {
    #[default]
    Day,
    Week,
}

impl HistoryPeriod {
    pub fn query(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
        }
    }
}

/// A group's runtime status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeStatus {
    Up,
    Degraded,
    Down,
    Unknown,
}

impl RuntimeStatus {
    pub fn from_wire(raw: &str) -> Self {
        match raw.trim() {
            "up" => Self::Up,
            "degraded" => Self::Degraded,
            "down" => Self::Down,
            _ => Self::Unknown,
        }
    }
}

/// A fingerprint badge's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FingerprintStatus {
    Pass,
    Mismatch,
    /// One mismatch, waiting for the confirming re-check.
    Suspect,
    Insufficient,
    Unknown,
}

/// The web page's `normalizeAstraCheckStatus`: a confirmed mismatch wins,
/// otherwise the latest run speaks, so an old pass never hides a new doubt.
pub fn fingerprint_status(stable: &str, verdict: &str) -> FingerprintStatus {
    if stable == "mismatch" {
        return FingerprintStatus::Mismatch;
    }
    match verdict {
        "match" => FingerprintStatus::Pass,
        "mismatch" => FingerprintStatus::Suspect,
        "insufficient" => FingerprintStatus::Insufficient,
        _ if stable == "pass" => FingerprintStatus::Pass,
        _ => FingerprintStatus::Unknown,
    }
}

/// How an event reads at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventTone {
    Good,
    Bad,
    Neutral,
}

pub fn event_tone(event_type: &str) -> EventTone {
    match event_type {
        "down" | "astra_mismatch" | "modeltrace_mismatch" | "sol_juice_mismatch" => EventTone::Bad,
        "up" | "astra_recovered" | "modeltrace_recovered" | "sol_juice_recovered" => {
            EventTone::Good
        }
        _ => EventTone::Neutral,
    }
}

pub fn is_astra_check_event(event_type: &str) -> bool {
    matches!(event_type, "astra_mismatch" | "astra_recovered")
}

/// Events the retired Juice and ModelTrace checks left in the history.
pub fn is_legacy_fingerprint_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "sol_juice_mismatch"
            | "sol_juice_recovered"
            | "modeltrace_mismatch"
            | "modeltrace_recovered"
    )
}

/// A fingerprint event's from/to states are pass/mismatch, not up/down.
pub fn is_fingerprint_event(event_type: &str) -> bool {
    is_astra_check_event(event_type) || is_legacy_fingerprint_event(event_type)
}

/// The models a fingerprint event names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AstraEventModels {
    pub expected: String,
    pub winner: String,
}

/// An event's `sub_status` is `<expected>:winner_<winner>`; events from the
/// single-model days carry only `winner_<winner>`, and those checked
/// GPT-6 Astra.
pub fn parse_astra_event_sub_status(sub_status: &str) -> AstraEventModels {
    let raw = sub_status.trim();
    const MARKER: &str = ":winner_";
    let (expected, mut winner) = if let Some(index) = raw.rfind(MARKER) {
        (
            raw[..index].to_owned(),
            raw[index + MARKER.len()..].to_owned(),
        )
    } else if let Some(winner) = raw.strip_prefix("winner_") {
        ("gpt-6-astra".to_owned(), winner.to_owned())
    } else {
        (String::new(), String::new())
    };
    if winner == "unknown" {
        winner.clear();
    }
    AstraEventModels { expected, winner }
}

/// The readable name of a fingerprint candidate id, as the gateway's own
/// table names it.
pub fn astra_model_label(model: &str) -> Option<&'static str> {
    Some(match model.trim() {
        "gpt-6-astra" => "GPT-6 Astra",
        "gpt-6-sol" => "GPT-6 Sol",
        "gpt-6-luna" => "GPT-6 Luna",
        "gpt-6.1-sol" => "GPT-6.1 Sol",
        "gpt-5.6-sol" => "GPT-5.6 Sol",
        "gpt-5.6-terra" => "GPT-5.6 Terra",
        "gpt-5.6-luna" => "GPT-5.6 Luna",
        "gpt-5.5" => "GPT-5.5",
        "gpt-5.4-mini" => "GPT-5.4 mini",
        "claude-opus-5.5" | "claude-opus-5-5" => "Claude Opus 5.5",
        "claude-fable-5.1" => "Claude Fable 5.1",
        "claude-sonnet-5" => "Claude Sonnet 5",
        "claude-haiku-4.5" | "claude-haiku-4-5-20251001" => "Claude Haiku 4.5",
        "gpt-5.5/5.4" => "GPT-5.5 / GPT-5.4",
        "gpt-5.4" => "GPT-5.4",
        "claude-sonnet-4-6" => "Claude Sonnet 4.6",
        "claude-opus-4-6" => "Claude Opus 4.6",
        "claude-opus-4-7" => "Claude Opus 4.7",
        "claude-opus-4-8" => "Claude Opus 4.8",
        "claude-opus-5" => "Claude Opus 5",
        _ => return None,
    })
}

/// A candidate's name for display: its label, else the id itself, `?` for
/// none. [`ASTRA_OTHER_MODEL`] is the caller's to localize.
pub fn model_label(model: &str) -> String {
    let id = model.trim();
    if id.is_empty() {
        return "?".to_owned();
    }
    astra_model_label(id).map_or_else(|| id.to_owned(), str::to_owned)
}

/// Strip upstream addresses from a probe error: the request prefix of a Go
/// `url.Error` (`Post "https://host/path": reason`) goes, and any other URL
/// becomes `[upstream]`. Newer gateways no longer send them; older records
/// still carry them.
pub fn sanitize_error_detail(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let without_prefixes = strip_request_prefixes(&chars);
    replace_urls(&without_prefixes).trim().to_owned()
}

const REQUEST_METHODS: [&str; 6] = ["get", "post", "put", "patch", "delete", "head"];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `chars[at..]` starts with `word`, ignoring ASCII case.
fn starts_with_ignore_case(chars: &[char], at: usize, word: &str) -> bool {
    let mut index = at;
    for expected in word.chars() {
        match chars.get(index) {
            Some(c) if c.eq_ignore_ascii_case(&expected) => index += 1,
            _ => return false,
        }
    }
    true
}

/// The length of a URL scheme (`http://` / `https://`) at `at`, if any.
fn url_scheme_len(chars: &[char], at: usize) -> Option<usize> {
    ["https://", "http://"]
        .into_iter()
        .find(|scheme| starts_with_ignore_case(chars, at, scheme))
        .map(str::len)
}

/// `\b(?:Get|Post|...)\s+"https?://[^"]*":\s*`, case-insensitive, removed.
fn strip_request_prefixes(chars: &[char]) -> Vec<char> {
    let matched_at = |start: usize| -> Option<usize> {
        if start > 0 && is_word(chars[start - 1]) {
            return None;
        }
        let method = REQUEST_METHODS
            .into_iter()
            .find(|method| starts_with_ignore_case(chars, start, method))?;
        let mut index = start + method.len();
        let spaces = chars[index..]
            .iter()
            .take_while(|c| c.is_whitespace())
            .count();
        if spaces == 0 {
            return None;
        }
        index += spaces;
        if chars.get(index) != Some(&'"') {
            return None;
        }
        index += 1;
        index += url_scheme_len(chars, index)?;
        index += chars[index..].iter().take_while(|c| **c != '"').count();
        if chars.get(index) != Some(&'"') || chars.get(index + 1) != Some(&':') {
            return None;
        }
        index += 2;
        index += chars[index..]
            .iter()
            .take_while(|c| c.is_whitespace())
            .count();
        Some(index)
    };
    let mut out = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        match matched_at(index) {
            Some(end) => index = end,
            None => {
                out.push(chars[index]);
                index += 1;
            }
        }
    }
    out
}

/// `https?://[^\s"'<>]+`, case-insensitive, replaced with `[upstream]`.
fn replace_urls(chars: &[char]) -> String {
    let stops = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>');
    let mut out = String::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if let Some(scheme) = url_scheme_len(chars, index) {
            let rest = chars[index + scheme..]
                .iter()
                .take_while(|c| !stops(**c))
                .count();
            if rest > 0 {
                out.push_str("[upstream]");
                index += scheme + rest;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// At most `max_chars` characters, with an ellipsis when cut.
pub fn shorten_excerpt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(max_chars).collect();
    format!("{}...", cut.trim_end())
}

/// JavaScript's `Math.round`: halves go up.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// JavaScript's `toFixed`, whose halves go up where Rust's formatting rounds
/// them to even.
fn to_fixed(value: f64, digits: usize) -> String {
    let scale = 10_f64.powi(digits as i32);
    format!("{:.digits$}", js_round(value * scale) / scale)
}

/// `312 ms`, `1.5 s`, `13 s`; `-` for none.
pub fn format_latency(ms: Option<f64>) -> String {
    let Some(ms) = ms.filter(|ms| ms.is_finite()) else {
        return "-".to_owned();
    };
    if ms < 1000.0 {
        return format!("{} ms", js_round(ms) as i64);
    }
    let seconds = ms / 1000.0;
    if seconds >= 10.0 {
        format!("{} s", to_fixed(seconds, 0))
    } else {
        format!("{} s", to_fixed(seconds, 1))
    }
}

/// `99.50%` near the top, `97.3%` below it; `-` for none.
pub fn format_availability(value: Option<f64>) -> String {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return "-".to_owned();
    };
    let digits = if value >= 99.0 { 2 } else { 1 };
    format!("{}%", to_fixed(value, digits))
}

/// The heartbeat bar's cells, oldest first: the indexes of the last
/// [`HEARTBEAT_RECORDS`] records, padded on the left with `None` when a
/// group has fewer.
pub fn heartbeat_slots(records: &[GroupStatusRecord]) -> Vec<Option<usize>> {
    let start = records.len().saturating_sub(HEARTBEAT_RECORDS);
    let shown = records.len() - start;
    std::iter::repeat_n(None, HEARTBEAT_RECORDS - shown)
        .chain((start..records.len()).map(Some))
        .collect()
}

/// The summary tiles' counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub total: usize,
    pub up: usize,
    pub degraded: usize,
    pub down: usize,
}

pub fn status_counts(entries: &[GroupStatusEntry]) -> StatusCounts {
    let mut counts = StatusCounts {
        total: entries.len(),
        ..StatusCounts::default()
    };
    for entry in entries {
        match entry.status() {
            RuntimeStatus::Up => counts.up += 1,
            RuntimeStatus::Degraded => counts.degraded += 1,
            RuntimeStatus::Down => counts.down += 1,
            RuntimeStatus::Unknown => {}
        }
    }
    counts
}

/// The administrator has not turned the page on (or the gateway predates
/// it): the list answers 404.
pub fn feature_disabled(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<crate::http::ApiError>())
        .any(|api| api.status == 404 || api.code == 404 || api.reason == FEATURE_DISABLED_REASON)
}

/// One fingerprint benchmark, for the notes at the foot of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BenchmarkSource {
    /// Names the source's description.
    pub key: &'static str,
    /// `meow`, `modeltrace` or `sol_juice`.
    pub method: &'static str,
    pub models: &'static [&'static str],
    pub repo: Option<&'static str>,
    pub site: Option<&'static str>,
    pub license: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BenchmarkFamily {
    /// `gpt` or `claude`.
    pub family: &'static str,
    pub sources: &'static [BenchmarkSource],
}

const MEOW_REPO_URL: &str = "https://github.com/chen-006/meow-llm-detector";
const MEOW_SITE_URL: &str = "https://meowllm.top/";
const MEOW_LICENSE: &str = "PolyForm Noncommercial 1.0.0";
const MODELTRACE_REPO_URL: &str = "https://github.com/xqy2006/ModelTrace";
const MODELTRACE_SITE_URL: &str = "https://xqy2006.github.io/ModelTrace/";

/// What each family is checked with; follows the gateway's
/// `astraCheckTargets`, as the web page's `FINGERPRINT_BENCHMARKS` does.
pub const FINGERPRINT_BENCHMARKS: &[BenchmarkFamily] = &[
    BenchmarkFamily {
        family: "gpt",
        sources: &[
            BenchmarkSource {
                key: "gpt_meow",
                method: "meow",
                models: &["gpt-6-sol", "gpt-6-astra"],
                repo: Some(MEOW_REPO_URL),
                site: Some(MEOW_SITE_URL),
                license: Some(MEOW_LICENSE),
            },
            BenchmarkSource {
                key: "gpt_modeltrace",
                method: "modeltrace",
                models: &["gpt-6.1-sol"],
                repo: Some(MODELTRACE_REPO_URL),
                site: Some(MODELTRACE_SITE_URL),
                license: Some("MIT"),
            },
            BenchmarkSource {
                key: "gpt_juice",
                method: "sol_juice",
                models: &["gpt-5.6-sol"],
                repo: None,
                site: None,
                license: None,
            },
        ],
    },
    BenchmarkFamily {
        family: "claude",
        sources: &[
            BenchmarkSource {
                key: "claude_modeltrace",
                method: "modeltrace",
                models: &["claude-opus-5.5", "claude-opus-5"],
                repo: Some(MODELTRACE_REPO_URL),
                site: Some(MODELTRACE_SITE_URL),
                license: Some("MIT"),
            },
            BenchmarkSource {
                key: "claude_meow",
                method: "meow",
                models: &["claude-fable-5.1"],
                repo: Some(MEOW_REPO_URL),
                site: Some(MEOW_SITE_URL),
                license: Some(MEOW_LICENSE),
            },
        ],
    },
];

/// `https://github.com/owner/repo` → `owner/repo`.
pub fn repo_name(url: &str) -> &str {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .unwrap_or(url);
    rest.strip_suffix('/').unwrap_or(rest)
}

fn records_path(group_id: i64, limit: usize) -> String {
    format!("/group-status/{group_id}/records?limit={limit}")
}

fn history_path(group_id: i64, period: HistoryPeriod) -> String {
    format!("/group-status/{group_id}/history?period={}", period.query())
}

fn events_path(group_id: i64, limit: usize) -> String {
    format!("/group-status/{group_id}/events?limit={limit}")
}

impl Client {
    /// The monitored groups this account can reach. One item that does not
    /// parse is skipped rather than failing the list.
    pub fn group_status_entries(&self, access_token: &str) -> Result<Vec<GroupStatusEntry>> {
        let raw: Vec<serde_json::Value> = self.get_or_default("/group-status", access_token)?;
        Ok(raw
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect())
    }

    /// A group's latest probes, oldest first.
    pub fn group_status_records(
        &self,
        access_token: &str,
        group_id: i64,
        limit: usize,
    ) -> Result<Vec<GroupStatusRecord>> {
        self.get_or_default(&records_path(group_id, limit), access_token)
    }

    pub fn group_status_history(
        &self,
        access_token: &str,
        group_id: i64,
        period: HistoryPeriod,
    ) -> Result<Vec<HistoryBucket>> {
        self.get_or_default(&history_path(group_id, period), access_token)
    }

    pub fn group_status_events(
        &self,
        access_token: &str,
        group_id: i64,
        limit: usize,
    ) -> Result<Vec<GroupStatusEvent>> {
        self.get_or_default(&events_path(group_id, limit), access_token)
    }
}

/// The page's data: every group plus its heartbeat probes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusBoard {
    pub entries: Vec<GroupStatusEntry>,
    pub records: HashMap<i64, Vec<GroupStatusRecord>>,
}

/// The list, then each group's last probes a few at a time. A group whose
/// probes fail to load draws an empty bar, as on the web page; only the list
/// failing fails the board. Blocking.
pub fn load_board(client: &Client, access_token: &str) -> Result<StatusBoard> {
    let entries = client.group_status_entries(access_token)?;
    let mut ids: Vec<i64> = entries
        .iter()
        .map(GroupStatusEntry::group_id)
        .filter(|id| *id > 0)
        .collect();
    ids.dedup();
    let queue = std::sync::Mutex::new(ids);
    let records = std::sync::Mutex::new(HashMap::new());
    std::thread::scope(|scope| {
        for _ in 0..RECORD_WORKERS {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().ok().and_then(|mut queue| queue.pop());
                    let Some(group_id) = next else { break };
                    let probes = client
                        .group_status_records(access_token, group_id, HEARTBEAT_RECORDS)
                        .unwrap_or_default();
                    if let Ok(mut records) = records.lock() {
                        records.insert(group_id, probes);
                    }
                }
            });
        }
    });
    Ok(StatusBoard {
        entries,
        records: records.into_inner().unwrap_or_default(),
    })
}

/// The details dialog's history and events, fetched together. Blocking.
pub fn load_details(
    client: &Client,
    access_token: &str,
    group_id: i64,
    period: HistoryPeriod,
) -> Result<(Vec<HistoryBucket>, Vec<GroupStatusEvent>)> {
    std::thread::scope(|scope| {
        let history = scope.spawn(|| client.group_status_history(access_token, group_id, period));
        let events = client.group_status_events(access_token, group_id, EVENT_LIMIT);
        let history = history
            .join()
            .map_err(|_| anyhow::anyhow!("loading the history panicked"))??;
        Ok((history, events?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(json: &str) -> GroupStatusEntry {
        serde_json::from_str(json).expect("entry")
    }

    #[test]
    fn group_info_reads_both_key_styles() {
        let pascal: GroupInfo = serde_json::from_str(
            r#"{"ID":9,"Name":"Std","Description":"","Platform":"openai","SortOrder":3}"#,
        )
        .expect("pascal");
        assert_eq!(
            (pascal.id, pascal.name.as_str(), pascal.platform.as_str()),
            (9, "Std", "openai")
        );
        let snake: GroupInfo = serde_json::from_str(
            r#"{"id":4,"name":"Fast","description":null,"platform":"anthropic"}"#,
        )
        .expect("snake");
        assert_eq!(
            (snake.id, snake.name.as_str(), snake.description.as_str()),
            (4, "Fast", "")
        );
        assert_eq!(GroupInfo::default().display_name(12), "#12");
    }

    #[test]
    fn entries_parse_the_live_shape() {
        let live = entry(
            r#"{"group":{"ID":9,"Name":"GPT Pro","Description":"官方渠道","Platform":"openai","DailyLimitUSD":null},
                "summary":{"group_id":9,"config_id":3,"enabled":true,"latest_status":"degraded",
                  "stable_status":"up","latency_ms":812,"total_latency_ms":null,"http_code":200,
                  "observed_at":"2026-10-03T12:00:00.123456789+08:00","error_detail":"",
                  "astra_check_enabled":true,"astra_check_states":[
                    {"expected_model":"gpt-6-sol","display_name":"","method":"meow","verdict":"match",
                     "stable_status":"pass","winner":"","checked_at":null}]},
                "availability_24h":99.5,"availability_7d":97.25}"#,
        );
        assert_eq!(live.group_id(), 9);
        assert_eq!(live.display_name(), "GPT Pro");
        assert_eq!(live.status(), RuntimeStatus::Up);
        assert!(!live.waiting());
        assert_eq!(live.summary.latency_ms, Some(812.0));
        assert_eq!(
            live.summary.astra_check_states[0].status(),
            FingerprintStatus::Pass
        );

        let bare = entry(
            r#"{"group":null,"summary":{"group_id":5,"astra_check_states":null,"observed_at":null}}"#,
        );
        assert_eq!(bare.group_id(), 5);
        assert_eq!(bare.display_name(), "#5");
        assert!(bare.waiting());
        assert_eq!(bare.status(), RuntimeStatus::Unknown);
        assert_eq!(bare.preview(), Preview::WaitingForProbe);
    }

    #[test]
    fn status_prefers_the_stable_one() {
        let stable = entry(r#"{"summary":{"latest_status":"down","stable_status":"degraded"}}"#);
        assert_eq!(stable.status(), RuntimeStatus::Degraded);
        let latest = entry(r#"{"summary":{"latest_status":"down","stable_status":""}}"#);
        assert_eq!(latest.status(), RuntimeStatus::Down);
    }

    #[test]
    fn preview_prefers_the_sanitized_error() {
        let failing = entry(
            r#"{"summary":{"observed_at":"2026-10-03T00:00:00Z","response_excerpt":"pong",
                "error_detail":"Post \"https://up.example.com/v1/messages\": dial tcp: i/o timeout"}}"#,
        );
        assert_eq!(
            failing.preview(),
            Preview::Text("dial tcp: i/o timeout".into())
        );
        let ok = entry(
            r#"{"summary":{"observed_at":"2026-10-03T00:00:00Z","response_excerpt":" pong "}}"#,
        );
        assert_eq!(ok.preview(), Preview::Text("pong".into()));
        let empty = entry(r#"{"summary":{"observed_at":"2026-10-03T00:00:00Z"}}"#);
        assert_eq!(empty.preview(), Preview::NotAvailable);
    }

    #[test]
    fn fingerprint_statuses_follow_the_web_page() {
        assert_eq!(
            fingerprint_status("mismatch", "match"),
            FingerprintStatus::Mismatch
        );
        assert_eq!(
            fingerprint_status("pass", "mismatch"),
            FingerprintStatus::Suspect
        );
        assert_eq!(
            fingerprint_status("pass", "insufficient"),
            FingerprintStatus::Insufficient
        );
        assert_eq!(fingerprint_status("", "match"), FingerprintStatus::Pass);
        assert_eq!(fingerprint_status("pass", ""), FingerprintStatus::Pass);
        assert_eq!(fingerprint_status("", ""), FingerprintStatus::Unknown);

        // The Juice reading keeps its confirmed verdict through one bad run.
        let juice = AstraCheckState {
            method: "sol_juice".into(),
            stable_status: "pass".into(),
            verdict: "mismatch".into(),
            ..AstraCheckState::default()
        };
        assert_eq!(juice.status(), FingerprintStatus::Pass);
        let meow = AstraCheckState {
            method: "meow".into(),
            ..juice
        };
        assert_eq!(meow.status(), FingerprintStatus::Suspect);
    }

    #[test]
    fn model_labels_cover_the_gateway_table() {
        assert_eq!(
            astra_model_label("claude-opus-5-5"),
            Some("Claude Opus 5.5")
        );
        assert_eq!(astra_model_label("gpt-6.1-sol"), Some("GPT-6.1 Sol"));
        assert_eq!(astra_model_label("mystery"), None);
        assert_eq!(model_label("mystery"), "mystery");
        assert_eq!(model_label("  "), "?");
    }

    #[test]
    fn event_sub_status_names_the_models() {
        assert_eq!(
            parse_astra_event_sub_status("gpt-6-sol:winner_claude-opus-5"),
            AstraEventModels {
                expected: "gpt-6-sol".into(),
                winner: "claude-opus-5".into()
            }
        );
        assert_eq!(
            parse_astra_event_sub_status("winner_gpt-5.5"),
            AstraEventModels {
                expected: "gpt-6-astra".into(),
                winner: "gpt-5.5".into()
            }
        );
        assert_eq!(parse_astra_event_sub_status("x:winner_unknown").winner, "");
        assert_eq!(
            parse_astra_event_sub_status(""),
            AstraEventModels::default()
        );
        assert_eq!(event_tone("astra_mismatch"), EventTone::Bad);
        assert_eq!(event_tone("modeltrace_recovered"), EventTone::Good);
        assert_eq!(event_tone("weird"), EventTone::Neutral);
        assert!(is_fingerprint_event("sol_juice_mismatch"));
        assert!(!is_fingerprint_event("down"));
    }

    #[test]
    fn error_details_lose_their_upstream_addresses() {
        assert_eq!(
            sanitize_error_detail(
                "Post \"https://up.example.com/v1/messages\": dial tcp: i/o timeout"
            ),
            "dial tcp: i/o timeout"
        );
        assert_eq!(
            sanitize_error_detail("upstream POST \"http://10.0.0.2:8080/x\": EOF"),
            "upstream EOF"
        );
        assert_eq!(
            sanitize_error_detail(
                "bad gateway from https://relay.example.org/v1?key=1 after 3 tries"
            ),
            "bad gateway from [upstream] after 3 tries"
        );
        // A method glued to a word is no request prefix.
        assert_eq!(
            sanitize_error_detail("XPost \"https://a.b/c\": no"),
            "XPost \"[upstream]\": no"
        );
        assert_eq!(sanitize_error_detail("   "), "");
        assert_eq!(
            sanitize_error_detail("连接超时 https://上游.cn/路径"),
            "连接超时 [upstream]"
        );
    }

    #[test]
    fn excerpts_are_cut_by_characters() {
        assert_eq!(shorten_excerpt("你好世界", 4), "你好世界");
        assert_eq!(shorten_excerpt("你好 世界", 3), "你好...");
        assert_eq!(shorten_excerpt("abcdef", 5), "abcde...");
    }

    #[test]
    fn latency_and_availability_format_like_the_web_page() {
        assert_eq!(format_latency(None), "-");
        assert_eq!(format_latency(Some(312.4)), "312 ms");
        assert_eq!(format_latency(Some(999.5)), "1000 ms");
        assert_eq!(format_latency(Some(1500.0)), "1.5 s");
        assert_eq!(format_latency(Some(1250.0)), "1.3 s");
        assert_eq!(format_latency(Some(12_500.0)), "13 s");
        assert_eq!(format_availability(None), "-");
        assert_eq!(format_availability(Some(99.5)), "99.50%");
        assert_eq!(format_availability(Some(100.0)), "100.00%");
        assert_eq!(format_availability(Some(97.25)), "97.3%");
    }

    #[test]
    fn heartbeat_pads_on_the_left_and_keeps_the_latest() {
        let records = |count: usize| vec![GroupStatusRecord::default(); count];
        let few = heartbeat_slots(&records(3));
        assert_eq!(few.len(), HEARTBEAT_RECORDS);
        assert!(few[..21].iter().all(Option::is_none));
        assert_eq!(&few[21..], &[Some(0), Some(1), Some(2)]);
        let many = heartbeat_slots(&records(30));
        assert_eq!(many.first(), Some(&Some(6)));
        assert_eq!(many.last(), Some(&Some(29)));
        assert!(heartbeat_slots(&[]).iter().all(Option::is_none));
    }

    #[test]
    fn history_bars_keep_a_visible_minimum() {
        let bucket = |total: i64, availability: f64| HistoryBucket {
            total_count: total,
            availability,
            ..HistoryBucket::default()
        };
        assert_eq!(bucket(0, 0.0).bar_height_percent(), 8.0);
        assert_eq!(bucket(5, 3.0).bar_height_percent(), 10.0);
        assert_eq!(bucket(5, 99.6).bar_height_percent(), 100.0);
        assert_eq!(bucket(5, 55.5).bar_height_percent(), 56.0);
        let down = HistoryBucket {
            down_count: 1,
            ..HistoryBucket::default()
        };
        assert_eq!(down.bar_status(), RuntimeStatus::Down);
        assert_eq!(HistoryBucket::default().bar_status(), RuntimeStatus::Up);
    }

    #[test]
    fn counts_split_by_status() {
        let entries = vec![
            entry(r#"{"summary":{"stable_status":"up"}}"#),
            entry(r#"{"summary":{"stable_status":"up"}}"#),
            entry(r#"{"summary":{"latest_status":"degraded"}}"#),
            entry(r#"{"summary":{"stable_status":"down"}}"#),
            entry(r#"{"summary":{}}"#),
        ];
        assert_eq!(
            status_counts(&entries),
            StatusCounts {
                total: 5,
                up: 2,
                degraded: 1,
                down: 1
            }
        );
    }

    #[test]
    fn a_disabled_page_is_told_from_other_failures() {
        let api = |status: u16, code: i64, reason: &str| -> anyhow::Error {
            crate::http::ApiError {
                status,
                code,
                reason: reason.into(),
                message: "x".into(),
            }
            .into()
        };
        assert!(feature_disabled(&api(404, 404, "")));
        assert!(feature_disabled(&api(200, 404, FEATURE_DISABLED_REASON)));
        assert!(!feature_disabled(&api(500, 500, "")));
        assert!(!feature_disabled(&api(403, 403, "GROUP_STATUS_FORBIDDEN")));
        assert!(!feature_disabled(&anyhow::anyhow!("timeout")));
    }

    #[test]
    fn paths_carry_the_period_and_limit() {
        assert_eq!(records_path(7, 24), "/group-status/7/records?limit=24");
        assert_eq!(
            history_path(7, HistoryPeriod::Week),
            "/group-status/7/history?period=7d"
        );
        assert_eq!(
            events_path(7, EVENT_LIMIT),
            "/group-status/7/events?limit=20"
        );
    }

    #[test]
    fn benchmarks_follow_the_gateway_targets() {
        let models: Vec<&str> = FINGERPRINT_BENCHMARKS
            .iter()
            .flat_map(|family| family.sources.iter())
            .flat_map(|source| source.models.iter().copied())
            .collect();
        assert_eq!(
            models,
            vec![
                "gpt-6-sol",
                "gpt-6-astra",
                "gpt-6.1-sol",
                "gpt-5.6-sol",
                "claude-opus-5.5",
                "claude-opus-5",
                "claude-fable-5.1"
            ]
        );
        assert!(
            models
                .iter()
                .all(|model| astra_model_label(model).is_some())
        );
        assert_eq!(repo_name(MEOW_REPO_URL), "chen-006/meow-llm-detector");
        assert_eq!(
            repo_name("https://github.com/xqy2006/ModelTrace/"),
            "xqy2006/ModelTrace"
        );
    }
}
