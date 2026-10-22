//! Fork departure (Waku): one policy for every model route — which failures
//! are worth another attempt, how long to wait first, and how many attempts
//! a request gets.
//!
//! The engine's three routes each did their own thing: the Messages client
//! retried 429 / 529 only, the Responses and Chat Completions adapters never
//! retried an HTTP error at all, and the query loop shared two stream retries
//! across a whole prompt. A long task on an unsteady network died on the
//! first blip. The adapters now make single attempts and report a
//! [`ModelFailure`]; the query loop owns the retrying, so it can be cancelled
//! and can tell the user what it is waiting for.
//!
//! The shape follows ZCode's model runner (Apache-2.0; design only, no code
//! is used): exponential backoff from 2 s capped at 60 s with jitter,
//! `Retry-After` honoured up to five minutes, `x-should-retry: false`
//! respected, and a request whose answer had started streaming recovered by
//! sending it again rather than kept half-finished.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use claurst_core::error::ClaudeError;
use once_cell::sync::OnceCell;

use crate::error_handling::is_context_overflow;

/// The longest server-requested wait that is still worth sitting through. A
/// longer one is a quota reset, not a hiccup, and fails at once with the
/// server's own message.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

const BASE_DELAY: Duration = Duration::from_secs(2);
const MAX_DELAY: Duration = Duration::from_secs(60);

/// What to do about a failed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Transient: the same request may well succeed shortly.
    Retryable,
    /// The conversation no longer fits the model's window. Sending it again
    /// unchanged cannot help; compacting first can.
    ContextOverflow,
    /// Retrying cannot help: credentials, balance, a malformed request, a
    /// cancelled one.
    Fatal,
}

impl FailureClass {
    /// A stable identifier, for logs and the trajectory.
    pub fn as_str(self) -> &'static str {
        match self {
            FailureClass::Retryable => "retryable",
            FailureClass::ContextOverflow => "context_overflow",
            FailureClass::Fatal => "fatal",
        }
    }
}

/// Why a retryable failure happened, for the notice the user sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryReason {
    Network,
    Timeout,
    RateLimited,
    Overloaded,
    ServerError,
    /// The response stream broke off, or the server reported an error in it.
    StreamInterrupted,
    /// The stream ended without its terminal event.
    Truncated,
    /// The model finished without text, reasoning or a tool call.
    EmptyResponse,
}

impl RetryReason {
    /// A stable identifier, for logs and for clients that word it themselves.
    pub fn as_str(self) -> &'static str {
        match self {
            RetryReason::Network => "network",
            RetryReason::Timeout => "timeout",
            RetryReason::RateLimited => "rate_limited",
            RetryReason::Overloaded => "overloaded",
            RetryReason::ServerError => "server_error",
            RetryReason::StreamInterrupted => "stream_interrupted",
            RetryReason::Truncated => "truncated",
            RetryReason::EmptyResponse => "empty_response",
        }
    }
}

/// A failed model request, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFailure {
    pub class: FailureClass,
    /// Meaningful when `class` is [`FailureClass::Retryable`].
    pub reason: RetryReason,
    /// The HTTP status, when the server answered with one.
    pub status: Option<u16>,
    /// The server's own words where it gave any — the gateway's bilingual
    /// messages included, which the desktop classifies by substring — or a
    /// description of the transport failure.
    pub message: String,
    /// How long the server asked to wait, when it said.
    pub retry_after: Option<Duration>,
    /// The provider's own type or code for the error, lower-cased —
    /// `insufficient_quota`, `content_policy_violation`, `permission_error` —
    /// when it gave one. Names a failure the status alone does not.
    pub code: Option<String>,
    /// The server's id for the request: what support looks a failure up by.
    /// The managed gateway keys its error log on `X-Request-ID`.
    pub request_id: Option<String>,
}

impl ModelFailure {
    /// The constructors scrub the message ([`redact_secrets`]): a provider
    /// that echoes the key it refused must not have it shown, logged or
    /// written into the trajectory.
    pub fn retryable(reason: RetryReason, message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Retryable,
            reason,
            status: None,
            message: redact_secrets(&message.into()),
            retry_after: None,
            code: None,
            request_id: None,
        }
    }

    pub fn fatal(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Fatal,
            reason: RetryReason::ServerError,
            status,
            message: redact_secrets(&message.into()),
            retry_after: None,
            code: None,
            request_id: None,
        }
    }

    pub fn context_overflow(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::ContextOverflow,
            reason: RetryReason::ServerError,
            status,
            message: redact_secrets(&message.into()),
            retry_after: None,
            code: None,
            request_id: None,
        }
    }

    /// The request's id, unless the failure already carries one.
    pub fn with_request_id(mut self, id: Option<String>) -> Self {
        if self.request_id.is_none() {
            self.request_id = id;
        }
        self
    }

    fn with_code(mut self, code: Option<String>) -> Self {
        if self.code.is_none() {
            self.code = code.filter(|code| !code.is_empty());
        }
        self
    }

    /// The stream ended before its terminal event (`message_stop`,
    /// `response.completed`, `[DONE]` / a finish reason).
    pub fn truncated(what: &str) -> Self {
        Self::retryable(
            RetryReason::Truncated,
            format!("{what} ended before the response was complete"),
        )
    }

    pub fn empty_response() -> Self {
        Self::retryable(
            RetryReason::EmptyResponse,
            "the model finished without text, reasoning or a tool call",
        )
    }

    pub fn is_retryable(&self) -> bool {
        self.class == FailureClass::Retryable
    }

    /// The error a turn that gave up on this failure ends with. The status
    /// and the server's message survive, so the desktop's paywall and 404
    /// handling still recognise them.
    pub fn into_claude_error(self) -> ClaudeError {
        match self.status {
            Some(status) => ClaudeError::ApiStatus {
                status,
                message: self.message,
            },
            None => ClaudeError::Api(self.message),
        }
    }
}

impl std::fmt::Display for ModelFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(f, "HTTP {status}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway rules
// ---------------------------------------------------------------------------

/// A product-specific verdict on a failure the generic rules would get
/// wrong — a balance rejection worded as a 429, say. `None` defers to them.
pub type FailureOverride = fn(status: Option<u16>, message: &str) -> Option<FailureClass>;

static FAILURE_OVERRIDE: OnceCell<FailureOverride> = OnceCell::new();

/// Install the product's rules. Once per process; later calls are ignored.
pub fn set_failure_override(rules: FailureOverride) {
    let _ = FAILURE_OVERRIDE.set(rules);
}

fn overridden(status: Option<u16>, message: &str) -> Option<FailureClass> {
    FAILURE_OVERRIDE.get().and_then(|rules| rules(status, message))
}

/// `failure` with the product's verdict applied, when it has one.
fn with_override(mut failure: ModelFailure) -> ModelFailure {
    if let Some(class) = overridden(failure.status, &failure.message) {
        failure.class = class;
    }
    failure
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify an HTTP error response. Headers are read for `Retry-After`
/// (`retry-after-ms`, seconds or an HTTP date) and `x-should-retry`.
pub fn classify_http(status: u16, headers: &reqwest::header::HeaderMap, body: &str) -> ModelFailure {
    let retry_after = parse_retry_after(headers, SystemTime::now());
    let should_retry = headers
        .get("x-should-retry")
        .and_then(|value| value.to_str().ok())
        .map(|value| !matches!(value.trim().to_ascii_lowercase().as_str(), "false" | "0"));
    classify_status(status, body, retry_after, should_retry).with_request_id(request_id_of(headers))
}

/// The server's id for a request, from the response headers: the gateway's
/// `X-Request-ID` first — its error log is keyed on it — then the names
/// Anthropic and DeepSeek use. Only a short run of visible ASCII is kept.
pub fn request_id_of(headers: &reqwest::header::HeaderMap) -> Option<String> {
    ["x-request-id", "request-id", "x-deepseek-request-id"]
        .iter()
        .filter_map(|name| headers.get(*name)?.to_str().ok())
        .map(str::trim)
        .find(|id| !id.is_empty() && id.len() <= 128 && id.bytes().all(|byte| byte.is_ascii_graphic()))
        .map(str::to_string)
}

/// [`classify_http`] once the headers have been read.
pub fn classify_status(
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
    should_retry: Option<bool>,
) -> ModelFailure {
    let message = error_message_from_body(body).unwrap_or_else(|| match status {
        429 => "rate limited".to_string(),
        529 => "overloaded".to_string(),
        _ => format!("the server answered HTTP {status}"),
    });
    let code = error_code_from_body(body);
    if matches!(status, 400 | 413 | 422) && (names_context_overflow(&message) || names_context_overflow(body)) {
        return ModelFailure::context_overflow(Some(status), message).with_code(code);
    }

    let reason = match status {
        408 => Some(RetryReason::Timeout),
        429 => Some(RetryReason::RateLimited),
        529 => Some(RetryReason::Overloaded),
        409 | 425 => Some(RetryReason::ServerError),
        501 | 505 => None,
        500..=599 => Some(RetryReason::ServerError),
        _ => None,
    };
    let mut failure = match reason {
        Some(reason) => ModelFailure {
            status: Some(status),
            retry_after,
            ..ModelFailure::retryable(reason, message)
        },
        None => ModelFailure {
            retry_after,
            ..ModelFailure::fatal(Some(status), message)
        },
    }
    .with_code(code);
    if should_retry == Some(false) {
        failure.class = FailureClass::Fatal;
    }
    // OpenAI's spent credit comes as a 429; waiting does not refill it.
    if matches!(
        failure.code.as_deref(),
        Some("insufficient_quota" | "billing_hard_limit_reached")
    ) {
        failure.class = FailureClass::Fatal;
    }
    // A wait this long is a quota window, not congestion.
    if failure.is_retryable() && retry_after.is_some_and(|after| after > MAX_RETRY_AFTER) {
        failure.class = FailureClass::Fatal;
    }
    with_override(failure)
}

/// Classify a failure to get any response, or to read the rest of one.
pub fn classify_transport(error: &reqwest::Error) -> ModelFailure {
    classify_transport_detail(&error_chain(error), error.is_timeout())
}

/// [`classify_transport`] once the error is text.
fn classify_transport_detail(detail: &str, is_timeout: bool) -> ModelFailure {
    let lower = detail.to_ascii_lowercase();
    // A rejected certificate stays rejected; asking again only delays the
    // message that says so.
    if lower.contains("certificate") || lower.contains("unknownissuer") {
        return ModelFailure {
            reason: RetryReason::Network,
            ..ModelFailure::fatal(None, format!("TLS error: {detail}"))
        };
    }
    if is_timeout || lower.contains("timed out") {
        return ModelFailure::retryable(RetryReason::Timeout, format!("timed out: {detail}"));
    }
    ModelFailure::retryable(RetryReason::Network, format!("network error: {detail}"))
}

/// Classify an error the server reported inside a response stream — an SSE
/// `error` event, `response.failed`, an `{"error": …}` chunk. `kind` is its
/// type or code, when it carried one.
pub fn classify_stream_error(kind: &str, message: &str) -> ModelFailure {
    let kind = kind.trim().to_ascii_lowercase();
    let code = Some(kind.clone());
    if names_context_overflow(message) || kind.contains("context_length") || kind.contains("context_window") {
        return ModelFailure::context_overflow(None, message).with_code(code);
    }
    let lower = message.to_ascii_lowercase();
    let failure = match kind.as_str() {
        "authentication_error" | "permission_error" | "billing_error" | "invalid_request_error"
        | "not_found_error" | "insufficient_quota" | "invalid_prompt" | "content_filter"
        | "content_policy_violation" | "request_too_large" => ModelFailure::fatal(None, message),
        "rate_limit_error" | "rate_limit_exceeded" | "rate_limited" => {
            ModelFailure::retryable(RetryReason::RateLimited, message)
        }
        "overloaded_error" | "overloaded" => ModelFailure::retryable(RetryReason::Overloaded, message),
        _ if lower.contains("timed out") || lower.contains("timeout") => {
            ModelFailure::retryable(RetryReason::Timeout, message)
        }
        _ => ModelFailure::retryable(RetryReason::StreamInterrupted, message),
    };
    with_override(failure.with_code(code))
}

/// Classify whatever a provider adapter returned.
pub fn from_provider_error(error: &crate::provider_error::ProviderError) -> ModelFailure {
    use crate::provider_error::ProviderError as E;
    match error {
        E::Classified { failure, .. } => failure.clone(),
        E::ContextOverflow { message, .. } => ModelFailure::context_overflow(None, message.clone()),
        E::RateLimited { retry_after, .. } => {
            let retry_after = retry_after.map(Duration::from_secs);
            let mut failure = ModelFailure {
                status: Some(429),
                retry_after,
                ..ModelFailure::retryable(RetryReason::RateLimited, error.to_string())
            };
            if retry_after.is_some_and(|after| after > MAX_RETRY_AFTER) {
                failure.class = FailureClass::Fatal;
            }
            with_override(failure)
        }
        E::ServerError {
            status,
            message,
            is_retryable,
            ..
        } => {
            let failure = if *is_retryable {
                ModelFailure {
                    status: *status,
                    ..ModelFailure::retryable(RetryReason::ServerError, message.clone())
                }
            } else {
                ModelFailure::fatal(*status, message.clone())
            };
            with_override(failure)
        }
        E::StreamError { message, .. } => {
            let lower = message.to_ascii_lowercase();
            let reason = if lower.contains("timed out") || lower.contains("stalled") {
                RetryReason::Timeout
            } else {
                RetryReason::StreamInterrupted
            };
            with_override(ModelFailure::retryable(reason, message.clone()))
        }
        E::Other {
            status: Some(status),
            body,
            message,
            ..
        } => classify_status(*status, body.as_deref().unwrap_or(message), None, None),
        E::Other { message, .. } => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("timed out") {
                ModelFailure::retryable(RetryReason::Timeout, message.clone())
            } else if lower.contains("http request failed") || lower.contains("connect") {
                ModelFailure::retryable(RetryReason::Network, message.clone())
            } else {
                with_override(ModelFailure::fatal(None, message.clone()))
            }
        }
        // The variant names the failure; the code says so to the client.
        E::AuthFailed { message, .. } => {
            with_override(ModelFailure::fatal(None, message.clone()).with_code(Some("authentication_error".into())))
        }
        E::QuotaExceeded { message, .. } => {
            with_override(ModelFailure::fatal(None, message.clone()).with_code(Some("insufficient_quota".into())))
        }
        E::InvalidRequest { message, .. } => {
            with_override(ModelFailure::fatal(None, message.clone()).with_code(Some("invalid_request_error".into())))
        }
        E::ContentFiltered { message, .. } => {
            with_override(ModelFailure::fatal(None, message.clone()).with_code(Some("content_filter".into())))
        }
        E::ModelNotFound { .. } => ModelFailure::fatal(Some(404), error.to_string()),
    }
}

/// Classify the error a turn ended with, after the fact — whether it was
/// the kind a later attempt could get past. Used to decide whether an
/// autonomous run should try again on its own.
pub fn classify_claude_error(error: &ClaudeError) -> FailureClass {
    match error {
        ClaudeError::Cancelled
        | ClaudeError::Auth(_)
        | ClaudeError::PermissionDenied(_)
        | ClaudeError::Config(_) => FailureClass::Fatal,
        ClaudeError::ContextWindowExceeded => FailureClass::ContextOverflow,
        ClaudeError::RateLimit => FailureClass::Retryable,
        ClaudeError::Http(error) => classify_transport(error).class,
        ClaudeError::ApiStatus { status, message } => classify_status(*status, message, None, None).class,
        ClaudeError::Api(message) | ClaudeError::Other(message) => {
            if let Some(class) = overridden(None, message) {
                return class;
            }
            let lower = message.to_ascii_lowercase();
            const TRANSIENT: &[&str] = &[
                "network error",
                "timed out",
                "connection",
                "overloaded",
                "rate limit",
                "temporarily",
                "stream",
                "ended before the response was complete",
                "http 5",
            ];
            if TRANSIENT.iter().any(|word| lower.contains(word)) {
                FailureClass::Retryable
            } else {
                FailureClass::Fatal
            }
        }
        _ => FailureClass::Fatal,
    }
}

/// [`is_context_overflow`], minus the one pattern a CDN's body-size limit
/// also produces: an HTML "413 Request Entity Too Large" says nothing about
/// tokens, and compacting the conversation would not shrink an image.
fn names_context_overflow(text: &str) -> bool {
    is_context_overflow(text) && !text.to_ascii_lowercase().contains("request entity too large")
}

/// The human-readable message out of an error body, wherever the provider
/// put it. `None` for an empty body; an HTML page is summarised.
fn error_message_from_body(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        for path in ["/error/message", "/error/error/message", "/message", "/detail", "/error"] {
            if let Some(message) = json.pointer(path).and_then(|value| value.as_str()) {
                if !message.trim().is_empty() {
                    return Some(message.to_string());
                }
            }
        }
        return Some(trimmed.to_string());
    }
    if trimmed.starts_with('<') {
        return Some(
            "the server answered with an HTML error page (a proxy or CDN in between)".to_string(),
        );
    }
    Some(trimmed.chars().take(2_000).collect())
}

/// The provider's type or code for an error, from a JSON body: Anthropic's
/// `error.type`, OpenAI's `error.code` (falling back to its `error.type`),
/// Gemini's `error.status`. Lower-cased; `None` without one.
fn error_code_from_body(body: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    ["/error/code", "/error/type", "/error/status", "/code", "/type"]
        .iter()
        .filter_map(|path| json.pointer(path)?.as_str())
        .map(str::trim)
        .find(|code| !code.is_empty())
        .map(str::to_ascii_lowercase)
}

/// `text` with anything that looks like a credential replaced by
/// `[redacted]`: OpenAI-style `sk-` keys (masked echoes included), bearer
/// tokens, `x-api-key` / `authorization` / `api_key` assignments, `key=` and
/// `token=` in a URL, Google API keys, JWTs, and long runs that mix upper
/// case, lower case and digits the way generated secrets do. Hex hashes and
/// UUIDs — the gateway's `（hash: …）`, request ids — are left alone.
/// Applying it twice changes nothing.
pub fn redact_secrets(text: &str) -> String {
    use regex::{Captures, Regex};
    use std::sync::LazyLock;

    static OPENAI_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bsk-[A-Za-z0-9_*\-]{8,}").unwrap());
    static BEARER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(bearer)\s+[A-Za-z0-9._~+/=\-]{8,}").unwrap());
    static ASSIGNED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\b(x-api-key|x-goog-api-key|api[_-]?key|authorization)(["']?\s*[:=]\s*["']?)([^\s"',}&]+)"#,
        )
        .unwrap()
    });
    static QUERY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"([?&](?:key|api_key|access_token|token)=)[^&\s"']+"#).unwrap());
    static GOOGLE_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"AIza[0-9A-Za-z_\-]{35}").unwrap());
    static JWT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}").unwrap()
    });
    static LONG_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9_\-]{40,}").unwrap());
    const REDACTED: &str = "[redacted]";

    let text = OPENAI_KEY.replace_all(text, "sk-[redacted]");
    let text = BEARER.replace_all(&text, "$1 [redacted]");
    let text = ASSIGNED.replace_all(&text, |caps: &Captures| {
        let value = &caps[3];
        // `Authorization: Bearer …` was handled above; keep its scheme.
        if value.eq_ignore_ascii_case("bearer") || value.starts_with(REDACTED) {
            caps[0].to_string()
        } else {
            format!("{}{}{REDACTED}", &caps[1], &caps[2])
        }
    });
    let text = QUERY.replace_all(&text, "${1}[redacted]");
    let text = GOOGLE_KEY.replace_all(&text, REDACTED);
    let text = JWT.replace_all(&text, REDACTED);
    let text = LONG_RUN.replace_all(&text, |caps: &Captures| {
        let run = &caps[0];
        let mixed = run.bytes().any(|byte| byte.is_ascii_uppercase())
            && run.bytes().any(|byte| byte.is_ascii_lowercase())
            && run.bytes().any(|byte| byte.is_ascii_digit());
        if mixed { REDACTED.to_string() } else { run.to_string() }
    });
    text.into_owned()
}

/// The error and everything that caused it, in one line.
fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

/// How long the server asked to wait: `retry-after-ms`, then `retry-after`
/// as seconds or an HTTP date.
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap, now: SystemTime) -> Option<Duration> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).map(str::trim);
    if let Some(ms) = header("retry-after-ms").and_then(|value| value.parse::<f64>().ok()) {
        if ms.is_finite() && ms >= 0.0 {
            return Some(Duration::from_secs_f64(ms / 1000.0));
        }
    }
    let value = header("retry-after")?;
    if let Ok(seconds) = value.parse::<f64>() {
        return (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(date.timestamp()).ok()?);
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

// ---------------------------------------------------------------------------
// Budgets and backoff
// ---------------------------------------------------------------------------

/// How persistent a run is about transient failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RetryBudget {
    /// An interactive session: ten retries per request, ten recoveries of a
    /// half-streamed answer per prompt.
    #[default]
    Default,
    /// Work nobody is watching (an AgentTeams member): transient failures
    /// are retried until they stop or the run is cancelled — with a cap on
    /// back-to-back server errors so a permanently broken upstream still
    /// ends it.
    Unbounded,
    /// A quick side request — a title, a summary for compaction: a few
    /// short retries, no recovery mid-answer.
    Light,
    /// One attempt, for work that has a cheaper fallback than waiting.
    SingleAttempt,
}

/// Server errors in a row an [`RetryBudget::Unbounded`] run sits through.
pub const UNBOUNDED_SERVER_ERROR_LIMIT: u32 = 30;

impl RetryBudget {
    /// Retries one request may make after its first attempt; `None` is no
    /// limit.
    pub fn retries_per_request(self) -> Option<u32> {
        match self {
            RetryBudget::Default => Some(10),
            RetryBudget::Unbounded => None,
            RetryBudget::Light => Some(3),
            RetryBudget::SingleAttempt => Some(0),
        }
    }

    /// Answers that started streaming and broke off which may be thrown away
    /// and asked again, per prompt; `None` is no limit.
    pub fn recoveries_per_prompt(self) -> Option<u32> {
        match self {
            RetryBudget::Default => Some(10),
            RetryBudget::Unbounded => None,
            RetryBudget::Light | RetryBudget::SingleAttempt => Some(0),
        }
    }

    pub fn max_delay(self) -> Duration {
        match self {
            RetryBudget::Light => Duration::from_secs(15),
            _ => MAX_DELAY,
        }
    }
}

/// How long to wait before retry number `retry` (1-based): what the server
/// asked for when it asked for a reasonable wait, otherwise 2 s doubling to
/// the budget's cap, scaled by a random factor in [0.5, 1.0] so clients that
/// failed together do not come back together.
pub fn backoff_delay(budget: RetryBudget, retry: u32, retry_after: Option<Duration>) -> Duration {
    if let Some(after) = retry_after.filter(|after| *after <= MAX_RETRY_AFTER) {
        return after;
    }
    let doublings = retry.saturating_sub(1).min(16);
    let exponential = BASE_DELAY.saturating_mul(1u32 << doublings);
    let capped = exponential.min(budget.max_delay());
    capped.mul_f64(0.5 + 0.5 * random_unit())
}

/// A uniform value in [0, 1), from the v4 UUID generator the crate already
/// links rather than a dependency of its own.
fn random_unit() -> f64 {
    let bits = uuid::Uuid::new_v4().as_u128() >> 75; // 53 random-enough bits
    bits as f64 / (1u64 << 53) as f64
}

// ---------------------------------------------------------------------------
// Network timeouts
// ---------------------------------------------------------------------------

/// Defaults when nothing configured them.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

static CONNECT_TIMEOUT_MS: AtomicU64 = AtomicU64::new(15_000);
static IDLE_TIMEOUT_MS: AtomicU64 = AtomicU64::new(300_000);

/// Set the timeouts every model client is built with: how long a connection
/// may take, and how long a response may go without a single byte. A zero
/// restores the default. Takes effect for clients built afterwards.
pub fn set_network_timeouts(connect: Duration, idle: Duration) {
    let millis = |value: Duration, default: Duration| {
        let value = if value.is_zero() { default } else { value };
        u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
    };
    CONNECT_TIMEOUT_MS.store(millis(connect, DEFAULT_CONNECT_TIMEOUT), Ordering::Relaxed);
    IDLE_TIMEOUT_MS.store(millis(idle, DEFAULT_IDLE_TIMEOUT), Ordering::Relaxed);
}

pub fn connect_timeout() -> Duration {
    Duration::from_millis(CONNECT_TIMEOUT_MS.load(Ordering::Relaxed))
}

pub fn idle_timeout() -> Duration {
    Duration::from_millis(IDLE_TIMEOUT_MS.load(Ordering::Relaxed))
}

/// A client builder for model requests. No total deadline — a healthy
/// answer may stream for well over ten minutes — but a connect timeout and
/// a read timeout that every byte resets, SSE keep-alive comments included,
/// so a dead connection is noticed within [`idle_timeout`].
pub fn model_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(connect_timeout())
        .read_timeout(idle_timeout())
        .tcp_keepalive(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).expect("header"));
        }
        map
    }

    #[test]
    fn statuses_split_into_retry_compact_and_fail() {
        let none = HeaderMap::new();
        let class = |status: u16, body: &str| classify_http(status, &none, body).class;
        for status in [408, 409, 425, 429, 500, 502, 503, 504, 520, 522, 524, 529] {
            assert_eq!(class(status, ""), FailureClass::Retryable, "{status}");
        }
        for status in [400, 401, 402, 403, 404, 422, 501] {
            assert_eq!(class(status, "{}"), FailureClass::Fatal, "{status}");
        }
        assert_eq!(
            class(400, r#"{"error":{"message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#),
            FailureClass::ContextOverflow
        );
        assert_eq!(
            class(400, r#"{"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens"}}"#),
            FailureClass::ContextOverflow
        );
        // A CDN's body limit is not a context window.
        assert_eq!(
            class(413, "<html><title>413 Request Entity Too Large</title></html>"),
            FailureClass::Fatal
        );
    }

    #[test]
    fn the_server_message_survives_classification() {
        let failure = classify_http(
            403,
            &HeaderMap::new(),
            r#"{"error":{"type":"billing_error","message":"余额不足 / Insufficient balance"}}"#,
        );
        assert_eq!(failure.message, "余额不足 / Insufficient balance");
        assert_eq!(
            failure.clone().into_claude_error().to_string(),
            "API error 403: 余额不足 / Insufficient balance"
        );
    }

    #[test]
    fn retry_after_comes_in_three_shapes() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after-ms", "1500")]), now),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", "7")]), now),
            Some(Duration::from_secs(7))
        );
        // Tue, 14 Nov 2023 22:13:30 GMT is 10 s after `now`.
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", "Tue, 14 Nov 2023 22:13:30 GMT")]), now),
            Some(Duration::from_secs(10))
        );
        assert_eq!(parse_retry_after(&headers(&[("retry-after", "soon")]), now), None);
        assert_eq!(parse_retry_after(&HeaderMap::new(), now), None);
    }

    #[test]
    fn a_long_retry_after_or_no_retry_header_fails_at_once() {
        let soon = classify_http(429, &headers(&[("retry-after", "3")]), "{}");
        assert_eq!(soon.class, FailureClass::Retryable);
        assert_eq!(soon.retry_after, Some(Duration::from_secs(3)));

        let quota = classify_http(429, &headers(&[("retry-after", "86400")]), "{}");
        assert_eq!(quota.class, FailureClass::Fatal);
        assert_eq!(quota.retry_after, Some(Duration::from_secs(86400)));

        let refused = classify_http(503, &headers(&[("x-should-retry", "false")]), "{}");
        assert_eq!(refused.class, FailureClass::Fatal);
    }

    #[test]
    fn stream_errors_classify_by_type() {
        assert_eq!(classify_stream_error("overloaded_error", "Overloaded").reason, RetryReason::Overloaded);
        assert!(classify_stream_error("api_error", "Internal error").is_retryable());
        assert!(classify_stream_error("", "connection reset").is_retryable());
        assert_eq!(classify_stream_error("billing_error", "no credit").class, FailureClass::Fatal);
        assert_eq!(
            classify_stream_error("invalid_request_error", "prompt is too long").class,
            FailureClass::ContextOverflow
        );
        assert_eq!(
            classify_stream_error("stream_error", "HTTP error: operation timed out").reason,
            RetryReason::Timeout
        );
    }

    #[test]
    fn backoff_doubles_to_the_cap_with_jitter() {
        for retry in 1..=12 {
            let ceiling = BASE_DELAY
                .saturating_mul(1u32 << (retry - 1).min(16))
                .min(MAX_DELAY);
            for _ in 0..20 {
                let delay = backoff_delay(RetryBudget::Default, retry, None);
                assert!(delay <= ceiling, "retry {retry}: {delay:?} > {ceiling:?}");
                assert!(delay >= ceiling / 2, "retry {retry}: {delay:?} < half of {ceiling:?}");
            }
        }
        assert!(backoff_delay(RetryBudget::Light, 10, None) <= Duration::from_secs(15));
        // A reasonable server wait wins over the curve; an unreasonable one
        // does not.
        assert_eq!(
            backoff_delay(RetryBudget::Default, 1, Some(Duration::from_secs(42))),
            Duration::from_secs(42)
        );
        assert!(backoff_delay(RetryBudget::Default, 1, Some(Duration::from_secs(3600))) <= BASE_DELAY);
    }

    #[test]
    fn budgets_bound_retries_and_recoveries() {
        assert_eq!(RetryBudget::Default.retries_per_request(), Some(10));
        assert_eq!(RetryBudget::Default.recoveries_per_prompt(), Some(10));
        assert_eq!(RetryBudget::Unbounded.retries_per_request(), None);
        assert_eq!(RetryBudget::Light.recoveries_per_prompt(), Some(0));
        assert_eq!(RetryBudget::SingleAttempt.retries_per_request(), Some(0));
    }

    #[test]
    fn provider_errors_map_onto_the_same_classes() {
        use crate::provider_error::ProviderError;
        use claurst_core::provider_id::ProviderId;
        let id = ProviderId::new("codex");
        let stream = ProviderError::StreamError {
            provider: id.clone(),
            message: "Stream read error: connection reset".into(),
            partial_response: None,
        };
        assert!(from_provider_error(&stream).is_retryable());
        let overflow = ProviderError::ContextOverflow {
            provider: id.clone(),
            message: "too long".into(),
            max_tokens: None,
        };
        assert_eq!(from_provider_error(&overflow).class, FailureClass::ContextOverflow);
        let auth = ProviderError::AuthFailed {
            provider: id.clone(),
            message: "bad key".into(),
        };
        assert_eq!(from_provider_error(&auth).class, FailureClass::Fatal);
        let classified = ProviderError::Classified {
            provider: id,
            failure: ModelFailure::truncated("the stream"),
        };
        assert_eq!(from_provider_error(&classified).reason, RetryReason::Truncated);
    }

    #[test]
    fn after_the_fact_classification_spots_transient_errors() {
        assert_eq!(
            classify_claude_error(&ClaudeError::Api("network error: connection refused".into())),
            FailureClass::Retryable
        );
        assert_eq!(
            classify_claude_error(&ClaudeError::ApiStatus { status: 502, message: "Bad gateway".into() }),
            FailureClass::Retryable
        );
        assert_eq!(
            classify_claude_error(&ClaudeError::ApiStatus { status: 403, message: "Insufficient balance".into() }),
            FailureClass::Fatal
        );
        assert_eq!(classify_claude_error(&ClaudeError::Cancelled), FailureClass::Fatal);
    }

    #[test]
    fn network_timeouts_reset_to_defaults_on_zero() {
        set_network_timeouts(Duration::from_secs(5), Duration::from_secs(60));
        assert_eq!(connect_timeout(), Duration::from_secs(5));
        assert_eq!(idle_timeout(), Duration::from_secs(60));
        set_network_timeouts(Duration::ZERO, Duration::ZERO);
        assert_eq!(connect_timeout(), DEFAULT_CONNECT_TIMEOUT);
        assert_eq!(idle_timeout(), DEFAULT_IDLE_TIMEOUT);
    }

    #[test]
    fn the_request_id_comes_from_the_response_headers() {
        let gateway = classify_http(
            503,
            &headers(&[("x-request-id", "a4293c1e-0d5f-4c9e-9b1a-2f6d8e7c1b20"), ("request-id", "req_other")]),
            "{}",
        );
        assert_eq!(gateway.request_id.as_deref(), Some("a4293c1e-0d5f-4c9e-9b1a-2f6d8e7c1b20"));
        let anthropic = classify_http(529, &headers(&[("request-id", "req_011CSHoEeqs5C35K2UUqR7Fy")]), "{}");
        assert_eq!(anthropic.request_id.as_deref(), Some("req_011CSHoEeqs5C35K2UUqR7Fy"));
        assert_eq!(classify_http(500, &HeaderMap::new(), "{}").request_id, None);
        // An id already known is not overwritten.
        let stamped = anthropic.clone().with_request_id(Some("later".into()));
        assert_eq!(stamped.request_id.as_deref(), Some("req_011CSHoEeqs5C35K2UUqR7Fy"));
    }

    #[test]
    fn the_provider_error_code_survives_classification() {
        let none = HeaderMap::new();
        let openai = classify_http(
            400,
            &none,
            r#"{"error":{"type":"invalid_request_error","code":"content_policy_violation","message":"blocked"}}"#,
        );
        assert_eq!(openai.code.as_deref(), Some("content_policy_violation"));
        let anthropic = classify_http(403, &none, r#"{"type":"error","error":{"type":"permission_error","message":"no"}}"#);
        assert_eq!(anthropic.code.as_deref(), Some("permission_error"));
        let gemini = classify_http(429, &none, r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"slow"}}"#);
        assert_eq!(gemini.code.as_deref(), Some("resource_exhausted"));
        assert_eq!(classify_stream_error("overloaded_error", "busy").code.as_deref(), Some("overloaded_error"));
        assert_eq!(classify_stream_error("", "connection reset").code, None);
    }

    #[test]
    fn an_insufficient_quota_answer_never_retries() {
        let spent = classify_http(
            429,
            &HeaderMap::new(),
            r#"{"error":{"type":"insufficient_quota","code":"insufficient_quota","message":"You exceeded your current quota"}}"#,
        );
        assert_eq!(spent.class, FailureClass::Fatal);
        assert_eq!(spent.status, Some(429));
        // A plain rate limit still waits.
        let busy = classify_http(429, &HeaderMap::new(), r#"{"error":{"code":"rate_limit_exceeded","message":"slow down"}}"#);
        assert_eq!(busy.class, FailureClass::Retryable);
    }

    #[test]
    fn secrets_never_survive_into_a_failure_message() {
        let cases = [
            "Incorrect API key provided: sk-proj-abcdEFGH1234ijkl. You can find your API key at https://platform.openai.com",
            "Incorrect API key provided: sk-ant-api03-****wxyz.",
            "upstream said Authorization: Bearer abcDEF123456ghiJKL",
            r#"{"x-api-key": "cr_9f8e7d6c5b4a39281706"}"#,
            "GET https://generativelanguage.googleapis.com/v1beta/models?key=AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q",
            "token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U rejected",
            "secret Zx81kQp0Lm2Nb7Vc4Xs9Ad3Fg6Hj1Kl5Qw8Er0Ty2Ui4Op rejected",
        ];
        for case in cases {
            let failure = ModelFailure::fatal(Some(401), case);
            for secret in [
                "abcdEFGH1234ijkl",
                "****wxyz",
                "abcDEF123456ghiJKL",
                "cr_9f8e7d6c5b4a39281706",
                "AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q",
                "dozjgNryP4J3jVmNHl0w5N",
                "Zx81kQp0Lm2Nb7Vc4Xs9Ad3Fg6Hj1Kl5Qw8Er0Ty2Ui4Op",
            ] {
                assert!(!failure.message.contains(secret), "{secret} survived in {}", failure.message);
            }
            assert!(failure.message.contains("[redacted]"), "{}", failure.message);
            assert_eq!(redact_secrets(&failure.message), failure.message, "redaction is idempotent");
        }
    }

    #[test]
    fn redaction_leaves_the_gateways_wording_and_ids_alone() {
        for text in [
            "API Key 余额不足 / Insufficient balance for this API key",
            "内容审核未通过 / Content audit rejected（hash: 3f2a9c8e7d6b5a4f3e2d1c0b9a8f7e6d5c4b3a2f1e0d9c8b7a6f5e4d3c2b1a09）",
            "request a4293c1e-0d5f-4c9e-9b1a-2f6d8e7c1b20 failed: risk-management-service unavailable",
            "model claude-opus-5-5 is temporarily unavailable",
        ] {
            assert_eq!(redact_secrets(text), text);
        }
    }

    #[test]
    fn a_rejected_certificate_reads_as_a_network_failure() {
        let tls = classify_transport_detail("error sending request: invalid peer certificate: UnknownIssuer", false);
        assert_eq!(tls.class, FailureClass::Fatal);
        assert_eq!(tls.reason, RetryReason::Network);
        let refused = classify_transport_detail("error sending request: connection refused", false);
        assert_eq!((refused.class, refused.reason), (FailureClass::Retryable, RetryReason::Network));
        assert_eq!(classify_transport_detail("operation", true).reason, RetryReason::Timeout);
    }
}
