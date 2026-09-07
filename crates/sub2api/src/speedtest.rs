//! Concurrent latency test over a set of candidate origins.
//!
//! Borrowed from CC Switch's endpoint speed test: every candidate gets one
//! warm-up request (connection setup, TLS, the first-packet penalty — its
//! result is discarded) and one timed request; candidates run in parallel;
//! and the results come back in the order the URLs were given, so a caller
//! can line them up with its own list. This only informs a choice made
//! before a CLI starts — the winner is written into the CLI's config — it is
//! not a runtime failover.
//!
//! Each probe is a `curl` subprocess (see `http`), so concurrency is bounded
//! and the whole run blocks; callers run it on a background thread.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::custom_api::{
    ProbeResult, ProbeVerdict, UrlError, normalize_base_url, probe_endpoint_with_timeout,
};

pub const MIN_TIMEOUT_SECS: u32 = 2;
pub const MAX_TIMEOUT_SECS: u32 = 30;
pub const DEFAULT_TIMEOUT_SECS: u32 = 8;
/// How many probes run at once. Each is a subprocess; on Windows each is a
/// console-less child, so this stays small.
pub const MAX_CONCURRENCY: usize = 6;

/// Bring a requested timeout into the supported range.
pub fn clamp_timeout(secs: Option<u32>) -> u32 {
    secs.unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS)
}

/// How a measured latency reads. Thresholds follow CC Switch's badge colours
/// (300 / 500 / 800 ms); the UI pairs each tier with the number itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatencyTier {
    Fast,
    Ok,
    Slow,
    VerySlow,
}

pub fn latency_tier(ms: u128) -> LatencyTier {
    if ms < 300 {
        LatencyTier::Fast
    } else if ms < 500 {
        LatencyTier::Ok
    } else if ms < 800 {
        LatencyTier::Slow
    } else {
        LatencyTier::VerySlow
    }
}

/// One candidate's outcome. Exactly one of `result` / `invalid` is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateResult {
    /// The normalized URL that was probed, or the raw text when invalid.
    pub url: String,
    pub result: Option<ProbeResult>,
    /// The URL never went on the wire.
    pub invalid: Option<UrlError>,
}

impl CandidateResult {
    /// The endpoint answered the models listing with 2xx.
    pub fn is_ok(&self) -> bool {
        self.result
            .as_ref()
            .is_some_and(|result| result.verdict == ProbeVerdict::Ok)
    }

    /// Measured latency, only for a successful probe.
    pub fn latency_ms(&self) -> Option<u128> {
        self.result
            .as_ref()
            .filter(|result| result.verdict == ProbeVerdict::Ok)
            .map(|result| result.latency_ms)
    }
}

/// Probe every URL with the CLI's protocol and key. Blocking; results are in
/// input order. Invalid URLs are reported without being probed.
pub fn test_candidates(
    provider_id: &str,
    urls: &[String],
    api_key: &str,
    timeout_secs: u32,
) -> Vec<CandidateResult> {
    let timeout = clamp_timeout(Some(timeout_secs));
    let mut results: Vec<Option<CandidateResult>> = Vec::with_capacity(urls.len());
    let mut pending: Vec<(usize, String)> = Vec::new();
    for (index, raw) in urls.iter().enumerate() {
        match normalize_base_url(raw) {
            Ok(url) => {
                results.push(None);
                pending.push((index, url));
            }
            Err(error) => results.push(Some(CandidateResult {
                url: raw.clone(),
                result: None,
                invalid: Some(error),
            })),
        }
    }
    if pending.is_empty() {
        return results.into_iter().flatten().collect();
    }

    let slots = Mutex::new(results);
    let next = AtomicUsize::new(0);
    let workers = pending.len().min(MAX_CONCURRENCY);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let job = next.fetch_add(1, Ordering::Relaxed);
                    let Some((index, url)) = pending.get(job) else {
                        break;
                    };
                    let result = probe_twice(provider_id, url, api_key, timeout);
                    let outcome = CandidateResult {
                        url: url.clone(),
                        result: Some(result),
                        invalid: None,
                    };
                    slots.lock().unwrap()[*index] = Some(outcome);
                }
            });
        }
    });
    slots
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .into_iter()
        .flatten()
        .collect()
}

/// Warm up, then measure. A host that does not answer the warm-up at all is
/// reported from that attempt — a second wait on the same timeout would only
/// double the time to the same verdict.
fn probe_twice(provider_id: &str, url: &str, api_key: &str, timeout: u32) -> ProbeResult {
    let warm_up = probe_endpoint_with_timeout(provider_id, url, api_key, timeout);
    if warm_up.verdict == ProbeVerdict::Unreachable {
        return warm_up;
    }
    probe_endpoint_with_timeout(provider_id, url, api_key, timeout)
}

/// Index of the fastest candidate that answered successfully.
pub fn fastest_ok(results: &[CandidateResult]) -> Option<usize> {
    results
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| candidate.latency_ms().map(|ms| (index, ms)))
        .min_by_key(|(_, ms)| *ms)
        .map(|(index, _)| index)
}

/// Model ids from a models listing: OpenAI's `{data: [{id}]}`, Anthropic's
/// identical shape, or a bare array. Order kept, duplicates dropped.
pub fn model_ids_from_body(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let empty = Vec::new();
    let items: &Vec<Value> = match &value {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("data")
            .or_else(|| map.get("models"))
            .and_then(Value::as_array)
            .unwrap_or(&empty),
        _ => &empty,
    };
    let mut ids: Vec<String> = Vec::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty());
        if let Some(id) = id
            && !ids.iter().any(|known| known == id)
        {
            ids.push(id.to_owned());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    use super::*;

    #[test]
    fn clamp_timeout_bounds() {
        assert_eq!(clamp_timeout(None), DEFAULT_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(Some(0)), MIN_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(Some(999)), MAX_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(Some(12)), 12);
    }

    #[test]
    fn latency_tier_thresholds() {
        assert_eq!(latency_tier(0), LatencyTier::Fast);
        assert_eq!(latency_tier(299), LatencyTier::Fast);
        assert_eq!(latency_tier(300), LatencyTier::Ok);
        assert_eq!(latency_tier(499), LatencyTier::Ok);
        assert_eq!(latency_tier(500), LatencyTier::Slow);
        assert_eq!(latency_tier(799), LatencyTier::Slow);
        assert_eq!(latency_tier(800), LatencyTier::VerySlow);
    }

    #[test]
    fn invalid_urls_fill_in_place_without_probing() {
        let results = test_candidates(
            "codex",
            &["".to_owned(), "ftp://x.example".to_owned()],
            "",
            5,
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].invalid, Some(UrlError::Empty));
        assert_eq!(results[1].invalid, Some(UrlError::Scheme("ftp".to_owned())));
        assert!(results.iter().all(|candidate| candidate.result.is_none()));
        assert!(fastest_ok(&results).is_none());
    }

    fn probe(verdict: ProbeVerdict, latency_ms: u128) -> CandidateResult {
        CandidateResult {
            url: format!("https://{latency_ms}.example"),
            result: Some(ProbeResult {
                latency_ms,
                status: Some(200),
                verdict,
                detail: String::new(),
                body: String::new(),
            }),
            invalid: None,
        }
    }

    #[test]
    fn fastest_ok_ignores_non_ok() {
        let results = vec![
            probe(ProbeVerdict::Unauthorized, 10),
            probe(ProbeVerdict::Ok, 400),
            probe(ProbeVerdict::HttpError, 20),
            probe(ProbeVerdict::Ok, 250),
            probe(ProbeVerdict::Unreachable, 5),
        ];
        assert_eq!(fastest_ok(&results), Some(3));
        assert!(results[0].latency_ms().is_none());
        assert_eq!(results[3].latency_ms(), Some(250));
    }

    #[test]
    fn model_ids_from_body_handles_both_shapes() {
        assert_eq!(
            model_ids_from_body(
                r#"{"object":"list","data":[{"id":"gpt-a"},{"id":"gpt-b"},{"id":"gpt-a"}]}"#
            ),
            vec!["gpt-a".to_owned(), "gpt-b".to_owned()]
        );
        assert_eq!(
            model_ids_from_body(
                r#"{"data":[{"id":"claude-x","type":"model","display_name":"X"}]}"#
            ),
            vec!["claude-x".to_owned()]
        );
        assert_eq!(
            model_ids_from_body(r#"["m1", {"id":"m2"}, {"name":"no-id"}, " "]"#),
            vec!["m1".to_owned(), "m2".to_owned()]
        );
        assert!(model_ids_from_body("not json").is_empty());
        assert!(model_ids_from_body(r#"{"models":"x"}"#).is_empty());
    }

    /// A loopback server that answers every request with a models listing
    /// after `delay`. Returns its origin.
    fn spawn_delayed_server(delay: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let origin = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0
                        || line.trim_end().is_empty()
                    {
                        break;
                    }
                }
                std::thread::sleep(delay);
                let body = r#"{"data":[{"id":"m-1"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        origin
    }

    #[test]
    fn loopback_results_keep_order_and_pick_the_fast_one() {
        let slow = spawn_delayed_server(Duration::from_millis(700));
        let fast = spawn_delayed_server(Duration::ZERO);
        let urls = vec![slow.clone(), "not a url".to_owned(), fast.clone()];
        let results = test_candidates("codex", &urls, "sk", 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].url, slow);
        assert_eq!(results[2].url, fast);
        assert!(results[1].invalid.is_some());
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert!(results[2].is_ok(), "{:?}", results[2]);
        assert!(results[0].latency_ms().unwrap() >= 700);
        assert_eq!(fastest_ok(&results), Some(2));
        assert_eq!(
            model_ids_from_body(&results[2].result.as_ref().unwrap().body),
            vec!["m-1".to_owned()]
        );
    }
}
