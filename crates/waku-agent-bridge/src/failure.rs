//! Fork addition: what a failed turn was, named for the desktop.
//!
//! A turn that gives up ends with a `ClaudeError` that keeps only the status
//! and the message. The class, the reason, the provider's code and the
//! request's id travel separately, as the engine's
//! `QueryEvent::RequestFailed`, which `session::forward_events` keeps. The
//! failure is named from that; reading the error's text is the fallback for
//! errors that never went through the retry policy. Guessing from the text
//! alone got the cases wrong that this exists for: an overflow reported
//! inside the stream, a stalled or cut-off reply, the gateway's per-minute
//! limit once its retries ran out.

use claurst_api::error_handling::is_context_overflow;
use claurst_api::retry_policy::{
    FailureClass, ModelFailure, RetryReason, classify_claude_error, classify_status, redact_secrets,
};
use claurst_core::error::ClaudeError;

use crate::events::FailureKind;
use crate::retry_rules::{GatewayVerdict, gateway_verdict};

/// The remedy-bearing kind of a failure that ended a turn, or `None` to show
/// it as it came.
///
/// The gateway's account verdicts — a balance, a spent plan, a disabled key —
/// keep their text: the gateway's bilingual message and the paywall's
/// buttons already say them better.
pub(crate) fn kind_of(failure: &ModelFailure) -> Option<FailureKind> {
    match gateway_verdict(&failure.message) {
        Some(GatewayVerdict::Account) => return None,
        Some(GatewayVerdict::ContentPolicy) => return Some(FailureKind::ContentPolicy),
        Some(GatewayVerdict::Transient) => {
            let per_minute = failure
                .message
                .to_ascii_lowercase()
                .contains("requests-per-minute");
            return Some(
                if failure.reason == RetryReason::RateLimited || per_minute {
                    FailureKind::RateLimited
                } else {
                    FailureKind::Unavailable
                },
            );
        }
        None => {}
    }
    match failure.class {
        FailureClass::ContextOverflow => Some(FailureKind::ContextOverflow),
        // A transient failure that ended a turn ran out of retries.
        FailureClass::Retryable => Some(match failure.reason {
            RetryReason::RateLimited => FailureKind::RateLimited,
            RetryReason::Overloaded | RetryReason::ServerError => FailureKind::Unavailable,
            RetryReason::Network | RetryReason::Timeout => FailureKind::Network,
            RetryReason::StreamInterrupted
            | RetryReason::Truncated
            | RetryReason::EmptyResponse => FailureKind::Interrupted,
        }),
        FailureClass::Fatal => fatal_kind(failure),
    }
}

/// The kind of a failure no retry could help.
fn fatal_kind(failure: &ModelFailure) -> Option<FailureKind> {
    let code = failure.code.as_deref().unwrap_or("");
    let text = failure.message.to_ascii_lowercase();
    let content_policy = matches!(
        code,
        "content_filter" | "content_policy_violation" | "content_policy" | "invalid_prompt"
    ) || [
        "content policy",
        "content filter",
        "content management policy",
        "usage policy",
        "safety system",
        "content exists risk",
    ]
    .iter()
    .any(|phrase| text.contains(phrase));
    if content_policy {
        return Some(FailureKind::ContentPolicy);
    }
    // A 429 whose wait was longer than a retry sits through is a quota
    // window, not congestion.
    let quota = matches!(
        code,
        "insufficient_quota" | "billing_error" | "billing_hard_limit_reached" | "quota_exceeded"
    ) || failure.status == Some(402)
        || failure.reason == RetryReason::RateLimited;
    if quota {
        return Some(FailureKind::Quota);
    }
    match failure.status {
        Some(401) => Some(FailureKind::Auth),
        Some(403) if code == "authentication_error" => Some(FailureKind::Auth),
        Some(403) => Some(FailureKind::Forbidden),
        Some(400 | 413 | 422) => Some(FailureKind::BadRequest),
        Some(500..=599) => Some(FailureKind::Unavailable),
        Some(_) => None,
        None => match code {
            "authentication_error" => Some(FailureKind::Auth),
            "permission_error" => Some(FailureKind::Forbidden),
            "invalid_request_error" | "request_too_large" => Some(FailureKind::BadRequest),
            // A rejected certificate: the route cannot be reached as set up.
            _ if failure.reason == RetryReason::Network => Some(FailureKind::Network),
            _ => None,
        },
    }
}

/// The kind of a failure that came without a [`ModelFailure`] — one that
/// never went through the retry policy. Read from the error itself, as
/// before the engine reported failures.
pub(crate) fn kind_of_error(error: &ClaudeError) -> Option<FailureKind> {
    match error {
        ClaudeError::ContextWindowExceeded => return Some(FailureKind::ContextOverflow),
        ClaudeError::RateLimit => return Some(FailureKind::RateLimited),
        ClaudeError::Auth(_) => return Some(FailureKind::Auth),
        ClaudeError::Http(_) => return Some(FailureKind::Network),
        ClaudeError::ApiStatus { status, message } => {
            return kind_of(&classify_status(*status, message, None, None));
        }
        _ => {}
    }
    let text = error.to_string();
    if gateway_verdict(&text).is_some() {
        return kind_of(&ModelFailure::fatal(None, text));
    }
    if is_context_overflow(&text) {
        return Some(FailureKind::ContextOverflow);
    }
    let lower = text.to_ascii_lowercase();
    let says = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    match classify_claude_error(error) {
        FailureClass::ContextOverflow => Some(FailureKind::ContextOverflow),
        FailureClass::Retryable if says(&["rate limit"]) => Some(FailureKind::RateLimited),
        FailureClass::Retryable if says(&["overloaded", "http 5"]) => {
            Some(FailureKind::Unavailable)
        }
        FailureClass::Retryable if says(&["ended before the response was complete", "stream"]) => {
            Some(FailureKind::Interrupted)
        }
        FailureClass::Retryable if says(&["network", "timed out", "connection"]) => {
            Some(FailureKind::Network)
        }
        _ => None,
    }
}

/// Whether a later attempt could get past the failure that ended a turn,
/// for a goal's automatic resume.
pub(crate) fn is_transient(failure: Option<&ModelFailure>, error: &ClaudeError) -> bool {
    match failure {
        Some(failure) => failure.class == FailureClass::Retryable,
        None => classify_claude_error(error) == FailureClass::Retryable,
    }
}

/// The failure as the person sees it, inside the remedy's wording or on its
/// own. Anything that looks like a credential is scrubbed, a second time for
/// errors that never went through the engine's classification. A refused
/// key's message shows only its status: a provider may echo the key it
/// refused, and the remedy already says what to do (after dsh's ui-chat,
/// which blanks AUTH failures; behaviour only).
pub(crate) fn shown_detail(
    kind: Option<FailureKind>,
    failure: Option<&ModelFailure>,
    summary: &str,
) -> String {
    if kind == Some(FailureKind::Auth) {
        if let Some(status) = failure.and_then(|failure| failure.status) {
            return format!("HTTP {status}");
        }
    }
    redact_secrets(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn failure(
        class: FailureClass,
        reason: RetryReason,
        status: Option<u16>,
        message: &str,
    ) -> ModelFailure {
        ModelFailure {
            class,
            reason,
            status,
            ..ModelFailure::fatal(status, message)
        }
    }

    fn coded(status: Option<u16>, code: &str, message: &str) -> ModelFailure {
        ModelFailure {
            code: Some(code.into()),
            ..ModelFailure::fatal(status, message)
        }
    }

    #[test]
    fn structured_failures_name_their_remedy() {
        use FailureClass::*;
        use RetryReason::*;
        let cases = [
            (
                failure(ContextOverflow, ServerError, None, "prompt is too long"),
                Some(FailureKind::ContextOverflow),
            ),
            (
                failure(Retryable, RateLimited, Some(429), "slow down"),
                Some(FailureKind::RateLimited),
            ),
            (
                failure(Retryable, Overloaded, Some(529), "Overloaded"),
                Some(FailureKind::Unavailable),
            ),
            (
                failure(Retryable, ServerError, Some(502), "Bad gateway"),
                Some(FailureKind::Unavailable),
            ),
            (
                failure(Retryable, Network, None, "network error: connection reset"),
                Some(FailureKind::Network),
            ),
            (
                coded(Some(400), "content_policy_violation", "flagged"),
                Some(FailureKind::ContentPolicy),
            ),
            (
                coded(None, "content_filter", "blocked"),
                Some(FailureKind::ContentPolicy),
            ),
            (
                coded(
                    Some(429),
                    "insufficient_quota",
                    "You exceeded your current quota",
                ),
                Some(FailureKind::Quota),
            ),
            (
                failure(Fatal, ServerError, Some(402), "Payment required"),
                Some(FailureKind::Quota),
            ),
            (
                ModelFailure {
                    retry_after: Some(Duration::from_secs(86_400)),
                    ..failure(Fatal, RateLimited, Some(429), "daily limit")
                },
                Some(FailureKind::Quota),
            ),
            (
                failure(Fatal, ServerError, Some(401), "unauthorized"),
                Some(FailureKind::Auth),
            ),
            (
                coded(Some(403), "authentication_error", "OAuth token revoked"),
                Some(FailureKind::Auth),
            ),
            (
                coded(Some(403), "permission_error", "no access to this model"),
                Some(FailureKind::Forbidden),
            ),
            (
                coded(None, "permission_error", "no access"),
                Some(FailureKind::Forbidden),
            ),
            (
                failure(
                    Fatal,
                    ServerError,
                    Some(400),
                    "unsupported parameter: reasoning",
                ),
                Some(FailureKind::BadRequest),
            ),
            (
                coded(None, "invalid_request_error", "bad tool schema"),
                Some(FailureKind::BadRequest),
            ),
            (
                failure(Fatal, ServerError, Some(503), "x-should-retry: false"),
                Some(FailureKind::Unavailable),
            ),
            (
                failure(Fatal, Network, None, "TLS error: invalid peer certificate"),
                Some(FailureKind::Network),
            ),
            (
                failure(Fatal, ServerError, None, "something else entirely"),
                None,
            ),
        ];
        for (failure, kind) in cases {
            assert_eq!(kind_of(&failure), kind, "{failure:?}");
        }
    }

    #[test]
    fn the_gateways_account_verdicts_keep_their_text() {
        for message in [
            "账户余额不足 / Insufficient account balance",
            "已超出每日用量上限 / daily usage limit exceeded",
            "Invalid API key",
        ] {
            assert_eq!(
                kind_of(&failure(
                    FailureClass::Fatal,
                    RetryReason::RateLimited,
                    Some(429),
                    message
                )),
                None
            );
            assert_eq!(
                kind_of(&failure(
                    FailureClass::Fatal,
                    RetryReason::ServerError,
                    Some(401),
                    message
                )),
                None
            );
        }
        assert_eq!(
            kind_of(&failure(
                FailureClass::Fatal,
                RetryReason::ServerError,
                Some(403),
                "请求被内容安全策略拦截 / Request blocked by content policy"
            )),
            Some(FailureKind::ContentPolicy)
        );
    }

    #[test]
    fn the_gateways_transient_verdicts_that_ran_out_are_named() {
        let per_minute = failure(
            FailureClass::Retryable,
            RetryReason::ServerError,
            Some(503),
            "用户每分钟请求数已达上限 / user requests-per-minute limit exceeded",
        );
        assert_eq!(kind_of(&per_minute), Some(FailureKind::RateLimited));
        let billing = failure(
            FailureClass::Retryable,
            RetryReason::ServerError,
            Some(503),
            "计费服务暂时不可用，请稍后重试 / Billing service temporarily unavailable. Please retry later.",
        );
        assert_eq!(kind_of(&billing), Some(FailureKind::Unavailable));
    }

    #[test]
    fn a_stalled_or_cut_off_reply_is_transient_and_named() {
        let stalled =
            ModelFailure::retryable(RetryReason::Timeout, "no data from the model for 1200 s");
        assert_eq!(kind_of(&stalled), Some(FailureKind::Network));
        assert!(is_transient(
            Some(&stalled),
            &stalled.clone().into_claude_error()
        ));

        let cut_off = ModelFailure::truncated("the response stream");
        assert_eq!(kind_of(&cut_off), Some(FailureKind::Interrupted));

        let internal =
            claurst_api::retry_policy::classify_stream_error("api_error", "Internal error");
        assert_eq!(kind_of(&internal), Some(FailureKind::Interrupted));
        // Read back from the flattened error alone, it used to look fatal.
        assert!(is_transient(
            Some(&internal),
            &internal.clone().into_claude_error()
        ));

        let overflow = claurst_api::retry_policy::classify_stream_error(
            "invalid_request_error",
            "prompt is too long: 210000 tokens > 200000 maximum",
        );
        assert_eq!(kind_of(&overflow), Some(FailureKind::ContextOverflow));
    }

    #[test]
    fn without_a_structured_failure_the_text_is_read_as_before() {
        let status = |status: u16, message: &str| ClaudeError::ApiStatus {
            status,
            message: message.to_string(),
        };
        assert_eq!(
            kind_of_error(&ClaudeError::RateLimit),
            Some(FailureKind::RateLimited)
        );
        assert_eq!(
            kind_of_error(&ClaudeError::ContextWindowExceeded),
            Some(FailureKind::ContextOverflow)
        );
        assert_eq!(
            kind_of_error(&status(401, "unauthorized")),
            Some(FailureKind::Auth)
        );
        assert_eq!(
            kind_of_error(&status(429, "slow down")),
            Some(FailureKind::RateLimited)
        );
        assert_eq!(
            kind_of_error(&status(503, "overloaded")),
            Some(FailureKind::Unavailable)
        );
        assert_eq!(
            kind_of_error(&status(403, "Insufficient account balance")),
            None
        );
        assert_eq!(
            kind_of_error(&status(429, "daily usage limit exceeded")),
            None
        );
        assert_eq!(
            kind_of_error(&status(400, "bad request")),
            Some(FailureKind::BadRequest)
        );
        assert_eq!(
            kind_of_error(&ClaudeError::Api(
                "prompt is too long: 210000 tokens > 200000 maximum".into()
            )),
            Some(FailureKind::ContextOverflow)
        );
        assert_eq!(
            kind_of_error(&ClaudeError::Api(
                "network error: connection refused".into()
            )),
            Some(FailureKind::Network)
        );
        assert_eq!(
            kind_of_error(&ClaudeError::Api(
                "No API key for provider 'openai' (model 'gpt-6').".into()
            )),
            None
        );
    }

    #[test]
    fn a_refused_key_never_shows_what_the_provider_echoed() {
        let refused = ModelFailure::fatal(
            Some(401),
            "Incorrect API key provided: sk-proj-abcdEFGH1234ijkl.",
        );
        let kind = kind_of(&refused);
        assert_eq!(kind, Some(FailureKind::Auth));
        let summary = refused.clone().into_claude_error().to_string();
        assert_eq!(shown_detail(kind, Some(&refused), &summary), "HTTP 401");
        // Any other failure keeps its words, scrubbed.
        let raw = "upstream said Authorization: Bearer abcDEF123456ghiJKL";
        let shown = shown_detail(None, None, raw);
        assert!(!shown.contains("abcDEF123456ghiJKL"), "{shown}");
        assert!(shown.starts_with("upstream said"), "{shown}");
    }
}
