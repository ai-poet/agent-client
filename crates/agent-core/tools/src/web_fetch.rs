// WebFetch tool: HTTP GET with HTML-to-text conversion and LLM-powered semantic extraction
// for edge cases (JS-heavy pages, minimal content).

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, warn};

pub struct WebFetchTool;

#[derive(Debug, Deserialize)]
struct WebFetchInput {
    url: String,
    #[serde(default)]
    #[allow(dead_code)]
    prompt: Option<String>,
}

/// Compute a simple hash of the URL for cache purposes.
fn url_hash(url: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Get the cache directory for web_fetch content.
fn get_cache_dir() -> PathBuf {
    agent_core::config::Settings::config_dir().join("web_cache")
}

/// Attempt to load cached extracted content for a URL.
fn load_cached_extraction(url: &str) -> Option<String> {
    let cache_dir = get_cache_dir();
    let cache_file = cache_dir.join(format!("{}.txt", url_hash(url)));

    if cache_file.exists() {
        match fs::read_to_string(&cache_file) {
            Ok(content) => {
                debug!(file = ?cache_file, "Loaded cached web content");
                return Some(content);
            }
            Err(e) => {
                debug!(file = ?cache_file, error = %e, "Failed to load cache");
            }
        }
    }
    None
}

/// Save extracted content to cache.
fn save_cached_extraction(url: &str, content: &str) {
    let cache_dir = get_cache_dir();
    if let Err(e) = fs::create_dir_all(&cache_dir) {
        warn!(dir = ?cache_dir, error = %e, "Failed to create cache directory");
        return;
    }

    let cache_file = cache_dir.join(format!("{}.txt", url_hash(url)));
    if let Err(e) = fs::write(&cache_file, content) {
        warn!(file = ?cache_file, error = %e, "Failed to write cache file");
    } else {
        debug!(file = ?cache_file, "Cached extracted web content");
    }
}

/// Detect if HTML is likely a JS-heavy page with minimal semantic content.
fn is_edge_case_html(html: &str, extracted_text: &str) -> bool {
    // Check word count (rough estimate)
    let word_count = extracted_text.split_whitespace().count();
    if word_count < 100 {
        debug!(word_count, "Edge case: low word count");
        return true;
    }

    // Check for semantic HTML tags
    let lower = html.to_lowercase();
    let has_semantic =
        lower.contains("<article") || lower.contains("<main") || lower.contains("<body");

    if !has_semantic {
        debug!("Edge case: no semantic HTML tags");
        return true;
    }

    false
}

/// Call Claude Haiku to extract main content from HTML.
async fn semantic_extraction(html: &str, ctx: &ToolContext) -> Option<String> {
    // Try to create an Anthropic client from the config
    let client = match agent_core_api::AnthropicClient::from_config(&ctx.config) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to create Anthropic client for semantic extraction");
            return None;
        }
    };

    // Truncate HTML to avoid exceeding token limits
    // Fork departure (Waku): on a character boundary, as the other cuts in
    // this file are — a byte slice panicked on any non-ASCII page.
    let html_excerpt = if html.len() > 20000 {
        format!(
            "{}...",
            &html[..crate::pty_bash::floor_char_boundary(html, 20000)]
        )
    } else {
        html.to_string()
    };

    let system = "You are a content extraction expert. Given HTML, extract and return only the main text content. Return just plain text, no markdown or formatting.";
    let user_message = format!(
        "Extract the main content from this HTML and return only the text:\n\n{}",
        html_excerpt
    );

    // Use the builder API to construct the request
    let api_messages = vec![agent_core_api::ApiMessage {
        role: "user".to_string(),
        content: serde_json::Value::String(user_message),
    }];

    let request = agent_core_api::CreateMessageRequest::builder("claude-haiku-4-5", 2000)
        .messages(api_messages)
        .system(agent_core_api::SystemPrompt::Text(system.to_string()))
        .build();

    match client.create_message(request).await {
        Ok(response) => {
            // Extract text from the response content (Vec<Value>)
            // Response content is JSON objects like {"type": "text", "text": "..."}
            let text = response.content.iter().find_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str().map(str::to_owned)
                } else {
                    None
                }
            });

            if let Some(extracted) = text {
                debug!(
                    extracted_len = extracted.len(),
                    "Semantic extraction successful"
                );
                return Some(extracted);
            }

            warn!("No text block in semantic extraction response");
            None
        }
        Err(e) => {
            warn!(error = %e, "Semantic extraction API call failed");
            None
        }
    }
}

/// Naively strip HTML tags and decode common entities.
fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    let lower = html.to_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if !in_tag && chars[i] == '<' {
            in_tag = true;
            // Check for script/style open/close
            let rest: String = lower_chars[i..].iter().take(20).collect();
            if rest.starts_with("<script") {
                in_script = true;
            } else if rest.starts_with("</script") {
                in_script = false;
            } else if rest.starts_with("<style") {
                in_style = true;
            } else if rest.starts_with("</style") {
                in_style = false;
            }
            // Block tags => newline
            let block_tags = [
                "<br", "<p ", "<p>", "</p>", "<div", "</div>", "<h1", "<h2", "<h3", "<h4", "<h5",
                "<h6", "</h1", "</h2", "</h3", "</h4", "</h5", "</h6", "<li", "</li", "<tr",
                "</tr", "<hr",
            ];
            for tag in &block_tags {
                if rest.starts_with(tag) {
                    result.push('\n');
                    break;
                }
            }
            i += 1;
            continue;
        }

        if in_tag {
            if chars[i] == '>' {
                in_tag = false;
            }
            i += 1;
            continue;
        }

        if in_script || in_style {
            i += 1;
            continue;
        }

        // Decode basic entities
        if chars[i] == '&' {
            let rest: String = chars[i..].iter().take(10).collect();
            if rest.starts_with("&amp;") {
                result.push('&');
                i += 5;
            } else if rest.starts_with("&lt;") {
                result.push('<');
                i += 4;
            } else if rest.starts_with("&gt;") {
                result.push('>');
                i += 4;
            } else if rest.starts_with("&quot;") {
                result.push('"');
                i += 6;
            } else if rest.starts_with("&#39;") || rest.starts_with("&apos;") {
                result.push('\'');
                i += if rest.starts_with("&#39;") { 5 } else { 6 };
            } else if rest.starts_with("&nbsp;") {
                result.push(' ');
                i += 6;
            } else {
                result.push('&');
                i += 1;
            }
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    // Collapse multiple blank lines
    let mut collapsed = String::new();
    let mut blank_count = 0;
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                collapsed.push('\n');
            }
        } else {
            blank_count = 0;
            collapsed.push_str(trimmed);
            collapsed.push('\n');
        }
    }

    collapsed.trim().to_string()
}

const MAX_REDIRECTS: usize = 5;
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

fn redirect_target(current: &reqwest::Url, location: &str) -> Result<reqwest::Url, String> {
    current
        .join(location)
        .map_err(|error| format!("Invalid redirect URL: {error}"))
}

fn public_address(address: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            let blocked = address.is_loopback()
                || address.is_private()
                || address.is_link_local()
                || address.is_multicast()
                || address.is_broadcast()
                || address.is_unspecified()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
                || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
                || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
                || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
                || (octets[0] == 198 && (18..=19).contains(&octets[1]))
                || octets[0] >= 224;
            !blocked
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return public_address(IpAddr::V4(mapped));
            }
            let segments = address.segments();
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0xdb8)
                || (segments[0] == 0x2001 && segments[1] == 0))
        }
    }
}

async fn validate_url(value: &str) -> Result<(reqwest::Url, Vec<std::net::SocketAddr>), String> {
    let mut url = reqwest::Url::parse(value).map_err(|error| format!("Invalid URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only HTTP and HTTPS URLs are allowed".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with embedded credentials are not allowed".into());
    }
    url.set_fragment(None);
    let host = url.host_str().ok_or_else(|| "URL has no host".to_owned())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL has no port".to_owned())?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("DNS lookup failed: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_address(address.ip()))
    {
        return Err("The URL resolves to a non-public address".into());
    }
    Ok((url, addresses))
}

async fn read_body(response: reqwest::Response) -> Result<String, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err("Response exceeds the 10 MiB limit".into());
    }
    if response
        .headers()
        .get("content-encoding")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err("Encoded responses are not supported".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Failed to read response: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err("Response exceeds the 10 MiB limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn fetch_body(url: &str) -> Result<(String, String), String> {
    let mut current = reqwest::Url::parse(url).map_err(|error| format!("Invalid URL: {error}"))?;
    for redirects in 0..=MAX_REDIRECTS {
        let (url, addresses) = validate_url(current.as_str()).await?;
        let host = url.host_str().ok_or_else(|| "URL has no host".to_owned())?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|error| format!("Failed to create HTTP client: {error}"))?;
        let response = client
            .get(url.clone())
            .header("User-Agent", "Claude-Code/1.0")
            .send()
            .await
            .map_err(|error| format!("Failed to fetch {url}: {error}"))?;
        if response.status().is_redirection() {
            if redirects == MAX_REDIRECTS {
                return Err("Too many redirects".into());
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Redirect has no location".to_owned())?;
            current = redirect_target(&url, location)?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("HTTP {} when fetching {url}", response.status()));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !content_type.is_empty()
            && !content_type.starts_with("text/")
            && !["json", "xml", "html", "xhtml", "javascript"]
                .iter()
                .any(|kind| content_type.contains(kind))
        {
            return Err(format!("Unsupported response content type: {content_type}"));
        }
        let body = read_body(response).await?;
        return Ok((body, content_type));
    }
    Err("Too many redirects".into())
}

#[async_trait]
impl Tool for WebFetchTool {
    // Gates itself: calls `ctx.check_permission` in `execute()` (#210).
    fn self_gates(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        agent_core::constants::TOOL_NAME_WEB_FETCH
    }

    fn description(&self) -> &str {
        "Fetches a web page URL and returns its content as text. HTML is \
         automatically converted to plain text. Use this for reading documentation, \
         APIs, and other web resources."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional prompt for how to process the content"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: WebFetchInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        // Permission check
        if let Err(e) = ctx.check_permission(
            self.name(),
            &format!("Fetch {}", params.url),
            true, // read-only
        ) {
            return ToolResult::error(e.to_string());
        }

        debug!(url = %params.url, "Fetching web page");

        let (body, content_type) = match fetch_body(&params.url).await {
            Ok(result) => result,
            Err(error) => return ToolResult::error(error),
        };

        // Try to load from cache first
        if let Some(cached) = load_cached_extraction(&params.url) {
            return ToolResult::success(cached);
        }

        // Convert HTML to text if applicable
        let mut text = if content_type.contains("html") {
            strip_html(&body)
        } else {
            body.clone()
        };

        // Detect and handle edge cases with semantic extraction
        if content_type.contains("html") && is_edge_case_html(&body, &text) {
            debug!(url = %params.url, "Attempting semantic extraction for edge case");
            if let Some(extracted) = semantic_extraction(&body, ctx).await {
                text = extracted;
            } else {
                debug!("Semantic extraction failed, using basic HTML stripping");
            }
        }

        // Truncate very long content
        const MAX_LEN: usize = 100_000;
        let text = if text.len() > MAX_LEN {
            // Fork departure (Waku): character boundary, not byte offset. A
            // page in any non-Latin script would otherwise panic here.
            format!(
                "{}\n\n... (truncated, {} total characters)",
                &text[..crate::pty_bash::floor_char_boundary(&text, MAX_LEN)],
                text.len()
            )
        } else {
            text
        };

        // Cache the final result
        save_cached_extraction(&params.url, &text);

        ToolResult::success(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_public_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "::1",
            "fc00::1",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        assert!(public_address("1.1.1.1".parse().unwrap()));
        assert!(public_address("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn redirects_resolve_against_the_current_url() {
        let current = reqwest::Url::parse("https://example.com/docs/page").unwrap();
        assert_eq!(
            redirect_target(&current, "/next").unwrap().as_str(),
            "https://example.com/next"
        );
    }
}
