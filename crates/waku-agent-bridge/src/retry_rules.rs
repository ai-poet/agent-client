//! Fork addition: the managed gateway's own verdicts on failed model
//! requests, for the engine's retry policy
//! (`claurst_api::retry_policy::set_failure_override`).
//!
//! The generic rules go by status code, and the gateway's statuses do not
//! always say what its messages do: a spent subscription answers 429 like a
//! busy upstream, and a balance rejection retried ten times is ten more
//! rejections and a minute of the user waiting for nothing. These read the
//! English half of the gateway's bilingual messages
//! (`backend/internal/pkg/bilingual/gateway_messages.go`), case-insensitively,
//! the way the desktop's paywall classifier reads them. A message that
//! mentions "upstream" is about a provider account behind the gateway, not
//! about this user, and keeps the generic verdict.

use claurst_api::retry_policy::FailureClass;

/// Rejections about this account that no retry can change. The desktop's
/// paywall reads the same text and offers the way out.
const ACCOUNT: &[&str] = &[
    "too many invalid authentication attempts",
    "invalid api key",
    "api key is required",
    "api key is disabled",
    "user associated with api key not found",
    "user account is not active",
    "group has been deleted",
    "group has been disabled",
    "exclusive group is no longer available",
    "not assigned to any group",
    "no active subscription found",
    "api key has expired",
    "api key quota exhausted",
    "insufficient account balance",
    "insufficient balance",
    "subscription has expired",
    "subscription is suspended",
    "subscription is invalid or expired",
    "daily usage limit exceeded",
    "weekly usage limit exceeded",
    "monthly usage limit exceeded",
    "usage_limit_exceeded",
    "5-hour limit exhausted",
    "api key daily limit exhausted",
    "7-day limit exhausted",
    "usage quota exhausted for this platform",
    "billing error",
];

/// Rejections of this request's content. No retry changes them either, but
/// rewording the request can.
const CONTENT_POLICY: &[&str] = &[
    "content audit matched a risk rule",
    "request blocked by content policy",
    "content moderation blocked this request",
    "prompt security audit rejected this request",
];

/// The gateway's own temporary failures, whatever status carried them.
const TRANSIENT: &[&str] = &[
    "api key authentication is temporarily unavailable",
    "failed to validate api key",
    "failed to maintain subscription usage windows",
    "billing service temporarily unavailable",
    "requests-per-minute limit exceeded",
    "prompt security audit is temporarily unavailable",
];

/// What the gateway said about a failure, when it said anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GatewayVerdict {
    /// This account cannot make the request: key, group, plan, balance.
    Account,
    /// The request's content was refused.
    ContentPolicy,
    /// The gateway's own temporary trouble.
    Transient,
}

/// The gateway's verdict on a failure's message, or `None` when it is not
/// one of the gateway's own (or is about an upstream account).
pub(crate) fn gateway_verdict(message: &str) -> Option<GatewayVerdict> {
    let text = message.to_ascii_lowercase();
    if text.contains("upstream") {
        return None;
    }
    let says = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if says(ACCOUNT) {
        Some(GatewayVerdict::Account)
    } else if says(CONTENT_POLICY) {
        Some(GatewayVerdict::ContentPolicy)
    } else if says(TRANSIENT) {
        Some(GatewayVerdict::Transient)
    } else {
        None
    }
}

/// The gateway's verdict on a failure, or `None` to keep the generic one.
pub fn gateway_failure_class(_status: Option<u16>, message: &str) -> Option<FailureClass> {
    gateway_verdict(message).map(|verdict| match verdict {
        GatewayVerdict::Account | GatewayVerdict::ContentPolicy => FailureClass::Fatal,
        GatewayVerdict::Transient => FailureClass::Retryable,
    })
}

/// Install the rules for this process. Idempotent.
pub fn install() {
    claurst_api::retry_policy::set_failure_override(gateway_failure_class);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_rejections_never_retry() {
        for message in [
            "账户余额不足 / Insufficient account balance",
            "已超出每日用量上限 / daily usage limit exceeded",
            "api key 5小时限额已用完 / API key 5-hour limit exhausted",
            "订阅无效或已过期 / subscription is invalid or expired",
            "该平台今日用量额度已用完 / Daily usage quota exhausted for this platform.",
            "请求被内容安全策略拦截 / Request blocked by content policy",
            r#"{"code":"USAGE_LIMIT_EXCEEDED"}"#,
        ] {
            assert_eq!(
                gateway_failure_class(Some(429), message),
                Some(FailureClass::Fatal),
                "{message}"
            );
        }
    }

    #[test]
    fn the_gateway_s_own_hiccups_retry() {
        for message in [
            "用户每分钟请求数已达上限 / user requests-per-minute limit exceeded",
            "计费服务暂时不可用，请稍后重试 / Billing service temporarily unavailable. Please retry later.",
            "提示词安全审计暂时不可用，请稍后重试 / Prompt security audit is temporarily unavailable. Please retry later.",
        ] {
            assert_eq!(
                gateway_failure_class(Some(503), message),
                Some(FailureClass::Retryable),
                "{message}"
            );
        }
    }

    #[test]
    fn upstream_messages_and_unknown_text_keep_the_generic_verdict() {
        assert_eq!(
            gateway_failure_class(Some(429), "upstream account has insufficient balance"),
            None
        );
        assert_eq!(gateway_failure_class(Some(502), "Bad gateway"), None);
    }

    #[test]
    fn content_policy_verdicts_are_told_apart_from_account_ones() {
        assert_eq!(
            gateway_verdict("请求被内容安全策略拦截 / Request blocked by content policy"),
            Some(GatewayVerdict::ContentPolicy)
        );
        assert_eq!(
            gateway_verdict(
                "Prompt security audit rejected this request. Please adjust your input and try again."
            ),
            Some(GatewayVerdict::ContentPolicy)
        );
        assert_eq!(
            gateway_verdict("账户余额不足 / Insufficient account balance"),
            Some(GatewayVerdict::Account)
        );
        assert_eq!(
            gateway_verdict("用户每分钟请求数已达上限 / user requests-per-minute limit exceeded"),
            Some(GatewayVerdict::Transient)
        );
        assert_eq!(gateway_verdict("Bad gateway"), None);
    }

    /// The needles are the gateway's English halves; if the table there is
    /// reworded, these must follow. Checked against a copy of the lines the
    /// rules depend on, so a rename shows up here as a failing test rather
    /// than as ten silent retries.
    #[test]
    fn needles_match_the_gateway_wording() {
        const GATEWAY_EN: &[&str] = &[
            "Too many invalid authentication attempts; retry later",
            "Invalid API key",
            "API key is disabled",
            "User account is not active. Sign in to the website to view notices or submit an appeal.",
            "The API key's group has been deleted",
            "No active subscription found for this group",
            "API key has expired",
            "API key quota exhausted",
            "Insufficient account balance",
            "subscription has expired",
            "subscription is suspended",
            "daily usage limit exceeded",
            "API key 5-hour limit exhausted",
            "API key daily limit exhausted",
            "API key 7-day limit exhausted",
            "Daily usage quota exhausted for this platform.",
            "insufficient balance",
            "Billing error",
            "Prompt security audit rejected this request. Please adjust your input and try again.",
        ];
        for line in GATEWAY_EN {
            assert_eq!(
                gateway_failure_class(None, line),
                Some(FailureClass::Fatal),
                "{line}"
            );
        }
    }
}
