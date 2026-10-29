//! Fork addition: what kind of failure ended a built-in agent turn, so the
//! desktop can offer the action that fixes it, and why a model request is
//! being retried, so the working row can say so instead of a generic
//! "reconnecting".

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What kind of failure ended a turn, when the agent could name it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TurnFailureKind {
    /// The conversation no longer fits the model's context window.
    ContextOverflow,
    /// The route kept answering "too many requests" until the retries ran out.
    RateLimited,
    /// The route could not be reached, or stopped responding.
    Network,
    /// The route refused the key.
    Auth,
    /// The model's service kept failing (overloaded, 5xx).
    Unavailable,
    /// The request was refused under a content policy.
    ContentPolicy,
    /// The provider says the account's quota or credit is spent.
    Quota,
    /// The provider refused this key this request (no access to the model,
    /// a restricted account).
    Forbidden,
    /// The request was rejected as invalid — usually a model that does not
    /// support the API or an option it was sent with.
    BadRequest,
    /// The reply kept breaking off before it was complete.
    Interrupted,
    /// A kind a newer build wrote that this one does not know. Read as "no
    /// kind": the session it is stored in must still load. Never sent.
    #[serde(other)]
    #[ts(skip)]
    Unknown,
}

/// Sent just before the failure's message, when the agent named the
/// failure. Carries what the message's text cannot be relied on for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFailure {
    pub kind: TurnFailureKind,
    /// The failed request's id, when the server sent one.
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Why a model request is being retried, in the terms the person sees.
/// Several of the engine's reasons read the same to them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryCause {
    RateLimited,
    /// Overloaded, or a server error.
    Busy,
    /// The network, or a timeout.
    Connection,
    /// The stream broke off or ended before it was complete.
    CutOff,
    /// The model finished without saying anything.
    Empty,
    Other,
}

impl RetryCause {
    /// From `ModelRetry::reason` (`claurst_api::retry_policy::RetryReason::as_str`).
    pub fn from_reason(reason: &str) -> Self {
        match reason {
            "rate_limited" => Self::RateLimited,
            "overloaded" | "server_error" => Self::Busy,
            "network" | "timeout" => Self::Connection,
            "stream_interrupted" | "truncated" => Self::CutOff,
            "empty_response" => Self::Empty,
            _ => Self::Other,
        }
    }

    /// A short name for the cause: "Rate limited", "Service busy", …
    pub fn label(self) -> String {
        match self {
            Self::RateLimited => tr!("turn_status.retry_cause.rate_limited"),
            Self::Busy => tr!("turn_status.retry_cause.busy"),
            Self::Connection => tr!("turn_status.retry_cause.connection"),
            Self::CutOff => tr!("turn_status.retry_cause.cut_off"),
            Self::Empty => tr!("turn_status.retry_cause.empty"),
            Self::Other => tr!("turn_status.retry_cause.other"),
        }
    }

    /// What happened to a reply that had started streaming when it failed,
    /// as the opening of the line that says it was taken back.
    pub fn discarded_lead(self) -> String {
        match self {
            Self::RateLimited => tr!("session.stream_discarded_lead.rate_limited"),
            Self::Busy => tr!("session.stream_discarded_lead.busy"),
            Self::Connection => tr!("session.stream_discarded_lead.connection"),
            Self::CutOff => tr!("session.stream_discarded_lead.cut_off"),
            Self::Empty | Self::Other => tr!("session.stream_discarded_lead.other"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_reasons_fall_into_causes() {
        assert_eq!(
            RetryCause::from_reason("rate_limited"),
            RetryCause::RateLimited
        );
        assert_eq!(RetryCause::from_reason("overloaded"), RetryCause::Busy);
        assert_eq!(RetryCause::from_reason("server_error"), RetryCause::Busy);
        assert_eq!(RetryCause::from_reason("network"), RetryCause::Connection);
        assert_eq!(RetryCause::from_reason("timeout"), RetryCause::Connection);
        assert_eq!(
            RetryCause::from_reason("stream_interrupted"),
            RetryCause::CutOff
        );
        assert_eq!(RetryCause::from_reason("truncated"), RetryCause::CutOff);
        assert_eq!(RetryCause::from_reason("empty_response"), RetryCause::Empty);
        assert_eq!(RetryCause::from_reason("turn_failed"), RetryCause::Other);
        assert_eq!(RetryCause::from_reason(""), RetryCause::Other);
    }
    #[test]
    fn a_turn_error_without_a_kind_still_reads() {
        let error: crate::model::TurnError =
            serde_json::from_str(r#"{"message":"the provider exited","dismissed":false}"#).unwrap();
        assert_eq!(error.kind, None);
        // And none is written back, so an older build reads it unchanged.
        assert!(!serde_json::to_string(&error).unwrap().contains("kind"));
    }

    #[test]
    fn an_unknown_failure_kind_still_reads() {
        let error: crate::model::TurnError =
            serde_json::from_str(r#"{"message":"x","dismissed":false,"kind":"somethingNew"}"#)
                .unwrap();
        assert_eq!(error.kind, Some(TurnFailureKind::Unknown));
        let known: crate::model::TurnError =
            serde_json::from_str(r#"{"message":"x","dismissed":false,"kind":"badRequest"}"#)
                .unwrap();
        assert_eq!(known.kind, Some(TurnFailureKind::BadRequest));
    }
}
