//! Does this model answer on this endpoint?
//!
//! The endpoint test on the providers page asks for the models listing,
//! which proves the address and the key and nothing about any one model: a
//! relay happily lists a model its upstream no longer serves, and plenty of
//! servers do not implement the listing at all. This sends the smallest real
//! request in the endpoint's own format — one user word, one output token —
//! to the URL the built-in agent will use, so a green result means the agent
//! can talk to that model there.

use serde_json::{Value, json};

use crate::custom_api::{ProbeResult, ProbeVerdict, send_probe_request};
use crate::gateway::{anthropic_base_url, versioned_url};
use crate::providers::ApiFormat;

/// Seconds one model test waits. Longer than the listing probe: this is a
/// generation, and a reasoning model spends a while before its first token.
pub const MODEL_TEST_TIMEOUT_SECS: u32 = 30;

/// OpenAI's floor for `max_output_tokens` on the Responses API.
const RESPONSES_MIN_OUTPUT_TOKENS: u32 = 16;

/// The request a test of `model` sends, and where.
pub fn model_test_request(
    format: ApiFormat,
    base_url: &str,
    api_key: &str,
    model: &str,
    timeout_secs: u32,
) -> (String, crate::http::Request) {
    model_test_request_with(format, base_url, api_key, model, timeout_secs, false)
}

fn model_test_request_with(
    format: ApiFormat,
    base_url: &str,
    api_key: &str,
    model: &str,
    timeout_secs: u32,
    completion_tokens: bool,
) -> (String, crate::http::Request) {
    let key = api_key.trim();
    let model = model.trim();
    // The URL the agent's adapter builds for this format: `/v1` on the root,
    // unless the stored address already ends in a version of its own.
    let url = versioned_url(&anthropic_base_url(base_url), format.endpoint_path());
    let request = crate::http::Request::new().timeout_seconds(timeout_secs);
    let request = match format {
        ApiFormat::Anthropic => {
            let body = json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}],
            });
            let request = request
                .header("anthropic-version", "2023-06-01")
                .json_body(body.to_string());
            if key.is_empty() {
                request
            } else {
                request.header("x-api-key", key)
            }
        }
        ApiFormat::OpenAiResponses => {
            let body = json!({
                "model": model,
                "input": "hi",
                "max_output_tokens": RESPONSES_MIN_OUTPUT_TOKENS,
            });
            request.json_body(body.to_string())
        }
        ApiFormat::OpenAiChat => {
            let mut body = json!({
                "model": model,
                "messages": [{"role": "user", "content": "hi"}],
                "stream": false,
            });
            // Newer OpenAI reasoning models refuse `max_tokens` and ask for
            // `max_completion_tokens`, which some older servers refuse in
            // turn — so the first try uses the widely accepted one.
            if completion_tokens {
                body["max_completion_tokens"] = json!(RESPONSES_MIN_OUTPUT_TOKENS);
            } else {
                body["max_tokens"] = json!(1);
            }
            request.json_body(body.to_string())
        }
    };
    let request = if format.is_anthropic() || key.is_empty() {
        request
    } else {
        request.bearer(key)
    };
    (url, request)
}

/// Test `model` on the endpoint. Blocks for up to [`MODEL_TEST_TIMEOUT_SECS`]
/// (twice, for a Chat server that asks for the other token field); callers
/// run it off the UI thread.
pub fn test_model(format: ApiFormat, base_url: &str, api_key: &str, model: &str) -> ProbeResult {
    let (url, request) =
        model_test_request(format, base_url, api_key, model, MODEL_TEST_TIMEOUT_SECS);
    let result = read_answer(send_probe_request(&url, request));
    let retry = format == ApiFormat::OpenAiChat
        && result.status == Some(400)
        && result.detail.contains("max_completion_tokens");
    if !retry {
        return result;
    }
    let (url, request) = model_test_request_with(
        format,
        base_url,
        api_key,
        model,
        MODEL_TEST_TIMEOUT_SECS,
        true,
    );
    read_answer(send_probe_request(&url, request))
}

/// Settle what the server said. A relay that wraps an upstream failure in a
/// 200 still failed; the error text is lifted out of whichever envelope the
/// server used; and a successful body is dropped — it is one token of
/// nothing, and the page has no use for it.
fn read_answer(mut result: ProbeResult) -> ProbeResult {
    if result.verdict == ProbeVerdict::Ok {
        if let Some(error) = error_in_body(&result.body) {
            result.verdict = ProbeVerdict::HttpError;
            result.detail = error;
        }
        result.body.clear();
    } else if result.status.is_some()
        && let Some(error) = error_in_body(&result.detail)
    {
        result.detail = error;
    }
    result
}

/// The error message in a JSON body, whichever of the common shapes it
/// takes: `{"error": {"message": …}}` (OpenAI, Anthropic), `{"error": "…"}`,
/// `{"message": …}` with an error `type`. `None` for a body that is not one.
fn error_in_body(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?;
    if error.is_null() {
        return None;
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string());
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(request: &crate::http::Request) -> Value {
        serde_json::from_str(request.body_text().expect("a body")).expect("json")
    }

    #[test]
    fn each_format_asks_for_one_token_in_its_own_shape() {
        let (url, request) = model_test_request(
            ApiFormat::Anthropic,
            "https://relay.example.org",
            "sk-a",
            "claude-sonnet-5-5",
            5,
        );
        assert_eq!(url, "https://relay.example.org/v1/messages");
        assert!(request.header_lines().iter().any(|line| line == "x-api-key: sk-a"));
        assert!(request.header_lines().iter().any(|line| line.starts_with("anthropic-version")));
        let body = body_of(&request);
        assert_eq!(body["model"], "claude-sonnet-5-5");
        assert_eq!(body["max_tokens"], 1);

        let (url, request) = model_test_request(
            ApiFormat::OpenAiResponses,
            "https://relay.example.org/v1",
            "sk-b",
            "gpt-6.1-sol",
            5,
        );
        assert_eq!(url, "https://relay.example.org/v1/responses");
        assert!(
            request
                .header_lines()
                .iter()
                .any(|line| line == "Authorization: Bearer sk-b")
        );
        assert_eq!(body_of(&request)["max_output_tokens"], 16);

        let (url, request) = model_test_request(
            ApiFormat::OpenAiChat,
            "https://relay.example.org",
            "sk-c",
            "deepseek-v4",
            5,
        );
        assert_eq!(url, "https://relay.example.org/v1/chat/completions");
        let body = body_of(&request);
        assert_eq!(body["max_tokens"], 1);
        assert_eq!(body["stream"], false);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn a_versioned_chat_base_is_tested_under_its_version() {
        let (url, _) = model_test_request(
            ApiFormat::OpenAiChat,
            "https://open.bigmodel.cn/api/paas/v4",
            "sk",
            "glm-5.1",
            5,
        );
        assert_eq!(url, "https://open.bigmodel.cn/api/paas/v4/chat/completions");
        let (url, _) = model_test_request(
            ApiFormat::OpenAiResponses,
            "https://ark.cn-beijing.volces.com/api/v3",
            "sk",
            "doubao",
            5,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/v3/responses");
    }

    #[test]
    fn the_retry_asks_for_completion_tokens_instead() {
        let (_, request) = model_test_request_with(
            ApiFormat::OpenAiChat,
            "https://relay.example.org",
            "sk",
            "o5",
            5,
            true,
        );
        let body = body_of(&request);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["max_completion_tokens"], 16);
    }

    fn answered(status: u16, verdict: ProbeVerdict, detail: &str, body: &str) -> ProbeResult {
        ProbeResult {
            latency_ms: 12,
            status: Some(status),
            verdict,
            detail: detail.to_owned(),
            body: body.to_owned(),
        }
    }

    #[test]
    fn an_error_body_with_status_200_is_a_failure() {
        let result = read_answer(answered(
            200,
            ProbeVerdict::Ok,
            "",
            r#"{"error":{"message":"no available channel for model x","type":"new_api_error"}}"#,
        ));
        assert_eq!(result.verdict, ProbeVerdict::HttpError);
        assert_eq!(result.detail, "no available channel for model x");
        assert!(result.body.is_empty());

        let ok = read_answer(answered(200, ProbeVerdict::Ok, "", r#"{"id":"x","error":null}"#));
        assert_eq!(ok.verdict, ProbeVerdict::Ok);
        assert!(ok.body.is_empty(), "the answer itself is not kept");
    }

    #[test]
    fn a_nested_error_message_is_lifted_out() {
        let result = read_answer(answered(
            404,
            ProbeVerdict::HttpError,
            r#"{"error":{"message":"The model `nope` does not exist","code":"model_not_found"}}"#,
            "",
        ));
        assert_eq!(result.detail, "The model `nope` does not exist");
        // A plain-text detail is left as it was.
        let plain = read_answer(answered(502, ProbeVerdict::HttpError, "Bad Gateway", ""));
        assert_eq!(plain.detail, "Bad Gateway");
    }
}
