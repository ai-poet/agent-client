//! Image generation through the gateway, as an MCP tool on the REPL server.
//!
//! It lives here rather than in the bridge because this server is already
//! registered with every agent that has Computer Use on — the built-in one
//! and six CLIs — so one implementation reaches all of them, and the picture
//! arrives as a real MCP image block rather than a string the model has to
//! imagine.
//!
//! Credentials are read from the engine's own `settings.json` rather than
//! passed in the environment: Codex registers this server with `-c
//! mcp_servers.waku_js_repl.env.X=…`, which would put a gateway key on a
//! command line for anything on the machine to read.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value as JsonValue, json};

/// Where to send the request, and what to send it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageGateway {
    /// The gateway origin, without the `/v1` the OpenAI path adds.
    pub(crate) origin: String,
    pub(crate) api_key: String,
}

/// What the model asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ImageRequest {
    pub(crate) prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) size: Option<String>,
    pub(crate) quality: Option<String>,
    pub(crate) background: Option<String>,
    pub(crate) output_format: Option<String>,
    pub(crate) count: Option<u8>,
    pub(crate) output_dir: Option<PathBuf>,
}

/// The gateway's default when no model is named.
const DEFAULT_MODEL: &str = "gpt-image-2";

/// Image requests are slow — a large one runs well past a chat turn.
const TIMEOUT_SECONDS: u32 = 180;

/// Find the gateway this machine is signed in to.
///
/// `None` means there is nothing to call: either the user is signed out, or
/// the built-in agent is routed at an endpoint of their own, where the key
/// table is deliberately empty so a gateway key never reaches somebody
/// else's server. The tool is then not offered at all, rather than offered
/// and failing.
pub(crate) fn resolve_gateway() -> Option<ImageGateway> {
    if let (Ok(origin), Ok(api_key)) = (
        std::env::var("WAKU_IMAGE_GATEWAY_ORIGIN"),
        std::env::var("WAKU_IMAGE_GATEWAY_KEY"),
    ) && !origin.is_empty()
        && !api_key.is_empty()
    {
        return Some(ImageGateway { origin, api_key });
    }
    let path = sub2api::global_config::native::config_dir()?.join("settings.json");
    let document: JsonValue = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    gateway_from_settings(&document)
}

/// The route and key an image call needs, read out of the engine's settings.
///
/// The gateway dispatches images on the *key's* group platform, so the key
/// matters more than usual: an OpenAI-platform group has the endpoint, an
/// Anthropic one answers 404. That is why this never falls through to
/// `anthropic` the way the session's own route resolution does.
pub(crate) fn gateway_from_settings(document: &JsonValue) -> Option<ImageGateway> {
    let anthropic = document.pointer("/config/provider_configs/anthropic")?;
    let origin = anthropic.get("api_base")?.as_str()?.trim();
    let keys = anthropic.pointer("/options/gateway_keys")?.as_object()?;
    let api_key = ["openai", "default"]
        .iter()
        .find_map(|platform| keys.get(*platform)?.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())?;
    (!origin.is_empty()).then(|| ImageGateway {
        origin: origin.to_owned(),
        api_key: api_key.to_owned(),
    })
}

/// The JSON body for one generation.
pub(crate) fn request_body(request: &ImageRequest) -> JsonValue {
    let mut body = json!({
        "model": request.model.as_deref().unwrap_or(DEFAULT_MODEL),
        "prompt": request.prompt,
        // Always ask for bytes. A `url` here is a data URL anyway, and the
        // bytes are what gets written to disk and shown in the transcript.
        "response_format": "b64_json",
    });
    let object = body.as_object_mut().expect("a JSON object");
    if let Some(count) = request.count {
        object.insert("n".into(), json!(count.clamp(1, 4)));
    }
    for (field, value) in [
        ("size", &request.size),
        ("quality", &request.quality),
        ("background", &request.background),
        ("output_format", &request.output_format),
    ] {
        if let Some(value) = value {
            object.insert(field.into(), json!(value));
        }
    }
    body
}

/// One generated picture.
pub(crate) struct GeneratedImage {
    pub(crate) path: PathBuf,
    pub(crate) mime: String,
    pub(crate) base64: String,
    pub(crate) revised_prompt: Option<String>,
}

/// Turn a gateway response into pictures on disk.
pub(crate) fn decode_response(
    response: &JsonValue,
    output_dir: &Path,
    output_format: Option<&str>,
) -> Result<Vec<GeneratedImage>> {
    let data = response
        .get("data")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| anyhow!("the gateway returned no `data` array"))?;
    if data.is_empty() {
        bail!("the gateway returned no images");
    }
    let extension = match output_format.unwrap_or("png") {
        "jpeg" | "jpg" => "jpg",
        "webp" => "webp",
        _ => "png",
    };
    let mime = format!("image/{}", if extension == "jpg" { "jpeg" } else { extension });
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("could not create {}", output_dir.display()))?;

    let mut images = Vec::new();
    for (index, item) in data.iter().enumerate() {
        let base64 = item
            .get("b64_json")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| anyhow!("image {} came back without bytes", index + 1))?;
        let bytes = base64_decode(base64)
            .with_context(|| format!("image {} was not valid base64", index + 1))?;
        let path = output_dir.join(format!("{stamp}-{}.{extension}", index + 1));
        std::fs::write(&path, &bytes)
            .with_context(|| format!("could not write {}", path.display()))?;
        images.push(GeneratedImage {
            path,
            mime: mime.clone(),
            base64: base64.to_owned(),
            revised_prompt: item
                .get("revised_prompt")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
        });
    }
    Ok(images)
}

fn base64_decode(encoded: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(Into::into)
}

/// What a failing status actually means to the person who asked for a
/// picture. The gateway's own message is kept where it says something
/// specific; these add the part it cannot know.
pub(crate) fn describe_failure(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<JsonValue>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.trim().chars().take(300).collect());
    match status {
        401 => format!("the gateway rejected the key ({detail})"),
        403 => format!(
            "this account's group is not allowed to generate images ({detail})"
        ),
        404 => format!(
            "this account's group has no image endpoint — image generation needs an \
             OpenAI or Grok group, and the signed-in key belongs to a different one \
             ({detail})"
        ),
        _ => format!("the gateway refused the request with HTTP {status} ({detail})"),
    }
}

/// Call the gateway and write what comes back.
pub(crate) fn generate(
    gateway: &ImageGateway,
    request: &ImageRequest,
) -> Result<Vec<GeneratedImage>> {
    let url = format!(
        "{}/images/generations",
        sub2api::gateway::openai_base_url(&gateway.origin)
    );
    // `sub2api::http` is the fork's own client: synchronous, which is what
    // this stdio server wants, and it passes headers to curl on stdin so the
    // key never appears in the process table.
    let response = sub2api::http::Request::new()
        .timeout_seconds(TIMEOUT_SECONDS)
        .bearer(&gateway.api_key)
        .json_body(request_body(request).to_string())
        .send(&url)
        .with_context(|| format!("could not reach {url}"))?;

    if !(200..300).contains(&response.status) {
        bail!("{}", describe_failure(response.status, &response.body));
    }
    let parsed: JsonValue =
        serde_json::from_str(&response.body).context("the gateway's reply was not JSON")?;
    let output_dir = request
        .output_dir
        .clone()
        .unwrap_or_else(default_output_directory);
    decode_response(&parsed, &output_dir, request.output_format.as_deref())
}

/// Where pictures land when the caller names no directory: beside the work,
/// so the model can refer to them by path and the user can find them.
fn default_output_directory() -> PathBuf {
    std::env::var_os("WAKU_SESSION_CWD")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("generated-images")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(keys: JsonValue) -> JsonValue {
        json!({
            "config": {
                "provider_configs": {
                    "anthropic": {
                        "api_base": "https://gw.example.org",
                        "options": { "gateway_keys": keys }
                    }
                }
            }
        })
    }

    /// The gateway dispatches on the key's group platform, so the OpenAI key
    /// is the one with an image endpoint behind it.
    #[test]
    fn the_openai_key_is_preferred_and_anthropic_is_never_used() {
        let all = settings(json!({"anthropic": "sk-a", "openai": "sk-o", "default": "sk-d"}));
        assert_eq!(
            gateway_from_settings(&all).unwrap().api_key,
            "sk-o",
            "the OpenAI-group key has the endpoint"
        );

        let fallback = settings(json!({"anthropic": "sk-a", "default": "sk-d"}));
        assert_eq!(gateway_from_settings(&fallback).unwrap().api_key, "sk-d");

        // Anthropic alone is not a fallback: that group answers 404, so
        // offering the tool would only produce a confusing failure.
        let anthropic_only = settings(json!({"anthropic": "sk-a"}));
        assert_eq!(gateway_from_settings(&anthropic_only), None);
    }

    /// A route pointed at the user's own endpoint has an empty key table by
    /// design, so there is nothing to call.
    #[test]
    fn a_custom_endpoint_has_no_gateway_to_reach() {
        assert_eq!(gateway_from_settings(&settings(json!({}))), None);
        assert_eq!(gateway_from_settings(&json!({"config": {}})), None);
    }

    #[test]
    fn the_body_defaults_to_the_gateways_model_and_asks_for_bytes() {
        let body = request_body(&ImageRequest {
            prompt: "a quiet harbour".into(),
            ..ImageRequest::default()
        });
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["prompt"], "a quiet harbour");
        assert_eq!(body["response_format"], "b64_json");
        // Nothing the caller did not ask for.
        assert!(body.get("size").is_none());
        assert!(body.get("n").is_none());
    }

    #[test]
    fn the_count_is_clamped_to_what_the_gateway_accepts() {
        let body = request_body(&ImageRequest {
            prompt: "x".into(),
            count: Some(99),
            size: Some("1024x1024".into()),
            ..ImageRequest::default()
        });
        assert_eq!(body["n"], 4);
        assert_eq!(body["size"], "1024x1024");
    }

    #[test]
    fn a_response_becomes_files_named_for_the_format() {
        let dir = std::env::temp_dir().join(format!("waku-img-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // "hello" in base64.
        let response = json!({
            "data": [
                {"b64_json": "aGVsbG8=", "revised_prompt": "a quiet harbour at dawn"},
                {"b64_json": "aGVsbG8="}
            ]
        });
        let images = decode_response(&response, &dir, Some("jpeg")).expect("decode");
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].mime, "image/jpeg");
        assert!(images[0].path.extension().is_some_and(|ext| ext == "jpg"));
        assert_eq!(std::fs::read(&images[0].path).unwrap(), b"hello");
        assert_eq!(
            images[0].revised_prompt.as_deref(),
            Some("a quiet harbour at dawn")
        );
        assert!(images[1].revised_prompt.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_reply_is_an_error_rather_than_an_empty_success() {
        let dir = std::env::temp_dir().join("waku-img-none");
        assert!(decode_response(&json!({"data": []}), &dir, None).is_err());
        assert!(decode_response(&json!({}), &dir, None).is_err());
    }

    /// The three statuses a user actually hits, each meaning something they
    /// can act on.
    #[test]
    fn failures_say_which_of_them_it_was() {
        let body = r#"{"error":{"message":"nope"}}"#;
        assert!(describe_failure(401, body).contains("rejected the key"));
        assert!(describe_failure(403, body).contains("not allowed"));
        let missing = describe_failure(404, body);
        assert!(missing.contains("OpenAI or Grok group"), "{missing}");
        // The gateway's own words survive.
        assert!(missing.contains("nope"));
        // An unexpected status still says what happened.
        assert!(describe_failure(500, "boom").contains("500"));
    }
}
