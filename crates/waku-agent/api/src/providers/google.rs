// providers/google.rs — GoogleProvider: implements LlmProvider for the
// Google Gemini API (generativelanguage.googleapis.com).
//
// Supports:
// - Non-streaming: POST .../generateContent?key={api_key}
// - Streaming SSE: POST .../streamGenerateContent?alt=sse&key={api_key}
// - Tool/function calling via functionDeclarations
// - System prompts via systemInstruction field
// - Thinking config for Gemini 2.5+ and 3.0+ models
// - Image/video inputs via inlineData parts
// - list_models via GET /v1beta/models
//
// Fork departures (Waku), so the built-in agent can drive Gemini 4 Argon
// through the managed gateway's `/v1beta` (or `/antigravity/v1beta`) mount:
// `with_gateway` takes any base and the connect/read timeouts every other
// model client uses; the key rides in `x-goog-api-key` only, never in the
// URL (which was also logged); `thought` parts stream as reasoning instead
// of answer text; usage counts thinking and cached tokens; failures are
// classified for the retry loop and a stream that ends without a
// `finishReason` is one; tool-call ids are unique across turns.

use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use claurst_core::provider_id::ProviderId;
use claurst_core::types::{ContentBlock, Message, MessageContent, Role, ToolResultContent, UsageInfo};
use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::error_handling::parse_error_response as parse_http_error;
use crate::provider::LlmProvider;
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StopReason,
    StreamEvent, SystemPrompt, SystemPromptStyle,
};

use super::request_options::merge_google_options;

// ---------------------------------------------------------------------------
// GoogleProvider
// ---------------------------------------------------------------------------

const GOOGLE_API_BASE: &str = "https://generativelanguage.googleapis.com";

/// Hex digits of the per-response nonce in a made-up tool-call id.
const TOOL_ID_NONCE_LEN: usize = 8;

/// The index thinking deltas stream under, apart from the text block at 0
/// and the tool calls from 1000 (the Chat adapter uses the same one).
const THINKING_BLOCK_INDEX: usize = usize::MAX - 100;

pub struct GoogleProvider {
    id: ProviderId,
    api_key: String,
    base_url: String,
    http_client: reqwest::Client,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_gateway(GOOGLE_API_BASE, api_key)
    }

    /// Fork addition (Waku): the Gemini API behind any base — the managed
    /// gateway's origin, its `/antigravity` mount, or a base that already
    /// names its version (`…/v1beta`). Request shape, streaming and tool
    /// translation are unchanged; only where the request goes moves.
    pub fn with_gateway(base_url: impl Into<String>, api_key: String) -> Self {
        let http_client = crate::retry_policy::model_client_builder()
            .build()
            .expect("failed to build reqwest client");
        Self {
            id: ProviderId::new(ProviderId::GOOGLE),
            api_key,
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            http_client,
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Returns true if the model supports thinking config (Gemini 2.5+ / 3.0+ / 4).
    fn supports_thinking(model: &str) -> bool {
        model.contains("2.5")
            || model.contains("3.0")
            || model.contains("3.1")
            || model.contains("gemini-3")
            || model.contains("gemini-4")
    }

    /// `{base}/v1beta/{path}`, or `{base}/{path}` when the base already ends
    /// in a version segment.
    fn api_url(&self, path: &str) -> String {
        if crate::endpoint::ends_with_version_segment(&self.base_url) {
            format!("{}/{}", self.base_url, path)
        } else {
            format!("{}/v1beta/{}", self.base_url, path)
        }
    }

    /// Build the full generateContent URL for non-streaming. The key is sent
    /// as `x-goog-api-key`, never in the URL.
    fn generate_url(&self, model: &str) -> String {
        self.api_url(&format!("models/{model}:generateContent"))
    }

    /// Build the full streamGenerateContent URL for streaming.
    fn stream_url(&self, model: &str) -> String {
        self.api_url(&format!("models/{model}:streamGenerateContent?alt=sse"))
    }

    /// A tool call's id. Gemini sends none, so one is made up: the function
    /// name, a per-response nonce — ids restarting at `call_<name>` every
    /// response repeated across turns, and Waku keys tool rows by id — and an
    /// occurrence number for a name called twice in one response.
    fn tool_use_id_for_name(name: &str, nonce: &str, occurrence: usize) -> String {
        let sanitized: String = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let base = if sanitized.is_empty() { "tool" } else { sanitized.as_str() };
        if occurrence == 0 {
            format!("call_{}_{}", base, nonce)
        } else {
            format!("call_{}_{}_{}", base, nonce, occurrence + 1)
        }
    }

    /// A fresh nonce for [`Self::tool_use_id_for_name`]: eight hex digits.
    fn tool_id_nonce() -> String {
        let id = uuid_v4_simple();
        id[id.len() - TOOL_ID_NONCE_LEN..].to_string()
    }

    fn tool_name_by_id(messages: &[Message]) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        for message in messages {
            let MessageContent::Blocks(blocks) = &message.content else {
                continue;
            };
            for block in blocks {
                if let ContentBlock::ToolUse { id, name, .. } = block {
                    map.insert(id.clone(), name.clone());
                }
            }
        }
        map
    }

    fn infer_tool_name_from_id(tool_use_id: &str) -> Option<String> {
        let raw = tool_use_id.strip_prefix("call_")?;
        let trimmed = if let Some((candidate, suffix)) = raw.rsplit_once('_') {
            if !candidate.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) {
                candidate
            } else {
                raw
            }
        } else {
            raw
        };
        // Fork (Waku): then the nonce `tool_use_id_for_name` puts before it.
        let trimmed = match trimmed.rsplit_once('_') {
            Some((candidate, nonce))
                if !candidate.is_empty()
                    && nonce.len() == TOOL_ID_NONCE_LEN
                    && nonce.chars().all(|ch| ch.is_ascii_hexdigit()) =>
            {
                candidate
            }
            _ => trimmed,
        };

        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Convert a single ContentBlock to a Gemini "part" Value.
    /// Returns None for blocks that should be dropped (e.g. Thinking).
    fn content_block_to_part(block: &ContentBlock) -> Option<Value> {
        match block {
            ContentBlock::Text { text } => Some(json!({ "text": text })),

            ContentBlock::Image { source } => {
                // Prefer base64 inline data; fall back to URL if available.
                if let (Some(data), Some(mime)) = (&source.data, &source.media_type) {
                    Some(json!({
                        "inlineData": {
                            "data": data,
                            "mimeType": mime
                        }
                    }))
                } else { source.url.as_ref().map(|url| json!({
                        "fileData": {
                            "fileUri": url,
                            "mimeType": source.media_type.as_deref().unwrap_or("image/jpeg")
                        }
                    })) }
            }

            ContentBlock::ToolUse {
                name,
                input,
                thought_signature,
                ..
            } => {
                let mut part = json!({
                    "functionCall": {
                        "name": name,
                        "args": input
                    }
                });
                // Gemini 3.x thinking models require the exact `thoughtSignature`
                // captured from the response to be echoed back on the functionCall
                // part; omitting it yields HTTP 400 (issue #311). REST JSON uses
                // camelCase. Absent signature → old behaviour (no key emitted).
                if let (Some(sig), Some(obj)) = (thought_signature, part.as_object_mut()) {
                    obj.insert("thoughtSignature".to_string(), json!(sig));
                }
                Some(part)
            }

            // Thinking blocks are not supported by Gemini — drop silently.
            ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. } => None,

            // Document blocks: treat as file data when URL is available,
            // otherwise as inline base64.
            ContentBlock::Document { source, .. } => {
                if let (Some(data), Some(mime)) = (&source.data, &source.media_type) {
                    Some(json!({
                        "inlineData": {
                            "data": data,
                            "mimeType": mime
                        }
                    }))
                } else { source.url.as_ref().map(|url| json!({
                        "fileData": {
                            "fileUri": url,
                            "mimeType": source.media_type.as_deref().unwrap_or("application/pdf")
                        }
                    })) }
            }

            // Render UI-only / metadata blocks as text so context is not lost.
            ContentBlock::UserLocalCommandOutput { command, output } => Some(json!({
                "text": format!("$ {}\n{}", command, output)
            })),
            ContentBlock::UserCommand { name, args } => Some(json!({
                "text": format!("/{} {}", name, args)
            })),
            ContentBlock::UserMemoryInput { key, value } => Some(json!({
                "text": format!("[memory] {}: {}", key, value)
            })),
            ContentBlock::SystemAPIError { message, .. } => Some(json!({
                "text": format!("[error] {}", message)
            })),
            ContentBlock::CollapsedReadSearch { tool_name, paths, .. } => Some(json!({
                "text": format!("[{}] {}", tool_name, paths.join(", "))
            })),
            ContentBlock::TaskAssignment { id, subject, description } => Some(json!({
                "text": format!("[task:{}] {}: {}", id, subject, description)
            })),

            // ToolResult is handled specially in message conversion.
            ContentBlock::ToolResult { .. } => None,
        }
    }

    /// Convert a ToolResult block to a "functionResponse" part Value.
    fn tool_result_to_part(tool_name: &str, content: &ToolResultContent) -> Value {
        let response_content = match content {
            ToolResultContent::Text(t) => json!({ "content": t }),
            ToolResultContent::Blocks(blocks) => {
                // Concatenate all text blocks for the response payload.
                let text: String = blocks
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                json!({ "content": text })
            }
        };
        json!({
            "functionResponse": {
                "name": tool_name,
                "response": response_content
            }
        })
    }

    /// Sanitize a JSON Schema object for Google's stricter requirements:
    /// - Integer enums → string enums
    /// - `required` must only list fields actually in `properties`
    /// - Non-object types must not have `properties`/`required`
    /// - Array `items` must have a `type` field
    fn sanitize_schema(schema: Value) -> Value {
        match schema {
            Value::Object(mut map) => {
                // Strip keywords that Gemini's function-declaration schema does
                // not understand and will reject with a 400 error.
                map.remove("additionalProperties");
                map.remove("$schema");
                map.remove("default");
                map.remove("examples");
                map.remove("title");

                // Recurse into nested schemas first.
                let schema_type = map
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                // Convert integer enums to string enums.
                if let Some(Value::Array(enum_vals)) = map.get("enum") {
                    if enum_vals.iter().any(|v| v.is_number()) {
                        let string_enums: Vec<Value> = enum_vals
                            .iter()
                            .map(|v| Value::String(v.to_string()))
                            .collect();
                        map.insert("enum".to_string(), Value::Array(string_enums));
                        // Upgrade type to string when converting number enums.
                        map.insert("type".to_string(), Value::String("string".to_string()));
                    }
                }

                // For object types: sanitize properties recursively and fix required.
                if schema_type.as_deref() == Some("object") {
                    if let Some(Value::Object(props)) = map.get_mut("properties") {
                        let sanitized_props: serde_json::Map<String, Value> = props
                            .iter()
                            .map(|(k, v)| (k.clone(), Self::sanitize_schema(v.clone())))
                            .collect();
                        *props = sanitized_props;
                    }

                    // Filter required to only include keys present in properties.
                    if let Some(Value::Array(req_arr)) = map.get("required").cloned() {
                        let prop_keys: std::collections::HashSet<String> = map
                            .get("properties")
                            .and_then(|p| p.as_object())
                            .map(|o| o.keys().cloned().collect())
                            .unwrap_or_default();

                        let filtered: Vec<Value> = req_arr
                            .into_iter()
                            .filter(|v| {
                                v.as_str()
                                    .map(|s| prop_keys.contains(s))
                                    .unwrap_or(false)
                            })
                            .collect();
                        map.insert("required".to_string(), Value::Array(filtered));
                    }
                } else {
                    // Non-object types must not carry properties/required.
                    map.remove("properties");
                    map.remove("required");
                }

                // Array items: ensure a type field is present.
                if schema_type.as_deref() == Some("array") {
                    if let Some(items) = map.get_mut("items") {
                        if let Value::Object(ref mut items_map) = items {
                            if !items_map.contains_key("type") {
                                items_map
                                    .insert("type".to_string(), Value::String("string".to_string()));
                            }
                            // Recurse sanitize into items.
                            let sanitized = Self::sanitize_schema(Value::Object(items_map.clone()));
                            *items = sanitized;
                        }
                    }
                }

                Value::Object(map)
            }
            other => other,
        }
    }

    /// Build the full request body JSON for the Gemini API.
    fn build_request_body(&self, request: &ProviderRequest) -> Value {
        // ---- Convert messages ----
        // Google requires a flat list of content objects.
        // ToolResult blocks must become separate user-role messages.
        let mut contents: Vec<Value> = Vec::new();
        let tool_name_by_id = Self::tool_name_by_id(&request.messages);

        for msg in &request.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "model",
            };

            let blocks = msg.content_blocks();

            let mut regular_parts: Vec<Value> = Vec::new();
            let mut tool_result_parts: Vec<Value> = Vec::new();
            let flush_regular_parts = |contents: &mut Vec<Value>, parts: &mut Vec<Value>| {
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": role,
                        "parts": std::mem::take(parts)
                    }));
                }
            };
            let flush_tool_result_parts = |contents: &mut Vec<Value>, parts: &mut Vec<Value>| {
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": "user",
                        "parts": std::mem::take(parts)
                    }));
                }
            };

            for block in &blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    flush_regular_parts(&mut contents, &mut regular_parts);
                    let tool_name = tool_name_by_id
                        .get(tool_use_id)
                        .cloned()
                        .or_else(|| Self::infer_tool_name_from_id(tool_use_id))
                        .unwrap_or_else(|| tool_use_id.clone());
                    tool_result_parts.push(Self::tool_result_to_part(&tool_name, content));
                } else if let Some(part) = Self::content_block_to_part(block) {
                    flush_tool_result_parts(&mut contents, &mut tool_result_parts);
                    regular_parts.push(part);
                }
            }

            flush_regular_parts(&mut contents, &mut regular_parts);
            flush_tool_result_parts(&mut contents, &mut tool_result_parts);
        }

        // ---- System instruction ----
        let system_instruction: Option<Value> = request.system_prompt.as_ref().map(|sp| {
            let text = match sp {
                SystemPrompt::Text(t) => t.clone(),
                SystemPrompt::Blocks(blocks) => blocks
                    .iter()
                    .map(|b| b.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            json!({ "parts": [{ "text": text }] })
        });

        // ---- Tool declarations ----
        let tools_value: Option<Value> = if request.tools.is_empty() {
            None
        } else {
            let declarations: Vec<Value> = request
                .tools
                .iter()
                .map(|td| {
                    json!({
                        "name": td.name,
                        "description": td.description,
                        "parameters": Self::sanitize_schema(td.input_schema.clone())
                    })
                })
                .collect();
            Some(json!([{ "functionDeclarations": declarations }]))
        };

        // ---- Generation config ----
        let mut gen_config = serde_json::Map::new();
        gen_config.insert(
            "maxOutputTokens".to_string(),
            json!(request.max_tokens),
        );
        if let Some(temp) = request.temperature {
            gen_config.insert("temperature".to_string(), json!(temp));
        }
        if !request.stop_sequences.is_empty() {
            gen_config.insert(
                "stopSequences".to_string(),
                json!(request.stop_sequences),
            );
        }
        if let Some(top_p) = request.top_p {
            gen_config.insert("topP".to_string(), json!(top_p));
        }
        if let Some(top_k) = request.top_k {
            gen_config.insert("topK".to_string(), json!(top_k));
        }

        // Thinking config for supported models.
        if Self::supports_thinking(&request.model) && request.thinking.is_some() {
            let budget = request
                .thinking
                .as_ref()
                .map(|t| t.budget_tokens)
                .unwrap_or(8192);
            gen_config.insert(
                "thinkingConfig".to_string(),
                json!({
                    "includeThoughts": true,
                    "thinkingBudget": budget
                }),
            );
        }

        // ---- Assemble body ----
        let mut body = serde_json::Map::new();
        body.insert("contents".to_string(), Value::Array(contents));
        body.insert(
            "generationConfig".to_string(),
            Value::Object(gen_config),
        );
        if let Some(si) = system_instruction {
            body.insert("systemInstruction".to_string(), si);
        }
        if let Some(tools) = tools_value {
            body.insert("tools".to_string(), tools);
        }

        let mut value = Value::Object(body);
        merge_google_options(&mut value, &request.provider_options);
        value
    }

    /// Parse a Google error JSON body and return the appropriate ProviderError.
    fn parse_error_response(&self, status: u16, body: &str) -> ProviderError {
        parse_http_error(status, body, &self.id)
    }

    /// Extract content blocks and usage from a completed Gemini response body.
    fn parse_response_body(
        &self,
        body: &Value,
        model: &str,
    ) -> Result<ProviderResponse, ProviderError> {
        if let Some(failure) = prompt_block_failure(body) {
            return Err(ProviderError::Classified { provider: self.id.clone(), failure });
        }
        let candidates = body
            .get("candidates")
            .and_then(|c| c.as_array())
            .ok_or_else(|| ProviderError::Other {
                provider: self.id.clone(),
                message: "Missing 'candidates' in response".to_string(),
                status: None,
                body: Some(body.to_string()),
            })?;

        let candidate = candidates.first().ok_or_else(|| ProviderError::Other {
            provider: self.id.clone(),
            message: "Empty 'candidates' array in response".to_string(),
            status: None,
            body: Some(body.to_string()),
        })?;

        let finish_reason = candidate
            .get("finishReason")
            .and_then(|r| r.as_str())
            .unwrap_or("STOP");

        let stop_reason = match finish_reason {
            "STOP" => StopReason::EndTurn,
            "MAX_TOKENS" => StopReason::MaxTokens,
            "SAFETY" => StopReason::ContentFiltered,
            "RECITATION" => StopReason::ContentFiltered,
            "TOOL_CODE" | "FUNCTION_CALL" => StopReason::ToolUse,
            other => StopReason::Other(other.to_string()),
        };

        let parts = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array());

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_name_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let nonce = Self::tool_id_nonce();

        if let Some(parts) = parts {
            for part in parts {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    // Fork (Waku): a thought summary is reasoning, not answer.
                    if is_thought_part(part) {
                        content_blocks.push(ContentBlock::Thinking {
                            thinking: text.to_string(),
                            signature: String::new(),
                        });
                        continue;
                    }
                    content_blocks.push(ContentBlock::Text {
                        text: text.to_string(),
                    });
                } else if let Some(fc) = part.get("functionCall") {
                    let name = fc
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    let thought_signature = thought_signature_from_part(part);
                    let occurrence = tool_name_counts
                        .entry(name.clone())
                        .and_modify(|count| *count += 1)
                        .or_insert(0);
                    let id = Self::tool_use_id_for_name(&name, &nonce, *occurrence);
                    content_blocks.push(ContentBlock::ToolUse {
                        id,
                        name,
                        input: args,
                        thought_signature,
                    });
                }
            }
        }

        // Extract usage metadata.
        let usage = self.extract_usage(body);

        Ok(ProviderResponse {
            id: format!("gemini-{}", uuid_v4_simple()),
            content: content_blocks,
            stop_reason,
            usage,
            model: model.to_string(),
        })
    }

    /// Extract UsageInfo from a response body's usageMetadata field.
    fn extract_usage(&self, body: &Value) -> UsageInfo {
        usage_from_metadata(body.get("usageMetadata"))
    }

    /// Fork (Waku): a failure to get any response, classified for the
    /// retry loop.
    fn transport_error(&self, error: &reqwest::Error) -> ProviderError {
        ProviderError::Classified {
            provider: self.id.clone(),
            failure: crate::retry_policy::classify_transport(error),
        }
    }

    /// Fork (Waku): an HTTP error answer, classified with its headers so the
    /// loop sees `Retry-After` and the gateway's own message.
    async fn http_error(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();
        ProviderError::Classified {
            provider: self.id.clone(),
            failure: crate::retry_policy::classify_http(status, &headers, &body),
        }
    }

}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for GoogleProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "Google"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let url = self.generate_url(&request.model);
        let model = request.model.clone();
        let body = self.build_request_body(&request);

        debug!("Google create_message: POST {}", url);

        let resp = self
            .http_client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| self.transport_error(&e))?;

        if !resp.status().is_success() {
            return Err(self.http_error(resp).await);
        }
        let status = resp.status().as_u16();
        let resp_body = resp.text().await.map_err(|e| self.transport_error(&e))?;

        let json_body: Value =
            serde_json::from_str(&resp_body).map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Failed to parse response JSON: {}", e),
                status: Some(status),
                body: Some(resp_body.clone()),
            })?;

        self.parse_response_body(&json_body, &model)
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let url = self.stream_url(&request.model);
        let model = request.model.clone();
        let body = self.build_request_body(&request);

        debug!("Google create_message_stream: POST {}", url);

        let resp = self
            .http_client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| self.transport_error(&e))?;

        if !resp.status().is_success() {
            return Err(self.http_error(resp).await);
        }
        // Fork (Waku): a failure inside the stream still names its request.
        let request_id = crate::retry_policy::request_id_of(resp.headers());

        // Wrap the byte stream in a line-based SSE parser.
        let provider_id_for_stream = self.id.clone();
        let model_clone = model.clone();
        let byte_stream = resp.bytes_stream();

        // TODO(#228): Gemini has its own SSE JSON shape (candidates/parts); this
        // decode belongs in a `protocol::gemini` decoder, alongside the
        // OpenAI-Chat and AnthropicMessages protocols.
        let stream = async_stream::stream! {
            let mut byte_stream = byte_stream;
            let text_block_index: usize = 0;
            let mut tool_block_index: usize = 1000;
            // Fork (Waku): the blocks opened so far, closed at the finish.
            // Gemini sends every functionCall part whole and each chunk
            // numbers its parts from 0, so parallel calls arrive as one
            // chunk each at `parts[0]`; keying open calls by that position
            // glued the second call's arguments onto the first's
            // (`{…}{…}`), and the engine refused both as malformed JSON.
            let mut open_tool_calls: Vec<usize> = Vec::new();
            let mut emitted_message_start = false;
            let message_id = format!("gemini-{}", uuid_v4_simple());
            // Shared byte-buffering decoder (#228): buffers raw bytes and only
            // decodes complete lines. Previously a non-UTF8 chunk — which is
            // exactly what a multibyte codepoint split across a chunk boundary
            // looks like — was skipped entirely, dropping data.
            let mut decoder = crate::SseByteDecoder::new();
            let mut tool_name_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let nonce = Self::tool_id_nonce();
            // Fork (Waku): a stream that closes before any candidate names a
            // finish reason broke off, and its partial answer must not stand.
            let mut saw_finish = false;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk: Bytes = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ProviderError::Classified {
                            provider: provider_id_for_stream.clone(),
                            failure: crate::retry_policy::classify_transport(&e),
                        });
                        return;
                    }
                };

                // Process complete lines.
                for line in decoder.push(&chunk) {
                    let line = line.trim_end_matches('\r');

                    if let Some(data) = line.strip_prefix("data: ") {
                        let data = data.trim();
                        if data.is_empty() || data == "[DONE]" {
                            continue;
                        }

                        // Parse the JSON payload and emit events.
                        let parsed: Value = match serde_json::from_str(data) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!("Google SSE: JSON parse error: {}: {}", e, data);
                                continue;
                            }
                        };

                        // Check for stream-level error.
                        if let Some(err) = parsed.get("error") {
                            yield Err(ProviderError::Classified {
                                provider: provider_id_for_stream.clone(),
                                failure: stream_error_failure(err),
                            });
                            return;
                        }

                        if let Some(failure) = prompt_block_failure(&parsed) {
                            yield Err(ProviderError::Classified {
                                provider: provider_id_for_stream.clone(),
                                failure,
                            });
                            return;
                        }

                        // Emit MessageStart on first chunk.
                        if !emitted_message_start {
                            emitted_message_start = true;
                            let usage = usage_from_metadata(parsed.get("usageMetadata"));
                            yield Ok(StreamEvent::MessageStart {
                                id: message_id.clone(),
                                model: model_clone.clone(),
                                usage,
                            });
                        }

                        let candidates = parsed
                            .get("candidates")
                            .and_then(|c| c.as_array());

                        let Some(candidates) = candidates else { continue };

                        for candidate in candidates {
                            let parts = candidate
                                .get("content")
                                .and_then(|c| c.get("parts"))
                                .and_then(|p| p.as_array());

                            if let Some(parts) = parts {
                                for part in parts {
                                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                        if text.is_empty() {
                                            continue;
                                        }
                                        // Fork (Waku): with `includeThoughts`, the
                                        // thought summary arrives as text parts
                                        // marked `thought`; it is reasoning, not
                                        // the answer.
                                        if is_thought_part(part) {
                                            yield Ok(StreamEvent::ReasoningDelta {
                                                index: THINKING_BLOCK_INDEX,
                                                reasoning: text.to_string(),
                                            });
                                            continue;
                                        }
                                        yield Ok(StreamEvent::TextDelta {
                                            index: text_block_index,
                                            text: text.to_string(),
                                        });
                                    } else if let Some(fc) = part.get("functionCall") {
                                        let name = fc
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let args_str = fc
                                            .get("args")
                                            .map(|a| a.to_string())
                                            .unwrap_or_else(|| "{}".to_string());
                                        // Gemini streams a functionCall part whole, so its
                                        // thoughtSignature is present on this same part and
                                        // must ride along on the ToolUse block (#311).
                                        let thought_signature = thought_signature_from_part(part);

                                        let occurrence = tool_name_counts
                                            .entry(name.clone())
                                            .and_modify(|count| *count += 1)
                                            .or_insert(0);
                                        let id = Self::tool_use_id_for_name(&name, &nonce, *occurrence);
                                        let idx = tool_block_index;
                                        tool_block_index += 1;
                                        open_tool_calls.push(idx);
                                        yield Ok(StreamEvent::ContentBlockStart {
                                            index: idx,
                                            content_block: ContentBlock::ToolUse {
                                                id,
                                                name: name.clone(),
                                                input: json!({}),
                                                thought_signature,
                                            },
                                        });

                                        yield Ok(StreamEvent::InputJsonDelta {
                                            index: idx,
                                            partial_json: args_str,
                                        });
                                    }
                                }
                            }

                            // Handle finish reason.
                            let finish_reason = candidate
                                .get("finishReason")
                                .and_then(|r| r.as_str())
                                .unwrap_or("");

                            if !finish_reason.is_empty()
                                && finish_reason != "FINISH_REASON_UNSPECIFIED"
                            {
                                saw_finish = true;
                                // Close text block.
                                yield Ok(StreamEvent::ContentBlockStop {
                                    index: text_block_index,
                                });

                                // Close tool call blocks.
                                for idx in std::mem::take(&mut open_tool_calls) {
                                    yield Ok(StreamEvent::ContentBlockStop { index: idx });
                                }

                                let stop_reason = match finish_reason {
                                    "STOP" => Some(StopReason::EndTurn),
                                    "MAX_TOKENS" => Some(StopReason::MaxTokens),
                                    "SAFETY" | "RECITATION" => Some(StopReason::ContentFiltered),
                                    "TOOL_CODE" | "FUNCTION_CALL" => Some(StopReason::ToolUse),
                                    other => Some(StopReason::Other(other.to_string())),
                                };

                                let final_usage = usage_from_metadata(parsed.get("usageMetadata"));

                                yield Ok(StreamEvent::MessageDelta {
                                    stop_reason,
                                    usage: Some(final_usage),
                                });
                                yield Ok(StreamEvent::MessageStop);
                            }
                        }
                    }
                    // SSE comment lines (": ...") and blank lines are ignored.
                }
            }

            if !saw_finish {
                yield Err(ProviderError::Classified {
                    provider: provider_id_for_stream.clone(),
                    failure: crate::retry_policy::ModelFailure::truncated("the Gemini stream"),
                });
            }
        };

        Ok(Box::pin(futures::StreamExt::map(stream, move |item| {
            item.map_err(|error| error.with_request_id(request_id.as_deref()))
        })))
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        // Lightweight liveness probe: a models-listing GET on the live endpoint.
        // (Model *listing* for the picker comes from the catalog, not here.)
        let url = self.api_url("models");
        let resp = self
            .http_client
            .get(&url)
            .header("x-goog-api-key", &self.api_key)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => Ok(ProviderStatus::Healthy),
            Ok(r) => {
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                match self.parse_error_response(status, &body) {
                    ProviderError::AuthFailed { message, .. } => {
                        Err(ProviderError::AuthFailed {
                            provider: self.id.clone(),
                            message,
                        })
                    }
                    e => Ok(ProviderStatus::Unavailable {
                        reason: e.to_string(),
                    }),
                }
            }
            Err(e) => Ok(ProviderStatus::Unavailable {
                reason: e.to_string(),
            }),
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: true,
            image_input: true,
            pdf_input: true,
            audio_input: false,
            video_input: true,
            caching: false,
            structured_output: true,
            system_prompt_style: SystemPromptStyle::SystemInstruction,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a simple pseudo-random hex ID without pulling in the uuid crate.
/// Uses a combination of the current time and a thread-local counter.
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Simple hash mix to spread bits.
    let a = t ^ (t >> 17) ^ (t << 13);
    let b = a.wrapping_mul(0x517cc1b727220a95);
    format!("{:032x}", b)
}

/// Extract Gemini's opaque `thoughtSignature` (camelCase in REST JSON) from a
/// response `part`. Thinking models attach it as a sibling of `functionCall`;
/// it must be captured and echoed back verbatim on the next turn or the API
/// rejects the tool call with HTTP 400 (issue #311). Shared by the streaming
/// and non-streaming parsers, whose parts have the identical shape.
fn thought_signature_from_part(part: &Value) -> Option<String> {
    part.get("thoughtSignature")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// Fork (Waku): whether a text part is a thought summary (`"thought": true`).
fn is_thought_part(part: &Value) -> bool {
    part.get("thought").and_then(Value::as_bool).unwrap_or(false)
}

/// Fork (Waku): usage from a `usageMetadata` object. `promptTokenCount`
/// already includes the cached tokens, which are reported apart (as the
/// Responses adapter does); thinking tokens are output, billed as such.
fn usage_from_metadata(meta: Option<&Value>) -> UsageInfo {
    let count = |key: &str| {
        meta.and_then(|m| m.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = count("cachedContentTokenCount");
    UsageInfo {
        input_tokens: count("promptTokenCount").saturating_sub(cached),
        output_tokens: count("candidatesTokenCount") + count("thoughtsTokenCount"),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
    }
}

/// Fork (Waku): a prompt Gemini refuses comes back as
/// `promptFeedback.blockReason` with no candidate, and so with no finish
/// reason; it read as a stream that broke off and was asked again ten times
/// for the same refusal. It is final, and says why.
fn prompt_block_failure(body: &Value) -> Option<crate::retry_policy::ModelFailure> {
    let feedback = body.get("promptFeedback")?;
    let reason = feedback.get("blockReason").and_then(Value::as_str)?;
    if reason.is_empty() || reason == "BLOCK_REASON_UNSPECIFIED" {
        return None;
    }
    let message = match feedback.get("blockReasonMessage").and_then(Value::as_str) {
        Some(detail) if !detail.trim().is_empty() => {
            format!("Gemini refused the prompt ({reason}): {}", detail.trim())
        }
        _ => format!("Gemini refused the prompt ({reason})"),
    };
    Some(crate::retry_policy::ModelFailure::fatal(None, message))
}

/// Fork (Waku): an `{"error": …}` object inside the stream. Gemini's carries
/// the HTTP `code` it stands for, which classifies it the way the same answer
/// outside a stream would be; without one, its `status` names the kind.
fn stream_error_failure(error: &Value) -> crate::retry_policy::ModelFailure {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    match error.get("code").and_then(Value::as_u64) {
        Some(code @ 400..=599) => crate::retry_policy::classify_status(
            code as u16,
            &json!({ "error": error }).to_string(),
            None,
            None,
        ),
        _ => {
            let kind = error.get("status").and_then(Value::as_str).unwrap_or("");
            crate::retry_policy::classify_stream_error(kind, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::types::Message;
    use serde_json::json;

    fn test_request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "gemini-3-flash-preview".to_string(),
            messages,
            system_prompt: None,
            tools: vec![],
            max_tokens: 512,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: vec![],
            thinking: None,
            provider_options: json!({}),
        }
    }

    #[test]
    fn build_request_body_uses_function_names_for_tool_results() {
        let provider = GoogleProvider::new("test".to_string());
        let request = test_request(vec![
            Message::assistant_blocks(vec![ContentBlock::ToolUse {
                id: "call_search_2".to_string(),
                name: "search".to_string(),
                input: json!({"q": "cats"}),
                thought_signature: None,
            }]),
            Message::user_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_search_2".to_string(),
                content: ToolResultContent::Text("ok".to_string()),
                is_error: Some(false),
            }]),
        ]);

        let body = provider.build_request_body(&request);
        let contents = body["contents"].as_array().expect("contents array");
        assert_eq!(contents.len(), 2);
        assert_eq!(
            contents[1]["parts"][0]["functionResponse"]["name"],
            json!("search")
        );
    }

    #[test]
    fn build_request_body_preserves_tool_result_order() {
        let provider = GoogleProvider::new("test".to_string());
        let request = test_request(vec![Message::user_blocks(vec![
            ContentBlock::Text {
                text: "before".to_string(),
            },
            ContentBlock::ToolResult {
                tool_use_id: "call_search".to_string(),
                content: ToolResultContent::Text("done".to_string()),
                is_error: Some(false),
            },
            ContentBlock::Text {
                text: "after".to_string(),
            },
        ])]);

        let body = provider.build_request_body(&request);
        let contents = body["contents"].as_array().expect("contents array");
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], json!("user"));
        assert_eq!(contents[0]["parts"][0]["text"], json!("before"));
        assert_eq!(contents[1]["parts"][0]["functionResponse"]["name"], json!("search"));
        assert_eq!(contents[2]["parts"][0]["text"], json!("after"));
    }

    #[test]
    fn parse_response_body_assigns_unique_ids_for_duplicate_tool_names() {
        let provider = GoogleProvider::new("test".to_string());
        let response = json!({
            "candidates": [{
                "finishReason": "FUNCTION_CALL",
                "content": {
                    "parts": [
                        { "functionCall": { "name": "search", "args": { "q": "a" } } },
                        { "functionCall": { "name": "search", "args": { "q": "b" } } }
                    ]
                }
            }],
            "usageMetadata": {}
        });

        let parsed = provider
            .parse_response_body(&response, "gemini-3-flash-preview")
            .expect("parsed response");

        let ids: Vec<&str> = parsed
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(ids[0].starts_with("call_search_"));
        assert!(ids[1].ends_with("_2"));
        // Either id still reads back as the function's name.
        for id in &ids {
            assert_eq!(GoogleProvider::infer_tool_name_from_id(id).as_deref(), Some("search"));
        }
    }

    #[test]
    fn tool_ids_differ_between_responses_and_keep_multi_word_names() {
        let first = GoogleProvider::tool_use_id_for_name("web_fetch", "0a1b2c3d", 0);
        let second = GoogleProvider::tool_use_id_for_name("web_fetch", "12345678", 0);
        assert_ne!(first, second);
        assert_eq!(GoogleProvider::infer_tool_name_from_id(&first).as_deref(), Some("web_fetch"));
        assert_eq!(GoogleProvider::infer_tool_name_from_id(&second).as_deref(), Some("web_fetch"));
        let third = GoogleProvider::tool_use_id_for_name("web_fetch", "12345678", 2);
        assert_eq!(GoogleProvider::infer_tool_name_from_id(&third).as_deref(), Some("web_fetch"));
    }

    #[test]
    fn urls_carry_no_key_and_respect_a_versioned_base() {
        let provider = GoogleProvider::new("secret".to_string());
        let url = provider.stream_url("gemini-4-argon");
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-4-argon:streamGenerateContent?alt=sse"
        );
        assert!(!url.contains("secret"));

        let gateway = GoogleProvider::with_gateway("https://cheaprouter.cc/antigravity/", "k".into());
        assert_eq!(
            gateway.generate_url("gemini-4-argon"),
            "https://cheaprouter.cc/antigravity/v1beta/models/gemini-4-argon:generateContent"
        );
        let versioned = GoogleProvider::with_gateway("https://example.com/v1beta", "k".into());
        assert_eq!(
            versioned.stream_url("gemini-4-argon"),
            "https://example.com/v1beta/models/gemini-4-argon:streamGenerateContent?alt=sse"
        );
    }

    /// Parallel calls come one chunk each, every one at `parts[0]` — the
    /// shape the gateway streams for Gemini 4 Argon. Each must be a block of
    /// its own with exactly its own arguments.
    #[tokio::test]
    async fn parallel_calls_in_separate_chunks_keep_their_own_arguments() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let sse = concat!(
            "data: {\"candidates\": [{\"content\": {\"role\": \"model\",\"parts\": [{\"functionCall\": {\"name\": \"Read\",\"args\": {\"file_path\": \"/tmp/a.txt\"},\"id\": \"call_1\"},\"thoughtSignature\": \"sig\"}]}}]}\r\n\r\n",
            "data: {\"candidates\": [{\"content\": {\"role\": \"model\",\"parts\": [{\"functionCall\": {\"name\": \"Read\",\"args\": {\"file_path\": \"/tmp/b.txt\"},\"id\": \"call_2\"}}]}}]}\r\n\r\n",
            "data: {\"candidates\": [{\"content\": {\"role\": \"model\",\"parts\": [{\"text\": \"\"}]},\"finishReason\": \"STOP\"}],\"usageMetadata\": {\"promptTokenCount\": 130,\"candidatesTokenCount\": 50}}\r\n\r\n",
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            // Read the headers and the body the content-length announces.
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..read]);
                let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= end + 4 + length || read == 0 {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                sse.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(sse.as_bytes()).await.unwrap();
        });

        let provider = GoogleProvider::with_gateway(format!("http://{address}"), "k".into());
        let mut request = test_request(vec![Message::user("read both")]);
        request.model = "gemini-4-argon".to_string();
        let mut stream = provider.create_message_stream(request).await.unwrap();

        let mut starts = Vec::new();
        let mut args: std::collections::HashMap<usize, String> = Default::default();
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                StreamEvent::ContentBlockStart {
                    index,
                    content_block: ContentBlock::ToolUse { id, thought_signature, .. },
                } => starts.push((index, id, thought_signature)),
                StreamEvent::InputJsonDelta { index, partial_json } => {
                    args.entry(index).or_default().push_str(&partial_json)
                }
                _ => {}
            }
        }

        assert_eq!(starts.len(), 2, "each functionCall part is its own block");
        assert_ne!(starts[0].0, starts[1].0);
        assert_ne!(starts[0].1, starts[1].1);
        assert_eq!(starts[0].2.as_deref(), Some("sig"));
        assert_eq!(starts[1].2, None);
        let first: Value = serde_json::from_str(&args[&starts[0].0]).unwrap();
        let second: Value = serde_json::from_str(&args[&starts[1].0]).unwrap();
        assert_eq!(first, json!({ "file_path": "/tmp/a.txt" }));
        assert_eq!(second, json!({ "file_path": "/tmp/b.txt" }));
    }

    #[test]
    fn thought_parts_are_thinking_and_usage_counts_thoughts_and_cache() {
        let provider = GoogleProvider::new("test".to_string());
        let response = json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [
                    { "text": "weighing options", "thought": true },
                    { "text": "answer" }
                ]}
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "cachedContentTokenCount": 40,
                "candidatesTokenCount": 7,
                "thoughtsTokenCount": 30
            }
        });
        let parsed = provider
            .parse_response_body(&response, "gemini-4-argon")
            .expect("parsed response");
        assert!(matches!(&parsed.content[0], ContentBlock::Thinking { thinking, .. } if thinking == "weighing options"));
        assert!(matches!(&parsed.content[1], ContentBlock::Text { text } if text == "answer"));
        assert_eq!(parsed.usage.input_tokens, 60);
        assert_eq!(parsed.usage.cache_read_input_tokens, 40);
        assert_eq!(parsed.usage.output_tokens, 37);
    }

    #[test]
    fn gemini_4_supports_thinking() {
        assert!(GoogleProvider::supports_thinking("gemini-4-argon"));
    }

    #[test]
    fn a_refused_prompt_is_final_and_names_the_reason() {
        let blocked = json!({
            "promptFeedback": {
                "blockReason": "SAFETY",
                "blockReasonMessage": "The prompt is blocked due to safety"
            }
        });
        let failure = prompt_block_failure(&blocked).expect("a blocked prompt fails");
        assert_eq!(failure.class, crate::retry_policy::FailureClass::Fatal);
        assert!(failure.message.contains("SAFETY"));
        assert!(failure.message.contains("blocked due to safety"));

        let provider = GoogleProvider::new("k".into());
        assert!(matches!(
            provider.parse_response_body(&blocked, "gemini-4-argon"),
            Err(ProviderError::Classified { .. })
        ));
        assert!(prompt_block_failure(&json!({ "candidates": [] })).is_none());
        assert!(prompt_block_failure(&json!({
            "promptFeedback": { "blockReason": "BLOCK_REASON_UNSPECIFIED" }
        }))
        .is_none());
    }

    #[test]
    fn in_stream_errors_classify_by_their_http_code() {
        let rate_limited = stream_error_failure(&json!({
            "code": 429, "message": "Resource exhausted", "status": "RESOURCE_EXHAUSTED"
        }));
        assert!(rate_limited.is_retryable());
        let bad_request = stream_error_failure(&json!({
            "code": 400, "message": "Invalid argument", "status": "INVALID_ARGUMENT"
        }));
        assert!(!bad_request.is_retryable());
    }

    #[test]
    fn thought_signature_from_part_captures_streaming_signature() {
        // The streaming SSE parser lives inside an async network generator with
        // no test seam, but it extracts the signature through this exact helper.
        // A streamed functionCall part carries `thoughtSignature` as a sibling of
        // `functionCall`, identical to the non-streaming shape (#311).
        let streamed_part = json!({
            "functionCall": { "name": "read", "args": { "path": "a.rs" } },
            "thoughtSignature": "SIG_STREAM_1"
        });
        assert_eq!(
            thought_signature_from_part(&streamed_part).as_deref(),
            Some("SIG_STREAM_1"),
            "streamed functionCall part's signature is captured"
        );

        // A part without a signature (older/non-thinking models) yields None.
        let unsigned_part = json!({
            "functionCall": { "name": "read", "args": {} }
        });
        assert!(
            thought_signature_from_part(&unsigned_part).is_none(),
            "absent signature stays None"
        );
    }

    #[test]
    fn parse_response_body_captures_thought_signature() {
        // Gemini 3.x thinking models attach a camelCase `thoughtSignature` as a
        // sibling of `functionCall` on the part; it must be captured (#311).
        let provider = GoogleProvider::new("test".to_string());
        let response = json!({
            "candidates": [{
                "finishReason": "FUNCTION_CALL",
                "content": {
                    "parts": [
                        {
                            "functionCall": { "name": "read", "args": { "path": "a.rs" } },
                            "thoughtSignature": "SIG_ABC123"
                        }
                    ]
                }
            }],
            "usageMetadata": {}
        });

        let parsed = provider
            .parse_response_body(&response, "gemini-3.1-pro-preview")
            .expect("parsed response");

        match &parsed.content[0] {
            ContentBlock::ToolUse {
                name,
                thought_signature,
                ..
            } => {
                assert_eq!(name, "read");
                assert_eq!(thought_signature.as_deref(), Some("SIG_ABC123"));
            }
            other => panic!("expected ToolUse block, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_body_omits_signature_when_absent() {
        // A functionCall part without a signature (older/non-thinking models)
        // yields a ToolUse with `thought_signature == None` (old behaviour).
        let provider = GoogleProvider::new("test".to_string());
        let response = json!({
            "candidates": [{
                "finishReason": "FUNCTION_CALL",
                "content": {
                    "parts": [
                        { "functionCall": { "name": "read", "args": {} } }
                    ]
                }
            }],
            "usageMetadata": {}
        });

        let parsed = provider
            .parse_response_body(&response, "gemini-2.0-flash")
            .expect("parsed response");

        assert!(matches!(
            &parsed.content[0],
            ContentBlock::ToolUse { thought_signature, .. } if thought_signature.is_none()
        ));
    }

    #[test]
    fn build_request_body_round_trips_thought_signature_onto_function_call() {
        // A captured signature must ride back as a camelCase sibling of the
        // functionCall part, or Gemini 3.x rejects the turn with HTTP 400 (#311).
        let provider = GoogleProvider::new("test".to_string());
        let request = test_request(vec![Message::assistant_blocks(vec![
            ContentBlock::ToolUse {
                id: "call_read".to_string(),
                name: "read".to_string(),
                input: json!({ "path": "a.rs" }),
                thought_signature: Some("SIG_ABC123".to_string()),
            },
        ])]);

        let body = provider.build_request_body(&request);
        let contents = body["contents"].as_array().expect("contents array");
        let part = &contents[0]["parts"][0];
        assert_eq!(part["functionCall"]["name"], json!("read"));
        assert_eq!(part["functionCall"]["args"], json!({ "path": "a.rs" }));
        assert_eq!(part["thoughtSignature"], json!("SIG_ABC123"));
    }

    #[test]
    fn build_request_body_without_signature_omits_thought_signature_key() {
        // Absent signature keeps the old wire shape: no thoughtSignature key.
        let provider = GoogleProvider::new("test".to_string());
        let request = test_request(vec![Message::assistant_blocks(vec![
            ContentBlock::ToolUse {
                id: "call_read".to_string(),
                name: "read".to_string(),
                input: json!({ "path": "a.rs" }),
                thought_signature: None,
            },
        ])]);

        let body = provider.build_request_body(&request);
        let part = &body["contents"][0]["parts"][0];
        assert_eq!(part["functionCall"]["name"], json!("read"));
        assert!(
            part.get("thoughtSignature").is_none(),
            "absent signature must not emit a thoughtSignature key"
        );
    }

    #[test]
    fn thought_signature_survives_content_block_serde_round_trip() {
        // The signature must persist across session save/load (JSON round-trip).
        let block = ContentBlock::ToolUse {
            id: "call_read".to_string(),
            name: "read".to_string(),
            input: json!({ "path": "a.rs" }),
            thought_signature: Some("SIG_PERSIST".to_string()),
        };
        let encoded = serde_json::to_string(&block).expect("serialize");
        assert!(encoded.contains("thought_signature"));
        let decoded: ContentBlock = serde_json::from_str(&encoded).expect("deserialize");
        assert!(matches!(
            decoded,
            ContentBlock::ToolUse { thought_signature, .. }
                if thought_signature.as_deref() == Some("SIG_PERSIST")
        ));
    }
}
