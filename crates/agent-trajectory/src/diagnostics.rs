//! Local diagnostic evidence derived from a trajectory.
//!
//! This is deliberately not a packet-capture diagnosis. The trajectory knows
//! how the engine classified a request, when its first token arrived, what it
//! retried and the usage it reported; it does not know response bytes, proxy
//! uptime or the upstream account. Findings therefore preserve those facts
//! and point at the next check instead of claiming an unobserved root cause.

use std::collections::HashMap;

use crate::metrics;
use crate::record::{FailureInfo, RequestStatus};
use crate::snapshot::{CellDetail, PromptChange, RequestView, Snapshot};

/// The same default slow-first-token threshold as the capture doctor.
pub const SLOW_FIRST_TOKEN_MS: i64 = 30_000;
/// Below this much input, a zero cache read is not useful evidence.
pub const MIN_CACHE_INPUT_TOKENS: u64 = 1_024;
/// Keep the report bounded even for a long, unhealthy session.
const MAX_FINDINGS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Percentiles {
    pub p50: Option<i64>,
    pub p95: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CacheSummary {
    /// Requests that reported usage.
    pub requests: u32,
    /// Uncached input plus cache reads and writes.
    pub input: u64,
    pub read: u64,
    pub write: u64,
    pub hit_ratio: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Finding {
    RequestFailed {
        cell: usize,
        request: u32,
        turn: u32,
        step: u32,
        model: String,
        failure: Option<FailureInfo>,
        error: Option<String>,
    },
    Retried {
        cell: usize,
        request: u32,
        retries: usize,
        reasons: Vec<String>,
        discarded_output: bool,
        recovered: bool,
    },
    SlowFirstToken {
        cell: usize,
        request: u32,
        waited_ms: i64,
        /// No token arrived; this is the whole wait before a transport-style
        /// failure, not an HTTP error answer.
        without_output: bool,
    },
    CacheReadDropped {
        cell: usize,
        request: u32,
        previous_request: u32,
        previous_read: u64,
        input: u64,
        since_previous_ms: i64,
        prompt_change: Option<PromptChange>,
    },
    CompactionFailed {
        cell: usize,
        turn: Option<u32>,
    },
}

impl Finding {
    pub fn cell(&self) -> usize {
        match self {
            Self::RequestFailed { cell, .. }
            | Self::Retried { cell, .. }
            | Self::SlowFirstToken { cell, .. }
            | Self::CacheReadDropped { cell, .. }
            | Self::CompactionFailed { cell, .. } => *cell,
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            Self::RequestFailed { .. } | Self::CompactionFailed { .. } => Severity::Error,
            Self::Retried {
                recovered: false, ..
            }
            | Self::Retried { retries: 3.., .. }
            | Self::SlowFirstToken { .. } => Severity::Warning,
            Self::Retried { .. } | Self::CacheReadDropped { .. } => Severity::Info,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub requests: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub retries: usize,
    pub retried_requests: u32,
    pub slow_first_token: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub first_token: Percentiles,
    pub total: Percentiles,
    pub cache: Option<CacheSummary>,
    pub findings: Vec<Finding>,
    pub omitted_findings: usize,
}

impl Report {
    /// Stable, machine-readable English context for pasting into an issue or
    /// handing to an agent. The panel itself localizes the same findings.
    /// Prompts, tool arguments and tool results are deliberately excluded.
    pub fn markdown(&self) -> String {
        let mut out = String::from(
            "# Trajectory diagnostic context\n\n\
             Local evidence for issue analysis, not a root-cause verdict.\n\n\
             ## Overview\n",
        );
        out.push_str(&format!(
            "- Requests: {} (failed: {}, cancelled: {})\n",
            self.requests, self.failed, self.cancelled
        ));
        out.push_str(&format!(
            "- Retries: {} across {} requests\n",
            self.retries, self.retried_requests
        ));
        out.push_str(&format!(
            "- Slow first token (>{}): {}\n",
            metrics::format_duration(SLOW_FIRST_TOKEN_MS),
            self.slow_first_token
        ));
        out.push_str(&format!(
            "- Tool calls: {} (errors: {})\n",
            self.tool_calls, self.tool_errors
        ));
        write_percentiles(&mut out, "First token", self.first_token);
        write_percentiles(&mut out, "Total request time", self.total);
        if let Some(cache) = self.cache {
            out.push_str(&format!(
                "- Cache: {} read / {} total input across {} requests with usage",
                cache.read, cache.input, cache.requests
            ));
            if let Some(ratio) = cache.hit_ratio {
                out.push_str(&format!(" ({:.1}%)", ratio * 100.0));
            }
            out.push_str(&format!("; {} cache write\n", cache.write));
        } else {
            out.push_str("- Cache: no request reported usage\n");
        }

        out.push_str("\n## Findings\n");
        if self.findings.is_empty() {
            out.push_str("No failure, retry, slow-first-token or cache-drop signal found.\n");
        }
        for (index, finding) in self.findings.iter().enumerate() {
            out.push_str(&format!("\n### {}. ", index + 1));
            match finding {
                Finding::RequestFailed {
                    request,
                    turn,
                    step,
                    model,
                    failure,
                    error,
                    ..
                } => {
                    out.push_str(&format!(
                        "Request #{request} failed (turn {turn}, step {step})\n"
                    ));
                    out.push_str(&format!("- Model: {}\n", inline(model, 200)));
                    if let Some(failure) = failure {
                        out.push_str(&format!(
                            "- Failure: {} / {}\n",
                            inline(&failure.class, 80),
                            inline(&failure.reason, 80)
                        ));
                        if let Some(status) = failure.status {
                            out.push_str(&format!("- HTTP status: {status}\n"));
                        }
                        if let Some(code) = &failure.code {
                            out.push_str(&format!("- Error code: {}\n", inline(code, 160)));
                        }
                        if let Some(id) = &failure.request_id {
                            out.push_str(&format!("- Request ID: {}\n", inline(id, 200)));
                        }
                    }
                    if let Some(error) = error {
                        out.push_str(&format!("- Recorded error: {}\n", inline(error, 600)));
                    }
                    out.push_str(
                        "- Next check: correlate the request ID, status and error code with provider/backend records.\n",
                    );
                }
                Finding::Retried {
                    request,
                    retries,
                    reasons,
                    discarded_output,
                    recovered,
                    ..
                } => {
                    out.push_str(&format!("Request #{request} retried\n"));
                    out.push_str(&format!("- Retries: {retries}\n"));
                    out.push_str(&format!("- Reasons: {}\n", reasons.join(", ")));
                    out.push_str(&format!("- Recovered: {}\n", yes_no(*recovered)));
                    if *discarded_output {
                        out.push_str("- A partial answer was discarded by an older engine.\n");
                    }
                    out.push_str(
                        "- Next check: repeated network/timeout reasons call for connection evidence; rate/server reasons call for provider records.\n",
                    );
                }
                Finding::SlowFirstToken {
                    request,
                    waited_ms,
                    without_output,
                    ..
                } => {
                    out.push_str(&format!("Request #{request} waited slowly for output\n"));
                    out.push_str(&format!(
                        "- Wait: {}\n",
                        metrics::format_duration(*waited_ms)
                    ));
                    out.push_str(&format!("- No token arrived: {}\n", yes_no(*without_output)));
                    out.push_str(
                        "- Next check: compare provider latency/status and network captures around this request.\n",
                    );
                }
                Finding::CacheReadDropped {
                    request,
                    previous_request,
                    previous_read,
                    input,
                    since_previous_ms,
                    prompt_change,
                    ..
                } => {
                    out.push_str(&format!(
                        "Request #{request} read zero cache after request #{previous_request}\n"
                    ));
                    out.push_str(&format!(
                        "- Previous cache read: {previous_read}; current total input: {input}\n"
                    ));
                    out.push_str(&format!(
                        "- Since previous request: {}\n",
                        metrics::format_duration(*since_previous_ms)
                    ));
                    if let Some(change) = prompt_change {
                        out.push_str(&format!("- Same-turn prompt/catalog change: {change:?}\n"));
                    }
                    out.push_str(
                        "- Next check: inspect the system/tool diff and verify provider cache/account records. This signal alone cannot distinguish a prefix change, expiry, eviction or account switch.\n",
                    );
                }
                Finding::CompactionFailed { turn, .. } => {
                    out.push_str("Context compaction failed\n");
                    if let Some(turn) = turn {
                        out.push_str(&format!("- Turn: {turn}\n"));
                    }
                    out.push_str("- Next check: inspect the adjacent request failure and context usage.\n");
                }
            }
            out.push_str(&format!("- Trajectory cell: {}\n", finding.cell()));
        }
        if self.omitted_findings > 0 {
            out.push_str(&format!(
                "\n{} further findings were omitted to keep this report bounded.\n",
                self.omitted_findings
            ));
        }
        out.push_str(
            "\n## Evidence limits\n\
             - Request start and first-token timing come from the agent engine; start is inferred and retries before the first token are included.\n\
             - No raw request-body segmentation, response-header/byte timing, chunk gaps, proxy uptime or upstream account is recorded here.\n\
             - Cache findings are correlations, not prefix-attribution conclusions. Use raw captures/backend records before choosing a fix.\n\
             - Prompts, tool arguments and tool results are not included in this report.\n",
        );
        out
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn inline(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= limit {
        flat
    } else {
        format!("{}…", flat.chars().take(limit).collect::<String>())
    }
}

fn write_percentiles(out: &mut String, label: &str, values: Percentiles) {
    let Some(p50) = values.p50 else {
        out.push_str(&format!("- {label}: unavailable\n"));
        return;
    };
    out.push_str(&format!("- {label}: p50 {}", metrics::format_duration(p50)));
    if let Some(p95) = values.p95 {
        out.push_str(&format!(", p95 {}", metrics::format_duration(p95)));
    }
    if let Some(max) = values.max {
        out.push_str(&format!(", max {}", metrics::format_duration(max)));
    }
    out.push('\n');
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheRoute {
    provider: String,
    model: String,
    api_base: String,
    wire_format: String,
    parent_call_id: Option<String>,
}

impl CacheRoute {
    fn of(request: &RequestView) -> Self {
        Self {
            provider: request.provider.clone(),
            model: request.model.clone(),
            api_base: request.api_base.clone(),
            wire_format: request.wire_format.clone(),
            parent_call_id: request.parent_call_id.clone(),
        }
    }
}

#[derive(Clone, Copy)]
struct CacheObservation {
    request: u32,
    read: u64,
    ended_at: i64,
}

fn push_finding(report: &mut Report, finding: Finding) {
    if report.findings.len() < MAX_FINDINGS {
        report.findings.push(finding);
    } else {
        report.omitted_findings += 1;
    }
}

fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.saturating_sub(1).min(sorted.len() - 1)).copied()
}

fn percentiles(mut values: Vec<i64>) -> Percentiles {
    values.sort_unstable();
    Percentiles {
        p50: percentile(&values, 0.5),
        p95: percentile(&values, 0.95),
        max: values.last().copied(),
    }
}

/// The wait used by the slow-first-token check. An HTTP error is an answer,
/// not a slow wait; a transport-style failure with no output waits for its
/// whole duration, as the capture doctor's connection-quality check does.
fn waited(request: &RequestView) -> Option<(i64, bool)> {
    if let Some(first) = request.first_token_at {
        return Some(((first - request.started_at).max(0), false));
    }
    let failure = request.failure.as_ref()?;
    if request.status != RequestStatus::Error
        || failure.status.is_some_and(|status| status >= 400)
        || !matches!(
            failure.reason.as_str(),
            "network" | "timeout" | "stream_interrupted" | "truncated"
        )
    {
        return None;
    }
    Some((metrics::total(request), true))
}

fn total_input(usage: &crate::record::Usage) -> u64 {
    usage.input + usage.cache_read + usage.cache_write
}

/// Derive bounded, read-only evidence from the already-built ledger.
pub fn analyze(snapshot: &Snapshot) -> Report {
    let mut report = Report {
        requests: snapshot.requests,
        ..Report::default()
    };
    let mut first_tokens = Vec::new();
    let mut totals = Vec::new();
    let mut prompt_changes: HashMap<u32, PromptChange> = HashMap::new();
    let mut previous_cache: HashMap<CacheRoute, CacheObservation> = HashMap::new();

    for cell in &snapshot.cells {
        match &cell.detail {
            CellDetail::System { change, .. } => {
                if let Some(turn) = cell.turn
                    && *change != PromptChange::Initial
                {
                    prompt_changes.insert(turn, *change);
                }
            }
            CellDetail::Request(request) => {
                match request.status {
                    RequestStatus::Error => report.failed += 1,
                    RequestStatus::Cancelled => report.cancelled += 1,
                    RequestStatus::Running | RequestStatus::Complete => {}
                }
                if request.status == RequestStatus::Error {
                    push_finding(
                        &mut report,
                        Finding::RequestFailed {
                            cell: cell.index,
                            request: request.number,
                            turn: request.turn,
                            step: request.step,
                            model: request.model.clone(),
                            failure: request.failure.clone(),
                            error: request.error.clone(),
                        },
                    );
                }
                if !request.retries.is_empty() {
                    report.retries += request.retries.len();
                    report.retried_requests += 1;
                    push_finding(
                        &mut report,
                        Finding::Retried {
                            cell: cell.index,
                            request: request.number,
                            retries: request.retries.len(),
                            reasons: request
                                .retries
                                .iter()
                                .map(|retry| retry.reason.clone())
                                .filter(|reason| !reason.is_empty())
                                .collect(),
                            discarded_output: request
                                .retries
                                .iter()
                                .any(|retry| retry.discarded_output),
                            recovered: request.status == RequestStatus::Complete,
                        },
                    );
                }
                if let Some((waited_ms, without_output)) = waited(request)
                    && waited_ms > SLOW_FIRST_TOKEN_MS
                {
                    report.slow_first_token += 1;
                    push_finding(
                        &mut report,
                        Finding::SlowFirstToken {
                            cell: cell.index,
                            request: request.number,
                            waited_ms,
                            without_output,
                        },
                    );
                }
                if request.status == RequestStatus::Complete {
                    totals.push(metrics::total(request));
                    if let Some(first) = request.first_token_at {
                        first_tokens.push((first - request.started_at).max(0));
                    }
                    if let Some(usage) = request.usage {
                        let cache = report.cache.get_or_insert_with(CacheSummary::default);
                        cache.requests += 1;
                        cache.input += total_input(&usage);
                        cache.read += usage.cache_read;
                        cache.write += usage.cache_write;

                        let route = CacheRoute::of(request);
                        let input = total_input(&usage);
                        if let Some(previous) = previous_cache.get(&route).copied()
                            && previous.read > 0
                            && usage.cache_read == 0
                            && input >= MIN_CACHE_INPUT_TOKENS
                            && request.started_at >= previous.ended_at
                        {
                            push_finding(
                                &mut report,
                                Finding::CacheReadDropped {
                                    cell: cell.index,
                                    request: request.number,
                                    previous_request: previous.request,
                                    previous_read: previous.read,
                                    input,
                                    since_previous_ms: (request.started_at
                                        - previous.ended_at)
                                        .max(0),
                                    prompt_change: prompt_changes.get(&request.turn).copied(),
                                },
                            );
                        }
                        previous_cache.insert(
                            route,
                            CacheObservation {
                                request: request.number,
                                read: usage.cache_read,
                                ended_at: request.ended_at,
                            },
                        );
                    }
                }
            }
            CellDetail::Tool(tool) => {
                report.tool_calls += 1;
                report.tool_errors += u32::from(tool.is_error);
            }
            CellDetail::Compaction {
                phase: crate::record::CompactionPhase::Failed,
                ..
            } => push_finding(
                &mut report,
                Finding::CompactionFailed {
                    cell: cell.index,
                    turn: cell.turn,
                },
            ),
            _ => {}
        }
    }
    report.first_token = percentiles(first_tokens);
    report.total = percentiles(totals);
    if let Some(cache) = &mut report.cache {
        cache.hit_ratio = (cache.input > 0).then_some(cache.read as f64 / cache.input as f64);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Record, RetryInfo, Usage};
    use crate::snapshot::tests::{request, sample, turn_start};
    use crate::snapshot::build;

    fn with_usage(mut record: Record, input: u64, cache_read: u64) -> Record {
        let Record::Request { usage, .. } = &mut record else {
            panic!("a request")
        };
        *usage = Some(Usage {
            input,
            output: 10,
            cache_read,
            cache_write: 0,
        });
        record
    }

    #[test]
    fn a_healthy_sample_has_metrics_without_findings() {
        let report = analyze(&build(&sample()));
        assert_eq!(report.requests, 2);
        assert_eq!(report.failed, 0);
        assert!(report.findings.is_empty());
        assert_eq!(report.first_token.p50, Some(200));
        assert_eq!(report.total.max, Some(1_000));
        assert_eq!(report.cache.unwrap().requests, 2);
    }

    #[test]
    fn failure_retry_and_no_output_wait_are_linked_to_the_request() {
        let mut records = vec![turn_start(1, 0, "s1", "t1")];
        let mut failed = request(1, 1, 0, 0, 40_000, 0);
        let Record::Request {
            first_token_at,
            status,
            retries,
            failure,
            error,
            ..
        } = &mut failed
        else {
            unreachable!()
        };
        *first_token_at = None;
        *status = RequestStatus::Error;
        *retries = vec![RetryInfo {
            attempt: 1,
            max_attempts: Some(2),
            delay_ms: 2_000,
            error: "timed out".into(),
            reason: "timeout".into(),
            discarded_output: false,
            request_id: Some("req_retry".into()),
        }];
        *failure = Some(FailureInfo {
            class: "retryable".into(),
            reason: "timeout".into(),
            status: None,
            code: None,
            request_id: Some("req_final".into()),
        });
        *error = Some("request timed out".into());
        records.push(failed);
        let report = analyze(&build(&records));
        assert_eq!(report.failed, 1);
        assert_eq!(report.retries, 1);
        assert_eq!(report.slow_first_token, 1);
        assert!(matches!(
            &report.findings[0],
            Finding::RequestFailed { request: 1, .. }
        ));
        assert!(matches!(
            &report.findings[2],
            Finding::SlowFirstToken {
                waited_ms: 40_000,
                without_output: true,
                ..
            }
        ));
        let markdown = report.markdown();
        assert!(markdown.contains("req_final"));
        assert!(markdown.contains("Evidence limits"));
        assert!(!markdown.contains("prompt 1"));
    }

    #[test]
    fn an_http_error_is_not_counted_as_a_slow_wait() {
        let mut failed = request(1, 1, 0, 0, 60_000, 0);
        let Record::Request {
            first_token_at,
            status,
            failure,
            ..
        } = &mut failed
        else {
            unreachable!()
        };
        *first_token_at = None;
        *status = RequestStatus::Error;
        *failure = Some(FailureInfo {
            class: "retryable".into(),
            reason: "server_error".into(),
            status: Some(500),
            code: None,
            request_id: None,
        });
        let snapshot = build(&[turn_start(1, 0, "s1", "t1"), failed.clone()]);
        let report = analyze(&snapshot);
        assert_eq!(report.slow_first_token, 0);
        assert_eq!(report.findings.len(), 1);

        let Record::Request { failure, .. } = &mut failed else {
            unreachable!()
        };
        *failure = Some(FailureInfo {
            class: "retryable".into(),
            reason: "truncated".into(),
            status: Some(200),
            code: None,
            request_id: None,
        });
        let report = analyze(&build(&[turn_start(1, 0, "s1", "t1"), failed]));
        assert_eq!(report.slow_first_token, 1);
    }

    #[test]
    fn a_cache_drop_is_correlated_not_called_a_root_cause() {
        let records = vec![
            turn_start(1, 0, "s1", "t1"),
            with_usage(request(1, 1, 0, 100, 1_000, 10), 100, 2_000),
            turn_start(2, 2_000, "s2", "t1"),
            with_usage(request(2, 1, 2_000, 2_100, 3_000, 10), 2_000, 0),
        ];
        let report = analyze(&build(&records));
        let finding = report
            .findings
            .iter()
            .find_map(|finding| match finding {
                Finding::CacheReadDropped {
                    previous_request,
                    previous_read,
                    prompt_change,
                    ..
                } => Some((previous_request, previous_read, prompt_change)),
                _ => None,
            })
            .unwrap();
        assert_eq!(finding, (&1, &2_000, &Some(PromptChange::System)));
        assert!(report.markdown().contains("cannot distinguish"));
    }

    #[test]
    fn a_small_or_concurrent_zero_cache_read_is_not_a_drop() {
        let small = vec![
            turn_start(1, 0, "s1", "t1"),
            with_usage(request(1, 1, 0, 100, 1_000, 10), 100, 2_000),
            with_usage(request(1, 2, 1_000, 1_100, 2_000, 10), 100, 0),
        ];
        assert!(analyze(&build(&small)).findings.is_empty());

        let concurrent = vec![
            turn_start(1, 0, "s1", "t1"),
            with_usage(request(1, 1, 0, 100, 5_000, 10), 2_000, 2_000),
            with_usage(request(1, 2, 1_000, 1_100, 2_000, 10), 2_000, 0),
        ];
        assert!(analyze(&build(&concurrent)).findings.is_empty());
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let values = percentiles(vec![40, 10, 30, 20]);
        assert_eq!(values.p50, Some(20));
        assert_eq!(values.p95, Some(40));
        assert_eq!(values.max, Some(40));
        assert_eq!(percentiles(Vec::new()), Percentiles::default());
    }
}
