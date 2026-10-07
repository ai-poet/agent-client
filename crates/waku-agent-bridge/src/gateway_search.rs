//! `WebSearch` through the gateway's Codex search endpoint.
//!
//! The engine's own `WebSearch` reads `SEARXNG_URL` or `BRAVE_SEARCH_API_KEY`
//! from the environment and otherwise asks DuckDuckGo's Instant Answer API,
//! which answers almost nothing for a real query — and nothing here ever sets
//! either variable. The gateway already serves Codex's standalone search,
//! `POST /v1/alpha/search`: a session on it carries the key that pays for it.
//! This tool takes the engine tool's place under the same name, with the same
//! `query` input (the transcript row titles itself from it), and calls that
//! endpoint with the key of the group bound to Codex.
//!
//! A session that has no gateway key — routed to the person's own endpoint,
//! which must never see one, or signed out — keeps the engine's search, as
//! does a gateway whose group cannot search (`404`): a thin answer beats none.
//!
//! The request is the one Codex sends: the conversation item asking, a
//! `search_query` command, and the settings that let the search reach the
//! web. The model reads the response's `output`; `results` names the pages,
//! which are listed after it so the answer can cite them.

use std::time::Duration;

use async_trait::async_trait;
use claurst_core::config::Config;
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult, WebSearchTool};
use serde_json::{Value, json};

use crate::config::{GATEWAY_KEYS_OPTION, MODEL_KEYS_MEMBER};

/// The model a search runs on when the session's own model is not served by
/// the Codex group: Codex's default on the gateway. The endpoint requires one
/// and the group's model list must allow it.
const DEFAULT_SEARCH_MODEL: &str = "gpt-5.6-sol";
/// A search reads and summarises pages before it answers.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(90);
/// How many sources to list after the answer.
const MAX_SOURCES: usize = 10;

pub(crate) struct GatewaySearchTool;

/// Where a search goes and what it is sent with.
#[derive(Debug, PartialEq, Eq)]
struct SearchRoute {
    url: String,
    key: String,
    model: String,
}

#[async_trait]
impl Tool for GatewaySearchTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_WEB_SEARCH
    }

    fn description(&self) -> &str {
        "Search the web for current information. Returns an answer drawn from the pages \
         found, followed by those pages' titles and URLs. Use this when you need information \
         newer than your training data, or documentation, examples or news. Write the query \
         the way you would ask a search engine; narrow it with `domains` or `recency` when \
         that helps."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only search these sites, e.g. [\"docs.rs\"]"
                },
                "recency": {
                    "type": "integer",
                    "description": "Only pages from the last this many days"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let Some(query) = input
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
        else {
            return ToolResult::error("Invalid input: `query` is required".to_owned());
        };
        let Some(route) = search_route(&ctx.config) else {
            return WebSearchTool.execute(input, ctx).await;
        };
        let body = search_body(&ctx.session_id, &route.model, query, &input);
        match send(&route, &ctx.session_id, &body).await {
            Ok(response) => ToolResult::success(format_response(&response)),
            Err(SearchError::Unavailable) => WebSearchTool.execute(input, ctx).await,
            Err(SearchError::Failed(message)) => ToolResult::error(message),
        }
    }
}

enum SearchError {
    /// The gateway does not search for this key's group or model.
    Unavailable,
    Failed(String),
}

/// The gateway's search endpoint and the Codex group's key, when the session
/// runs on the gateway. An endpoint session has had the key table removed
/// (`config::select_route`), so it gets `None` by construction.
fn search_route(config: &Config) -> Option<SearchRoute> {
    let keys = config
        .provider_configs
        .get("anthropic")?
        .options
        .get(GATEWAY_KEYS_OPTION)?
        .as_object()?;
    let key = ["openai", "default"]
        .iter()
        .find_map(|platform| keys.get(*platform)?.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())?;
    let base = ["anthropic", "codex", "openai"]
        .iter()
        .find_map(|provider| {
            config
                .provider_configs
                .get(*provider)?
                .api_base
                .as_deref()
                .map(str::trim)
                .filter(|base| !base.is_empty())
        })?;
    let model_keys = keys.get(MODEL_KEYS_MEMBER).and_then(Value::as_object);
    // The session's own model when the Codex group serves it — filed under
    // that group's key, or a GPT model the platform key routes — so the
    // search runs where the conversation does.
    let model = config
        .model
        .as_deref()
        .map(str::trim)
        .filter(
            |model| match model_keys.and_then(|models| models.get(*model)) {
                Some(model_key) => model_key.as_str() == Some(key),
                None => model.starts_with("gpt-"),
            },
        )
        .unwrap_or(DEFAULT_SEARCH_MODEL);
    Some(SearchRoute {
        url: claurst_api::endpoint::versioned_url(base, "alpha/search"),
        key: key.to_owned(),
        model: model.to_owned(),
    })
}

/// Codex's request: the asking turn, one `search_query` command, and live
/// web access. `id` keeps a session's searches on one upstream account.
fn search_body(session_id: &str, model: &str, query: &str, input: &Value) -> Value {
    let mut command = json!({ "q": query });
    if let Some(domains) = input.get("domains").and_then(Value::as_array) {
        let domains = domains
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|domain| !domain.is_empty())
            .collect::<Vec<_>>();
        if !domains.is_empty() {
            command["domains"] = json!(domains);
        }
    }
    if let Some(recency) = input
        .get("recency")
        .and_then(Value::as_u64)
        .filter(|days| *days > 0)
    {
        command["recency"] = json!(recency);
    }
    json!({
        "id": session_id,
        "model": model,
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": query }]
        }],
        "commands": { "search_query": [command] },
        "settings": {
            "allowed_callers": ["direct"],
            "external_web_access": true
        }
    })
}

async fn send(route: &SearchRoute, session_id: &str, body: &Value) -> Result<Value, SearchError> {
    let client = reqwest::Client::builder()
        .timeout(SEARCH_TIMEOUT)
        .build()
        .map_err(|error| SearchError::Failed(format!("Web search failed: {error}")))?;
    let response = client
        .post(&route.url)
        .bearer_auth(&route.key)
        .header("session_id", session_id)
        .json(body)
        .send()
        .await
        .map_err(|error| SearchError::Failed(format!("Web search failed: {error}")))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| SearchError::Failed(format!("Web search failed: {error}")))?;
    if status == reqwest::StatusCode::NOT_FOUND {
        tracing::warn!(%status, body = %text, "gateway search unavailable; using the engine's");
        return Err(SearchError::Unavailable);
    }
    if !status.is_success() {
        return Err(SearchError::Failed(format!(
            "Web search failed ({status}): {}",
            error_message(&text)
        )));
    }
    serde_json::from_str(&text).map_err(|error| {
        SearchError::Failed(format!("Web search returned an unreadable answer: {error}"))
    })
}

/// The gateway's own words for a failure: `{"error": {"message"}}`, a flat
/// `{"message"}`, or the body itself.
fn error_message(body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(500).collect())
}

/// The answer, then the pages it came from as markdown links.
fn format_response(response: &Value) -> String {
    let output = response
        .get("output")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let mut seen = Vec::new();
    let sources = response
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let url = result.get("url").and_then(Value::as_str)?.trim();
            if url.is_empty() || seen.contains(&url) {
                return None;
            }
            seen.push(url);
            let title = result
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or(url);
            Some(format!("- [{title}]({url})"))
        })
        .take(MAX_SOURCES)
        .collect::<Vec<_>>();
    match (output.is_empty(), sources.is_empty()) {
        (true, true) => "No results found.".to_owned(),
        (false, true) => output.to_owned(),
        (true, false) => format!("Sources:\n{}", sources.join("\n")),
        (false, false) => format!("{output}\n\nSources:\n{}", sources.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use claurst_core::ProviderConfig;

    use super::*;

    fn gateway_config(model: &str, keys: Value) -> Config {
        let mut config = Config::default();
        config.model = Some(model.to_owned());
        let mut anthropic = ProviderConfig::default();
        anthropic.api_base = Some("https://gw.example".to_owned());
        anthropic
            .options
            .insert(GATEWAY_KEYS_OPTION.to_owned(), keys);
        config
            .provider_configs
            .insert("anthropic".to_owned(), anthropic);
        config
    }

    #[test]
    fn a_gateway_session_searches_with_the_codex_groups_key() {
        let config = gateway_config(
            "gpt-6-sol",
            json!({"openai": "sk-codex", "default": "sk-general", "anthropic": "sk-claude"}),
        );
        assert_eq!(
            search_route(&config),
            Some(SearchRoute {
                url: "https://gw.example/v1/alpha/search".to_owned(),
                key: "sk-codex".to_owned(),
                model: "gpt-6-sol".to_owned(),
            })
        );
    }

    #[test]
    fn a_model_another_group_serves_searches_on_codexs_default() {
        let keys = json!({
            "openai": "sk-codex",
            "models": {"claude-opus-5-5": "sk-claude", "gpt-6-sol": "sk-codex", "gpt-6-astra": "sk-other"}
        });
        let route = |model: &str| {
            search_route(&gateway_config(model, keys.clone()))
                .unwrap()
                .model
        };
        assert_eq!(route("claude-opus-5-5"), DEFAULT_SEARCH_MODEL);
        assert_eq!(route("gpt-6-sol"), "gpt-6-sol");
        assert_eq!(route("gpt-6-astra"), DEFAULT_SEARCH_MODEL);
        assert_eq!(route("deepseek-v4"), DEFAULT_SEARCH_MODEL);
    }

    #[test]
    fn without_a_codex_key_the_general_key_is_tried() {
        let config = gateway_config("gpt-6-sol", json!({"default": "sk-general"}));
        assert_eq!(search_route(&config).unwrap().key, "sk-general");
    }

    #[test]
    fn a_session_without_gateway_keys_has_no_gateway_search() {
        let mut config = gateway_config("gpt-6-sol", json!({}));
        assert_eq!(search_route(&config), None);
        config.provider_configs.clear();
        assert_eq!(search_route(&config), None);
    }

    #[test]
    fn the_request_carries_the_query_as_codex_sends_it() {
        let body = search_body(
            "session-1",
            "gpt-6-sol",
            "tokio 1.40 release notes",
            &json!({"query": "tokio 1.40 release notes", "domains": ["github.com", " "], "recency": 30}),
        );
        assert_eq!(body["id"], "session-1");
        assert_eq!(body["model"], "gpt-6-sol");
        assert_eq!(
            body["commands"]["search_query"],
            json!([{"q": "tokio 1.40 release notes", "domains": ["github.com"], "recency": 30}])
        );
        assert_eq!(
            body["input"][0]["content"][0]["text"],
            "tokio 1.40 release notes"
        );
        assert_eq!(body["settings"]["external_web_access"], true);

        let plain = search_body("s", "m", "q", &json!({"query": "q", "recency": 0}));
        assert_eq!(plain["commands"]["search_query"], json!([{"q": "q"}]));
    }

    #[test]
    fn the_answer_comes_first_and_its_sources_after_it() {
        let response = json!({
            "output": "Tokio 1.40 added ...",
            "encrypted_output": "opaque",
            "results": [
                {"type": "text_result", "url": "https://github.com/tokio-rs/tokio/releases", "title": "Releases"},
                {"type": "text_result", "url": "https://github.com/tokio-rs/tokio/releases", "title": "Again"},
                {"type": "text_result", "url": "https://docs.rs/tokio", "title": ""},
                {"type": "image_result"}
            ]
        });
        assert_eq!(
            format_response(&response),
            "Tokio 1.40 added ...\n\nSources:\n\
             - [Releases](https://github.com/tokio-rs/tokio/releases)\n\
             - [https://docs.rs/tokio](https://docs.rs/tokio)"
        );
        assert_eq!(
            format_response(&json!({"output": "Only text"})),
            "Only text"
        );
        assert_eq!(format_response(&json!({})), "No results found.");
    }

    #[test]
    fn a_failure_reports_the_gateways_own_words() {
        assert_eq!(
            error_message(
                r#"{"error":{"type":"api_error","message":"Service temporarily unavailable"}}"#
            ),
            "Service temporarily unavailable"
        );
        assert_eq!(
            error_message(
                r#"{"code":"INVALID_API_KEY","message":"API Key 无效 / Invalid API key"}"#
            ),
            "API Key 无效 / Invalid API key"
        );
        assert_eq!(error_message("bad gateway"), "bad gateway");
    }
}
