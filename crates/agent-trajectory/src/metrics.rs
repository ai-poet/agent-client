//! A request's timing: how long until the first token, how long the rest
//! took, and how fast it came.
//!
//! Each figure is either a value or the reason there is none, as dsh's
//! `timing.*` copy distinguishes them: the engine did not report a first
//! token, the request reported no usage, or it was too short to divide by.
//!
//! Translated from DeepSeek Harness's `ui-trajectory` timing rules — MIT
//! License, Copyright (c) 2026 DeepSeek. See NOTICE.md.

use crate::snapshot::RequestView;

/// Below this, a generation time is too short for a throughput figure.
pub const MIN_GENERATION_MS: i64 = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timing<T> {
    Value(T),
    /// The engine did not report the first token.
    FirstTokenUnavailable,
    /// The request reported no usage.
    UsageUnavailable,
    /// Too short to divide by.
    TooShort,
}

/// Step start to first token.
pub fn ttft(request: &RequestView) -> Timing<i64> {
    match request.first_token_at {
        Some(first) => Timing::Value((first - request.started_at).max(0)),
        None => Timing::FirstTokenUnavailable,
    }
}

/// First token to the end.
pub fn generation(request: &RequestView) -> Timing<i64> {
    match request.first_token_at {
        Some(first) => Timing::Value((request.ended_at - first).max(0)),
        None => Timing::FirstTokenUnavailable,
    }
}

/// Output tokens per second of generation.
pub fn throughput(request: &RequestView) -> Timing<f64> {
    let generation = match generation(request) {
        Timing::Value(ms) => ms,
        _ => return Timing::FirstTokenUnavailable,
    };
    let Some(usage) = request.usage else {
        return Timing::UsageUnavailable;
    };
    if generation < MIN_GENERATION_MS {
        return Timing::TooShort;
    }
    Timing::Value(usage.output as f64 * 1000.0 / generation as f64)
}

/// The whole request.
pub fn total(request: &RequestView) -> i64 {
    (request.ended_at - request.started_at).max(0)
}

/// `1,234 ms` or `12.3 s`.
pub fn format_duration(ms: i64) -> String {
    if ms < 10_000 {
        let digits = ms.to_string();
        let mut grouped = String::new();
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(digit);
        }
        format!("{grouped} ms")
    } else {
        format!("{:.1} s", ms as f64 / 1000.0)
    }
}

/// `1.2k`, `34k`, `1.1M`.
pub fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..=999 => tokens.to_string(),
        1_000..=9_999 => format!("{:.1}k", tokens as f64 / 1000.0),
        10_000..=999_999 => format!("{}k", tokens / 1000),
        _ => format!("{:.1}M", tokens as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Record;
    use crate::snapshot::tests::sample;
    use crate::snapshot::{CellDetail, build};

    fn first_request() -> RequestView {
        let snapshot = build(&sample());
        snapshot
            .cells
            .iter()
            .find_map(|cell| match &cell.detail {
                CellDetail::Request(request) => Some((**request).clone()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn timing_reads_the_recorded_moments() {
        let request = first_request();
        assert_eq!(ttft(&request), Timing::Value(400));
        assert_eq!(generation(&request), Timing::Value(600));
        assert_eq!(total(&request), 1_000);
        match throughput(&request) {
            Timing::Value(rate) => assert!((rate - 83.33).abs() < 0.1, "{rate}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_facts_say_why() {
        let mut request = first_request();
        request.usage = None;
        assert_eq!(throughput(&request), Timing::UsageUnavailable);
        request.first_token_at = None;
        assert_eq!(ttft(&request), Timing::FirstTokenUnavailable);
        let mut short = first_request();
        short.first_token_at = Some(short.ended_at - 10);
        assert_eq!(throughput(&short), Timing::TooShort);
        assert!(matches!(Record::Unknown, Record::Unknown));
    }

    #[test]
    fn durations_and_tokens_read_well() {
        assert_eq!(format_duration(1_234), "1,234 ms");
        assert_eq!(format_duration(12_345), "12.3 s");
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(1_250), "1.2k");
        assert_eq!(format_tokens(34_000), "34k");
        assert_eq!(format_tokens(1_100_000), "1.1M");
    }
}
