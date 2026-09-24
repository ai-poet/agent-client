//! Which purchase, if any, would have let a failed turn through.
//!
//! The gateway refuses a request the account cannot pay for before it reaches
//! any upstream: `INSUFFICIENT_BALANCE` (403) on a balance group,
//! `USAGE_LIMIT_EXCEEDED` (429) once a subscription window is spent,
//! `SUBSCRIPTION_NOT_FOUND` / `SUBSCRIPTION_INVALID` (403) when the plan
//! lapsed. Past the concurrency wait the same refusals come back as a
//! `billing_error` whose message is `insufficient balance`, `daily usage limit
//! exceeded` or `subscription is invalid or expired`.
//!
//! Each driver surfaces that answer in its own words, and the turn keeps only
//! the text:
//!
//! * Claude Code — `API Error: 403 {raw body}`: the code survives.
//! * Codex — `unexpected status 403 …: {body}`: the code survives, except that
//!   a 429 is retried away into `exceeded retry limit, last status: 429`.
//! * the built-in agent, Anthropic route — `API error 403: {body}`.
//! * the built-in agent, OpenAI route — `Authentication failed: Insufficient
//!   account balance`, or a bare `Rate limited` for a 429.
//!
//! So the text is read first and the account's own state fills the gaps the
//! drivers leave. An upstream account's billing trouble (`Upstream payment
//! required: insufficient balance`) is the operator's, never the user's.

/// A purchase that would have let the turn through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paywall {
    /// The balance ran out: top up.
    Balance,
    /// A subscription window is spent: a bigger (or another) plan.
    PlanLimit,
    /// The subscription lapsed or never existed: renew.
    PlanInactive,
}

/// What the app knows about the account when the turn failed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaywallState {
    /// US dollars; `None` before the account loaded.
    pub balance: Option<f64>,
    pub has_active_subscription: bool,
    /// Some active subscription has a window with a limit it has reached.
    pub exhausted_subscription: bool,
}

/// Classify a failed turn's message.
pub fn classify(message: &str, state: &PaywallState) -> Option<Paywall> {
    let text = message.to_lowercase();
    if text.contains("upstream") {
        return None;
    }

    if text.contains("usage_limit_exceeded")
        || ["daily", "weekly", "monthly"]
            .iter()
            .any(|window| text.contains(&format!("{window} usage limit exceeded")))
    {
        return Some(Paywall::PlanLimit);
    }
    if text.contains("subscription_not_found")
        || text.contains("subscription_invalid")
        || text.contains("no active subscription found")
        || text.contains("subscription is invalid or expired")
    {
        return Some(Paywall::PlanInactive);
    }
    if text.contains("insufficient_balance")
        || text.contains("insufficient account balance")
        || text.contains("insufficient balance")
    {
        return Some(Paywall::Balance);
    }

    // The drivers that lost the code: judge by the account.
    if state.exhausted_subscription && (text.contains("429") || text.contains("rate limit")) {
        return Some(Paywall::PlanLimit);
    }
    if !state.has_active_subscription
        && state.balance.is_some_and(|balance| balance <= 0.0)
        && (text.contains("403") || text.contains("forbidden") || text.contains("authentication failed"))
    {
        return Some(Paywall::Balance);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUNDED: PaywallState = PaywallState {
        balance: Some(12.5),
        has_active_subscription: false,
        exhausted_subscription: false,
    };

    #[test]
    fn gateway_codes_classify_in_every_driver_wording() {
        let balance = [
            r#"API Error: 403 {"code":"INSUFFICIENT_BALANCE","message":"Insufficient account balance"}"#,
            r#"unexpected status 403 Forbidden: {"code":"INSUFFICIENT_BALANCE","message":"Insufficient account balance"}"#,
            r#"API error 403: {"error":{"type":"billing_error","message":"insufficient balance"}}"#,
            "[openai] Authentication failed: Insufficient account balance",
        ];
        for message in balance {
            assert_eq!(classify(message, &FUNDED), Some(Paywall::Balance), "{message}");
        }

        let limit = [
            r#"API Error: 429 {"code":"USAGE_LIMIT_EXCEEDED","message":"daily usage limit exceeded"}"#,
            r#"API error 403: {"error":{"type":"billing_error","message":"weekly usage limit exceeded"}}"#,
        ];
        for message in limit {
            assert_eq!(classify(message, &FUNDED), Some(Paywall::PlanLimit), "{message}");
        }

        let inactive = [
            r#"API Error: 403 {"code":"SUBSCRIPTION_NOT_FOUND","message":"No active subscription found for this group"}"#,
            r#"unexpected status 403 Forbidden: {"code":"SUBSCRIPTION_INVALID","message":"subscription is invalid or expired"}"#,
        ];
        for message in inactive {
            assert_eq!(classify(message, &FUNDED), Some(Paywall::PlanInactive), "{message}");
        }
    }

    #[test]
    fn upstream_billing_trouble_is_not_the_users() {
        let message = "API Error: 402 Upstream payment required: insufficient balance on the provider account";
        assert_eq!(classify(message, &FUNDED), None);
    }

    #[test]
    fn unrelated_quota_and_plain_rate_limits_do_not_classify() {
        let message = r#"API Error: 429 {"code":"USER_PLATFORM_DAILY_QUOTA_EXHAUSTED"}"#;
        assert_eq!(classify(message, &FUNDED), None);
        assert_eq!(
            classify("exceeded retry limit, last status: 429 Too Many Requests", &FUNDED),
            None
        );
        assert_eq!(classify("[openai] Rate limited", &FUNDED), None);
    }

    #[test]
    fn lost_codes_fall_back_to_the_account_state() {
        let spent = PaywallState {
            balance: Some(5.0),
            has_active_subscription: true,
            exhausted_subscription: true,
        };
        assert_eq!(
            classify("exceeded retry limit, last status: 429 Too Many Requests", &spent),
            Some(Paywall::PlanLimit)
        );
        assert_eq!(classify("[openai] Rate limited", &spent), Some(Paywall::PlanLimit));

        let broke = PaywallState {
            balance: Some(0.0),
            has_active_subscription: false,
            exhausted_subscription: false,
        };
        assert_eq!(
            classify("[openai] Authentication failed: Forbidden", &broke),
            Some(Paywall::Balance)
        );
        // A 403 with money on the account is something else entirely.
        assert_eq!(classify("[openai] Authentication failed: Forbidden", &FUNDED), None);
        // Unknown balance never guesses.
        assert_eq!(
            classify("403 Forbidden", &PaywallState::default()),
            None
        );
    }
}
