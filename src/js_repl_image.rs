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
//!
//! The request itself is `sub2api::images`, shared with the app's image
//! studio: a task where the gateway has them, otherwise one streamed call —
//! either way nothing waits on a single response for minutes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde_json::Value as JsonValue;
use sub2api::images::{self, ImageError, ImageErrorKind, ImageRoute, ImageSpec};

/// Where to send the request, and what to send it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageGateway {
    /// The gateway origin, without the `/v1` the OpenAI path adds.
    pub(crate) origin: String,
    pub(crate) api_key: String,
    /// Image models' own keys from the routing table: the key of the group
    /// the image studio found each one draws in.
    pub(crate) model_keys: BTreeMap<String, String>,
}

impl ImageGateway {
    /// The key for `model`: its own when routing gave it one, the OpenAI
    /// group's otherwise.
    pub(crate) fn key_for(&self, model: &str) -> &str {
        self.model_keys
            .get(model)
            .map(String::as_str)
            .filter(|key| !key.trim().is_empty())
            .unwrap_or(&self.api_key)
    }
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

impl ImageRequest {
    fn spec(&self) -> ImageSpec {
        ImageSpec {
            model: self.model.clone().unwrap_or_default(),
            prompt: self.prompt.clone(),
            size: self.size.clone(),
            quality: self.quality.clone(),
            background: self.background.clone(),
            output_format: self.output_format.clone(),
            count: self.count.unwrap_or(1),
            references: Vec::new(),
        }
    }
}

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
        return Some(ImageGateway {
            origin,
            api_key,
            model_keys: BTreeMap::new(),
        });
    }
    let path = sub2api::global_config::native::config_dir()?.join("settings.json");
    let document: JsonValue = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    gateway_from_settings(&document)
}

/// The route and keys an image call needs, read out of the engine's
/// settings.
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
    let model_keys = keys
        .get("models")
        .and_then(JsonValue::as_object)
        .into_iter()
        .flatten()
        .filter(|(model, _)| images::is_image_model(model))
        .filter_map(|(model, key)| Some((model.clone(), key.as_str()?.trim().to_owned())))
        .collect();
    (!origin.is_empty()).then(|| ImageGateway {
        origin: origin.to_owned(),
        api_key: api_key.to_owned(),
        model_keys,
    })
}

/// One generated picture.
pub(crate) struct GeneratedImage {
    pub(crate) path: PathBuf,
    pub(crate) mime: String,
    pub(crate) base64: String,
    pub(crate) revised_prompt: Option<String>,
}

/// Write what the gateway drew into `output_dir`, named by the time and
/// their order, and read each back for the transcript.
pub(crate) fn save_images(
    outputs: &[images::ImageOutput],
    output_dir: &Path,
) -> Result<Vec<GeneratedImage>> {
    use base64::Engine as _;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut saved = Vec::new();
    for (index, output) in outputs.iter().enumerate() {
        let path = images::save_output(output, &output_dir.join(format!("{stamp}-{}", index + 1)))?;
        let bytes =
            std::fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
        let mime = match path.extension().and_then(|extension| extension.to_str()) {
            Some("jpg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/png",
        };
        saved.push(GeneratedImage {
            path,
            mime: mime.to_owned(),
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            revised_prompt: output.revised_prompt.clone(),
        });
    }
    Ok(saved)
}

/// What a failure actually means to the person who asked for a picture.
/// The gateway's own message is kept; these add the part it cannot know.
pub(crate) fn describe_failure(error: &ImageError) -> String {
    let detail = &error.message;
    match error.kind {
        ImageErrorKind::Unauthorized => format!("the gateway rejected the key ({detail})"),
        ImageErrorKind::NoImagePermission => format!(
            "this account's group is not allowed to generate images — drawing once in the \
             app's image studio finds a group that is and routes this tool there too \
             ({detail})"
        ),
        ImageErrorKind::WrongPlatform => format!(
            "this account's group has no image endpoint — image generation needs an \
             OpenAI or Grok group, and the signed-in key belongs to a different one \
             ({detail})"
        ),
        ImageErrorKind::ContentPolicy => {
            format!("the request was blocked by moderation ({detail})")
        }
        ImageErrorKind::OwnBalance => format!("the account's balance is too low ({detail})"),
        _ => match error.status {
            Some(status) => {
                format!("the gateway refused the request with HTTP {status} ({detail})")
            }
            None => format!("image generation failed ({detail})"),
        },
    }
}

/// Call the gateway and write what comes back.
pub(crate) fn generate(
    gateway: &ImageGateway,
    request: &ImageRequest,
) -> Result<Vec<GeneratedImage>> {
    let spec = request.spec();
    let route = ImageRoute {
        origin: gateway.origin.clone(),
        api_key: gateway.key_for(spec.model()).to_owned(),
    };
    let outputs = images::generate_blocking(&route, &spec)
        .map_err(|error| anyhow!(describe_failure(&error)))?;
    let output_dir = request
        .output_dir
        .clone()
        .unwrap_or_else(default_output_directory);
    save_images(&outputs, &output_dir)
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
    use serde_json::json;

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

    /// An image model routed to its own group — the one the image studio
    /// found it draws in — is sent with that group's key.
    #[test]
    fn an_image_model_uses_its_routed_key() {
        let routed = settings(json!({
            "openai": "sk-o",
            "models": {"gpt-image-2": "sk-img", "gpt-5.2": "sk-chat"}
        }));
        let gateway = gateway_from_settings(&routed).unwrap();
        assert_eq!(gateway.key_for("gpt-image-2"), "sk-img");
        assert_eq!(gateway.key_for("gpt-image-1"), "sk-o");
        assert!(
            !gateway.model_keys.contains_key("gpt-5.2"),
            "chat routes are not image routes"
        );
    }

    /// A route pointed at the user's own endpoint has an empty key table by
    /// design, so there is nothing to call.
    #[test]
    fn a_custom_endpoint_has_no_gateway_to_reach() {
        assert_eq!(gateway_from_settings(&settings(json!({}))), None);
        assert_eq!(gateway_from_settings(&json!({"config": {}})), None);
    }

    #[test]
    fn the_request_becomes_a_spec_with_the_gateways_default_model() {
        let spec = ImageRequest {
            prompt: "a quiet harbour".into(),
            count: Some(9),
            ..ImageRequest::default()
        }
        .spec();
        assert_eq!(spec.model(), images::DEFAULT_MODEL);
        assert_eq!(spec.count(), images::MAX_COUNT);
        assert!(!spec.is_edit());
    }

    #[test]
    fn pictures_are_written_and_read_back_for_the_transcript() {
        let dir = std::env::temp_dir().join(format!("waku-img-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let outputs = vec![
            images::ImageOutput {
                // A JPEG's first bytes.
                source: images::ImageSource::Base64("/9j/4AAQ".into()),
                revised_prompt: Some("a quiet harbour at dawn".into()),
            },
            images::ImageOutput {
                source: images::ImageSource::Base64("aGVsbG8=".into()),
                revised_prompt: None,
            },
        ];
        let saved = save_images(&outputs, &dir).expect("save");
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].mime, "image/jpeg");
        assert!(saved[0].path.extension().is_some_and(|ext| ext == "jpg"));
        assert_eq!(
            saved[0].revised_prompt.as_deref(),
            Some("a quiet harbour at dawn")
        );
        // Bytes of no known format are kept as PNG.
        assert_eq!(saved[1].mime, "image/png");
        assert_eq!(std::fs::read(&saved[1].path).unwrap(), b"hello");
        assert_eq!(saved[1].base64, "aGVsbG8=");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The failures a user actually hits, each meaning something they can
    /// act on.
    #[test]
    fn failures_say_which_of_them_it_was() {
        let error = |status, body: &str| describe_failure(&images::classify_error(status, body));
        assert!(error(401, r#"{"error":{"message":"nope"}}"#).contains("rejected the key"));
        let denied = error(
            403,
            r#"{"error":{"type":"permission_error","message":"Image generation is not enabled for this group"}}"#,
        );
        assert!(denied.contains("not allowed"), "{denied}");
        let missing = error(
            404,
            r#"{"error":{"message":"Images API is not supported for this platform"}}"#,
        );
        assert!(missing.contains("OpenAI or Grok group"), "{missing}");
        // The gateway's own words survive.
        assert!(missing.contains("not supported for this platform"));
        // An unexpected status still says what happened.
        assert!(error(500, "boom").contains("500"));
    }
}
