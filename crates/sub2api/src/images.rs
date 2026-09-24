//! Image generation through the managed gateway's Images API.
//!
//! Fork addition, shared by the image studio page and the agents'
//! `generate_image` tool so both reach the gateway the same way.
//!
//! What the gateway does that this has to follow (sub2api's
//! `handler/openai_images.go`, `handler/image_task_handler.go`):
//!
//! * `POST /v1/images/generations` draws from a prompt, `/v1/images/edits`
//!   from a prompt plus pictures (multipart, `image[]` parts). A part past
//!   20 MB is cut short without a word, so references are checked here first.
//! * The same two paths with `/async` answer `202` with a task id at once;
//!   `GET /v1/images/tasks/:id` is then polled with the key that submitted it.
//!   The feature needs object storage behind it and answers `404 async image
//!   tasks are not enabled` without it; a completed task hands back storage
//!   URLs rather than bytes.
//! * Synchronously, a picture can take minutes, past what a proxy in front of
//!   the gateway waits for a first byte. Streamed (`stream: true`), the
//!   gateway sends a keep-alive every ten seconds, so that is how the fallback
//!   asks, one picture per call.
//! * A `200` can still be a failure: a late error arrives as a JSON body with
//!   `error`, and a task can complete without any `data`.
//! * Image generation is granted per group, and the model catalog lists image
//!   models under groups that were not granted it — so which group to ask is
//!   found by asking ([`image_route_candidates`]) and remembered in
//!   `Credentials::image_groups`.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::auth::Credentials;
use crate::client::{ModelCatalogItem, PricingDetails};
use crate::{brand, gateway, http};

/// The gateway's own default when a request names no model.
pub const DEFAULT_MODEL: &str = "gpt-image-2";
/// The gateway cuts every uploaded part at this size without saying so.
pub const MAX_REFERENCE_BYTES: u64 = 20 * 1024 * 1024;
/// What the gateway's `Retry-After` asks for while a task runs.
pub const POLL_INTERVAL_SECONDS: u64 = 3;
/// How long the gateway keeps a task's record.
pub const TASK_TTL_SECONDS: i64 = 24 * 60 * 60;
/// The gateway fails a task that has run this long.
pub const TASK_TIMEOUT_SECONDS: i64 = 30 * 60;
/// Pictures one request may ask for.
pub const MAX_COUNT: u8 = 4;

/// Submitting uploads the references, which is the slow part.
const SUBMIT_TIMEOUT_SECONDS: u32 = 180;
const POLL_TIMEOUT_SECONDS: u32 = 30;
/// A streamed picture can run for minutes; the gateway gives up on an idle
/// upstream at fifteen.
const SYNC_TIMEOUT_SECONDS: u32 = 16 * 60;
const DOWNLOAD_TIMEOUT_SECONDS: u32 = 300;

/// Where a request goes and what it is signed with.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageRoute {
    /// The gateway origin, without the `/v1` the OpenAI path adds.
    pub origin: String,
    pub api_key: String,
}

impl fmt::Debug for ImageRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageRoute")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl ImageRoute {
    fn url(&self, path: &str) -> String {
        format!("{}/{path}", gateway::openai_base_url(&self.origin))
    }

    fn request(&self) -> http::Request {
        http::Request::new().bearer(self.api_key.trim())
    }
}

/// One drawing request: a prompt, and pictures to start from when it is an
/// edit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSpec {
    pub model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    #[serde(default = "one")]
    pub count: u8,
    /// Pictures to edit. None makes it a generation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<PathBuf>,
}

fn one() -> u8 {
    1
}

impl ImageSpec {
    pub fn model(&self) -> &str {
        match self.model.trim() {
            "" => DEFAULT_MODEL,
            model => model,
        }
    }

    /// Whether this starts from pictures (`/edits`) rather than words alone.
    pub fn is_edit(&self) -> bool {
        !self.references.is_empty()
    }

    pub fn count(&self) -> u8 {
        self.count.clamp(1, MAX_COUNT)
    }

    fn path(&self) -> &'static str {
        if self.is_edit() {
            "images/edits"
        } else {
            "images/generations"
        }
    }
}

/// Whether the gateway's Images endpoints take `model`: `gpt-image-*` and
/// Grok's image models (not its video one).
pub fn is_image_model(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    id.starts_with("gpt-image-")
        || id == "grok-imagine"
        || id == "grok-imagine-edit"
        || id.starts_with("grok-imagine-image")
}

pub fn is_grok(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("grok-")
}

/// Grok takes no `quality`; its size is turned into an aspect ratio.
pub fn supports_quality(model: &str) -> bool {
    !is_grok(model)
}

/// How many pictures an edit may start from.
pub fn max_references(model: &str) -> usize {
    if is_grok(model) { 3 } else { 10 }
}

/// The sizes offered for `model`, the first being the default. The gateway
/// checks none of them; these are the ones its billing knows by name.
pub fn size_options(model: &str) -> &'static [&'static str] {
    let id = model.trim().to_ascii_lowercase();
    if is_grok(&id) {
        &[
            "1024x1024",
            "1536x1024",
            "1024x1536",
            "2048x2048",
            "2048x1152",
        ]
    } else if id.starts_with("gpt-image-2") {
        &[
            "1024x1024",
            "1536x1024",
            "1024x1536",
            "2048x2048",
            "2048x1152",
            "3840x2160",
            "2160x3840",
            "auto",
        ]
    } else {
        &["1024x1024", "1536x1024", "1024x1536", "auto"]
    }
}

pub const QUALITY_OPTIONS: &[&str] = &["auto", "low", "medium", "high"];

/// The price tier a picture is billed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    OneK,
    TwoK,
    FourK,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Self::OneK => "1K",
            Self::TwoK => "2K",
            Self::FourK => "4K",
        }
    }
}

/// The tier the gateway bills `size` at (`image_billing_size.go`): by the
/// longest edge, with `auto` or no size at the default 2K. The output size
/// decides in the end, so for `auto` this is the likely price, not the sure
/// one.
pub fn tier_for_size(size: Option<&str>) -> Tier {
    let size = size.map(str::trim).unwrap_or("").to_ascii_lowercase();
    match size.as_str() {
        "1k" => return Tier::OneK,
        "2k" | "2048x2048" | "2048x1152" => return Tier::TwoK,
        "4k" | "3840x2160" | "2160x3840" => return Tier::FourK,
        _ => {}
    }
    let edge = size
        .split_once('x')
        .and_then(|(width, height)| {
            Some(
                width
                    .trim()
                    .parse::<u32>()
                    .ok()?
                    .max(height.trim().parse::<u32>().ok()?),
            )
        })
        .unwrap_or(0);
    match edge {
        0 => Tier::TwoK,
        1..=1024 => Tier::OneK,
        1025..=2048 => Tier::TwoK,
        _ => Tier::FourK,
    }
}

/// One picture's price at `tier`, as the catalog prices it for the model's
/// cheapest group.
pub fn tier_price(details: &PricingDetails, tier: Tier) -> Option<f64> {
    details
        .media_tiers
        .iter()
        .find(|entry| entry.tier.eq_ignore_ascii_case(tier.label()))
        .and_then(|entry| entry.effective_usd)
}

/// What `count` pictures at `size` should cost. Priced for the catalog's
/// cheapest group, so another group can differ.
pub fn estimate_usd(item: &ModelCatalogItem, size: Option<&str>, count: u8) -> Option<f64> {
    let tier = tier_for_size(size);
    let unit = tier_price(&item.pricing_details, tier).or_else(|| {
        // Without tiers the flat price is the default (2K) tier's.
        item.pricing_details
            .media_tiers
            .is_empty()
            .then_some(item.effective_pricing_usd.per_image_usd)
            .flatten()
    })?;
    Some(unit * f64::from(count.max(1)))
}

/// A group that may draw a model.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageGroup {
    pub group_id: i64,
    /// Empty when the catalog does not list the group for this model.
    pub name: String,
    pub platform: String,
    pub rate_multiplier: f64,
}

/// The groups to ask for `model`, in order: the one it last drew through,
/// the one model routing picked, every other group the catalog lists it
/// under (own platform before composite, then cheaper), and last the slot a
/// key already exists for. Only platforms with an Images endpoint are kept.
pub fn image_route_candidates(
    credentials: &Credentials,
    catalog: &[ModelCatalogItem],
    model: &str,
) -> Vec<ImageGroup> {
    let wanted = model.trim().to_ascii_lowercase();
    let own_platform = if is_grok(&wanted) { "grok" } else { "openai" };
    let mut listed: Vec<ImageGroup> = catalog
        .iter()
        .filter(|item| item.model.trim().eq_ignore_ascii_case(&wanted) && item.best_group.id > 0)
        .map(|item| ImageGroup {
            group_id: item.best_group.id,
            name: item.best_group.name.clone(),
            platform: item.platform.trim().to_ascii_lowercase(),
            rate_multiplier: item.best_group.rate_multiplier,
        })
        .filter(|group| matches!(group.platform.as_str(), "openai" | "grok" | "composite"))
        .collect();
    let rank = |group: &ImageGroup| u8::from(group.platform != own_platform);
    listed.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(a.rate_multiplier.total_cmp(&b.rate_multiplier))
            .then(a.group_id.cmp(&b.group_id))
    });

    let known = |id: i64| {
        listed
            .iter()
            .find(|group| group.group_id == id)
            .cloned()
            .unwrap_or(ImageGroup {
                group_id: id,
                name: String::new(),
                platform: String::new(),
                rate_multiplier: 0.0,
            })
    };
    let lookup = |table: &std::collections::BTreeMap<String, i64>| {
        table
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(&wanted))
            .map(|(_, group)| *group)
    };
    let slot = if is_grok(&wanted) {
        None
    } else {
        credentials.codex_group_id
    };

    let mut order: Vec<ImageGroup> = Vec::new();
    let preferred = [
        lookup(&credentials.image_groups),
        lookup(&credentials.model_routes),
    ];
    let fallback = [slot, credentials.group_id];
    let ids = preferred
        .into_iter()
        .flatten()
        .chain(listed.iter().map(|group| group.group_id))
        .chain(fallback.into_iter().flatten());
    for id in ids {
        if id > 0 && !order.iter().any(|group| group.group_id == id) {
            order.push(known(id));
        }
    }
    order
}

/// Why a request drew nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageError {
    pub kind: ImageErrorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The gateway's own words, when it said any.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageErrorKind {
    /// The group was not granted image generation: try another group.
    NoImagePermission,
    /// The key's group has no Images endpoint (not OpenAI or Grok).
    WrongPlatform,
    /// The group does not serve this model.
    ModelUnavailable,
    /// Moderation turned the prompt or a picture down.
    ContentPolicy,
    /// The account's own balance or quota ran out.
    OwnBalance,
    /// The gateway's upstream image account ran out.
    UpstreamBalance,
    /// Concurrency or queue limits: try again shortly.
    Busy,
    Timeout,
    /// The gateway has no asynchronous tasks: ask synchronously.
    AsyncUnavailable,
    /// The task is gone: expired, or never this key's.
    TaskLost,
    /// The request was cut off before it answered — the app closed while it
    /// was drawing without a task to come back to.
    Interrupted,
    BadRequest,
    Unauthorized,
    Network,
    Other,
}

impl ImageError {
    pub fn new(kind: ImageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            status: None,
            message: message.into(),
        }
    }

    pub fn network(error: impl fmt::Display) -> Self {
        Self::new(ImageErrorKind::Network, format!("{error:#}"))
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "HTTP {status}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ImageError {}

/// Read a failed response, or a `200` whose body is an error.
pub fn classify_error(status: u16, body: &str) -> ImageError {
    let value: Option<Value> = serde_json::from_str(body).ok();
    match value.as_ref().and_then(|value| value.get("error")) {
        Some(error @ Value::Object(_)) => classify_object(Some(status), error),
        Some(Value::String(message)) => classify(Some(status), "", "", message),
        _ => {
            let message = value
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    let trimmed: String = body.trim().chars().take(300).collect();
                    if trimmed.is_empty() {
                        format!("HTTP {status}")
                    } else {
                        trimmed
                    }
                });
            classify(Some(status), "", "", &message)
        }
    }
}

/// Read an `{type, code, message}` error object; `status` is the HTTP
/// status it came with, when known.
pub fn classify_object(status: Option<u16>, error: &Value) -> ImageError {
    let text = |field: &str| match error.get(field) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    };
    let status = status.or_else(|| {
        error
            .get("http_status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
    });
    let message = match text("message") {
        message if message.is_empty() => "the gateway gave no reason".to_owned(),
        message => message,
    };
    classify(status, &text("type"), &text("code"), &message)
}

fn classify(status: Option<u16>, kind: &str, code: &str, message: &str) -> ImageError {
    use ImageErrorKind::*;
    let lower = message.to_ascii_lowercase();
    let is = |name: &str| kind.eq_ignore_ascii_case(name) || code.eq_ignore_ascii_case(name);
    let kind = if lower.contains("async image tasks are not enabled") {
        AsyncUnavailable
    } else if lower.contains("image generation is not enabled") {
        NoImagePermission
    } else if is("content_policy_violation")
        || is("moderation_blocked")
        || lower.contains("content policy")
        || lower.contains("safety system")
    {
        ContentPolicy
    } else if lower.contains("not supported for this platform") {
        WrongPlatform
    } else if status == Some(402) {
        UpstreamBalance
    } else if is("billing_error") || lower.contains("insufficient balance") {
        OwnBalance
    } else if is("image_task_not_found") {
        TaskLost
    } else if status == Some(404) {
        ModelUnavailable
    } else if status == Some(429)
        || is("rate_limit_error")
        || is("rate_limit_exceeded")
        || is("gateway_concurrency_limit")
        || is("gateway_queue_full")
    {
        Busy
    } else if matches!(status, Some(504 | 524)) || is("timeout_error") {
        Timeout
    } else if status == Some(401) || is("authentication_error") {
        Unauthorized
    } else if status == Some(400) || is("invalid_request_error") {
        BadRequest
    } else {
        Other
    };
    ImageError {
        kind,
        status,
        message: message.to_owned(),
    }
}

/// One picture as the gateway hands it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageOutput {
    pub source: ImageSource,
    pub revised_prompt: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ImageSource {
    Base64(String),
    /// A storage URL, fetched without the gateway key.
    Url(String),
}

impl fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Base64(data) => write!(f, "Base64({} chars)", data.len()),
            Self::Url(url) => write!(f, "Url({url})"),
        }
    }
}

/// The pictures in an Images response, or the error it carries instead.
pub fn outputs_from_body(body: &str) -> Result<Vec<ImageOutput>, ImageError> {
    let value: Value = serde_json::from_str(body).map_err(|_| {
        ImageError::new(
            ImageErrorKind::Other,
            format!(
                "the gateway's reply was not JSON: {}",
                body.trim().chars().take(200).collect::<String>()
            ),
        )
    })?;
    outputs_from_value(&value)
}

fn outputs_from_value(value: &Value) -> Result<Vec<ImageOutput>, ImageError> {
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(match error {
            Value::String(message) => classify(None, "", "", message),
            error => classify_object(None, error),
        });
    }
    let outputs: Vec<ImageOutput> = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(output_from_item)
        .collect();
    if outputs.is_empty() {
        return Err(ImageError::new(
            ImageErrorKind::Other,
            "the gateway returned no images",
        ));
    }
    Ok(outputs)
}

fn output_from_item(item: &Value) -> Option<ImageOutput> {
    let text = |field: &str| {
        item.get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    let source = match (text("b64_json"), text("url")) {
        (Some(data), _) => ImageSource::Base64(data.to_owned()),
        (None, Some(url)) => match url.strip_prefix("data:") {
            Some(data_url) => ImageSource::Base64(data_url.split_once(',')?.1.to_owned()),
            None => ImageSource::Url(url.to_owned()),
        },
        (None, None) => return None,
    };
    Some(ImageOutput {
        source,
        revised_prompt: text("revised_prompt").map(str::to_owned),
    })
}

/// The pictures in a streamed reply: the `*.completed` events, or the
/// `error` one. A gateway that answered with plain JSON instead is read as
/// such.
pub fn outputs_from_stream(body: &str) -> Result<Vec<ImageOutput>, ImageError> {
    if body.trim_start().starts_with('{') {
        return outputs_from_body(body);
    }
    let mut outputs = Vec::new();
    let mut event = String::new();
    let mut data = String::new();
    let mut finish = |event: &str, data: &str| -> Result<(), ImageError> {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Ok(());
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if event == "error" || kind == "error" || value.get("error").is_some() {
            let error = value.get("error").unwrap_or(&value);
            return Err(match error {
                Value::String(message) => classify(None, "", "", message),
                error => classify_object(None, error),
            });
        }
        if (event.ends_with(".completed") || kind.ends_with(".completed"))
            && let Some(output) = output_from_item(&value)
        {
            outputs.push(output);
        }
        Ok(())
    };
    for line in body.lines().map(|line| line.trim_end_matches('\r')) {
        if line.is_empty() {
            if !data.is_empty() {
                finish(&event, &data)?;
            }
            event.clear();
            data.clear();
        } else if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_owned();
        } else if let Some(chunk) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(chunk.trim_start());
        }
    }
    if !data.is_empty() {
        finish(&event, &data)?;
    }
    if outputs.is_empty() {
        return Err(ImageError::new(
            ImageErrorKind::Other,
            "the stream ended without an image",
        ));
    }
    Ok(outputs)
}

/// The request for `spec`, asking `count` pictures: JSON for a generation,
/// multipart with the pictures for an edit.
fn build_request(route: &ImageRoute, spec: &ImageSpec, count: u8, stream: bool) -> http::Request {
    let model = spec.model();
    let grok = is_grok(model);
    let mut fields: Vec<(&str, String)> = vec![
        ("model", model.to_owned()),
        ("prompt", spec.prompt.clone()),
        ("n", count.to_string()),
    ];
    for (name, value) in [
        ("size", &spec.size),
        ("quality", &spec.quality),
        ("background", &spec.background),
        ("output_format", &spec.output_format),
    ] {
        if name == "quality" && grok {
            continue;
        }
        if let Some(value) = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            fields.push((name, value.to_owned()));
        }
    }
    if stream {
        fields.push(("stream", "true".to_owned()));
    } else if grok {
        // OpenAI's image models always answer in base64 and reject the
        // field; Grok answers with a URL unless asked.
        fields.push(("response_format", "b64_json".to_owned()));
    }

    let request = route.request();
    if spec.is_edit() {
        let mut request = fields.iter().fold(request, |request, (name, value)| {
            request.form_text(name, value)
        });
        for path in &spec.references {
            request =
                request.form_file("image[]", path, reference_mime(path).unwrap_or("image/png"));
        }
        request
    } else {
        let mut body = Map::new();
        for (name, value) in fields {
            let value = match name {
                "n" => json!(count),
                "stream" => json!(true),
                _ => json!(value),
            };
            body.insert(name.to_owned(), value);
        }
        request.json_body(Value::Object(body).to_string())
    }
}

/// Hand `spec` to the gateway as a task. The answer is the task id to poll
/// with the same route.
pub fn submit_async(route: &ImageRoute, spec: &ImageSpec) -> Result<String, ImageError> {
    let url = route.url(&format!("{}/async", spec.path()));
    let response = build_request(route, spec, spec.count(), false)
        .timeout_seconds(SUBMIT_TIMEOUT_SECONDS)
        .send(&url)
        .map_err(ImageError::network)?;
    if !response.is_success() {
        return Err(classify_error(response.status, &response.body));
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|_| classify_error(response.status, &response.body))?;
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(classify_error(response.status, &response.body));
    }
    ["id", "task_id"]
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_str))
        .map(str::to_owned)
        .ok_or_else(|| ImageError::new(ImageErrorKind::Other, "the gateway returned no task id"))
}

/// Where a submitted task stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskState {
    Processing,
    Completed(Vec<ImageOutput>),
    Failed(ImageError),
}

pub fn poll_task(route: &ImageRoute, task_id: &str) -> Result<TaskState, ImageError> {
    let url = route.url(&format!("images/tasks/{}", task_id.trim()));
    let response = route
        .request()
        .timeout_seconds(POLL_TIMEOUT_SECONDS)
        .send(&url)
        .map_err(ImageError::network)?;
    if response.status == 404 {
        let mut error = classify_error(404, &response.body);
        error.kind = ImageErrorKind::TaskLost;
        return Err(error);
    }
    if !response.is_success() {
        return Err(classify_error(response.status, &response.body));
    }
    parse_task(&response.body)
}

/// Read a task record (`GET /v1/images/tasks/:id`).
pub fn parse_task(body: &str) -> Result<TaskState, ImageError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ImageError::new(ImageErrorKind::Other, "the task record was not JSON"))?;
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    let http_status = value
        .get("http_status")
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok());
    match status {
        "processing" | "queued" | "running" => Ok(TaskState::Processing),
        "failed" => {
            let error = value.get("error").cloned().unwrap_or(Value::Null);
            Ok(TaskState::Failed(match error {
                Value::Object(_) => classify_object(http_status, &error),
                Value::String(message) => classify(http_status, "", "", &message),
                _ => classify(http_status, "", "", "the task failed without a reason"),
            }))
        }
        "completed" => {
            // A late error lands as a completed task whose result is one.
            let from_result = value.get("result").map(outputs_from_value);
            match from_result {
                Some(Ok(outputs)) => Ok(TaskState::Completed(outputs)),
                Some(Err(error)) if error.kind != ImageErrorKind::Other => {
                    Ok(TaskState::Failed(error))
                }
                result => match value.get("image_url").and_then(Value::as_str) {
                    Some(url) if !url.trim().is_empty() => {
                        Ok(TaskState::Completed(vec![ImageOutput {
                            source: ImageSource::Url(url.trim().to_owned()),
                            revised_prompt: None,
                        }]))
                    }
                    _ => Ok(TaskState::Failed(match result {
                        Some(Err(error)) => error,
                        _ => ImageError::new(
                            ImageErrorKind::Other,
                            "the task completed without an image",
                        ),
                    })),
                },
            }
        }
        other => Err(ImageError::new(
            ImageErrorKind::Other,
            format!("the task is in an unknown state `{other}`"),
        )),
    }
}

/// Draw `spec` in the request itself. OpenAI's models are streamed, one
/// picture per call so every call ends in its own `completed` event; Grok's
/// answer quickly and are asked once. A failure after the first picture
/// keeps the pictures already drawn.
pub fn generate_sync(route: &ImageRoute, spec: &ImageSpec) -> Result<Vec<ImageOutput>, ImageError> {
    let url = route.url(spec.path());
    let send = |count: u8, stream: bool| {
        build_request(route, spec, count, stream)
            .timeout_seconds(SYNC_TIMEOUT_SECONDS)
            .send(&url)
            .map_err(ImageError::network)
    };
    if is_grok(spec.model()) {
        let response = send(spec.count(), false)?;
        if !response.is_success() {
            return Err(classify_error(response.status, &response.body));
        }
        return outputs_from_body(&response.body);
    }
    let mut outputs = Vec::new();
    for _ in 0..spec.count() {
        let drawn = send(1, true).and_then(|response| {
            if response.is_success() {
                outputs_from_stream(&response.body)
            } else {
                Err(classify_error(response.status, &response.body))
            }
        });
        match drawn {
            Ok(drawn) => outputs.extend(drawn),
            Err(error) if outputs.is_empty() => return Err(error),
            Err(_) => break,
        }
    }
    Ok(outputs)
}

/// Draw `spec` start to finish, blocking: as a task where the gateway has
/// them, polled until it ends, otherwise synchronously. For callers without
/// an event loop of their own — the agents' tool.
pub fn generate_blocking(
    route: &ImageRoute,
    spec: &ImageSpec,
) -> Result<Vec<ImageOutput>, ImageError> {
    let task = match submit_async(route, spec) {
        Ok(task) => task,
        Err(error)
            if matches!(
                error.kind,
                ImageErrorKind::AsyncUnavailable | ImageErrorKind::WrongPlatform
            ) =>
        {
            return generate_sync(route, spec);
        }
        Err(error) => return Err(error),
    };
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(TASK_TIMEOUT_SECONDS as u64 + 60);
    let mut network_failures = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECONDS));
        match poll_task(route, &task) {
            Ok(TaskState::Processing) => network_failures = 0,
            Ok(TaskState::Completed(outputs)) => return Ok(outputs),
            Ok(TaskState::Failed(error)) => return Err(error),
            Err(error) if error.kind == ImageErrorKind::Network && network_failures < 5 => {
                network_failures += 1;
            }
            Err(error) => return Err(error),
        }
        if std::time::Instant::now() > deadline {
            return Err(ImageError::new(
                ImageErrorKind::Timeout,
                "the image task did not finish in time",
            ));
        }
    }
}

/// Write one picture next to `stem` (a path without extension), named by
/// what its bytes are. A URL is downloaded without the gateway key: task
/// results live in object storage, public or presigned.
pub fn save_output(output: &ImageOutput, stem: &Path) -> anyhow::Result<PathBuf> {
    if let Some(parent) = stem.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let with_extension = |extension: &str| {
        let mut name = stem.file_name().unwrap_or_default().to_os_string();
        name.push(".");
        name.push(extension);
        stem.with_file_name(name)
    };
    match &output.source {
        ImageSource::Base64(data) => {
            let bytes = decode_base64(data)?;
            let path = with_extension(sniff_extension(&bytes).unwrap_or("png"));
            std::fs::write(&path, &bytes)
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(path)
        }
        ImageSource::Url(url) => {
            let partial = with_extension("part");
            let response = http::Request::new()
                .timeout_seconds(DOWNLOAD_TIMEOUT_SECONDS)
                .download_to(&partial)
                .send(url)
                .context("could not download the image")?;
            if !response.is_success() {
                let _ = std::fs::remove_file(&partial);
                return Err(anyhow!(
                    "downloading the image failed with HTTP {}",
                    response.status
                ));
            }
            let mut head = [0_u8; 16];
            let read = {
                use std::io::Read;
                std::fs::File::open(&partial)
                    .and_then(|mut file| file.read(&mut head))
                    .with_context(|| format!("could not read {}", partial.display()))?
            };
            let path = with_extension(sniff_extension(&head[..read]).unwrap_or("png"));
            std::fs::rename(&partial, &path)
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(path)
        }
    }
}

pub fn decode_base64(data: &str) -> anyhow::Result<Vec<u8>> {
    use base64::Engine as _;
    let data: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .context("the image was not valid base64")
}

/// The file extension `bytes` begin like, for the formats the Images API
/// returns.
pub fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

/// The type an edit uploads a reference as, or `None` for a file the Images
/// API does not take.
pub fn reference_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Where the image studio keeps what it draws: the Pictures folder, under
/// the product's name, or the data directory where there is none.
pub fn pictures_dir() -> Option<PathBuf> {
    dirs::picture_dir()
        .map(|pictures| pictures.join(brand::DISPLAY_NAME))
        .or_else(|| brand::data_dir().map(|data| data.join("images")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::client::{GroupRef, MediaTier, Price};

    fn route() -> ImageRoute {
        ImageRoute {
            origin: "https://gateway.example".into(),
            api_key: "secret".into(),
        }
    }

    fn catalog_item(model: &str, group: i64, platform: &str, rate: f64) -> ModelCatalogItem {
        ModelCatalogItem {
            model: model.into(),
            platform: platform.into(),
            billing_mode: "image".into(),
            best_group: GroupRef {
                id: group,
                name: format!("group {group}"),
                rate_multiplier: rate,
                ..GroupRef::default()
            },
            ..ModelCatalogItem::default()
        }
    }

    #[test]
    fn image_models_are_the_ones_the_endpoint_takes() {
        assert!(is_image_model("gpt-image-2"));
        assert!(is_image_model("GPT-Image-1.5"));
        assert!(is_image_model("grok-imagine-image-quality"));
        assert!(is_image_model("grok-imagine"));
        assert!(!is_image_model("grok-imagine-video"));
        assert!(!is_image_model("gpt-5.2"));
        assert!(!is_image_model("dall-e-3"));
        assert_eq!(max_references("grok-imagine-image"), 3);
        assert!(!supports_quality("grok-imagine-image"));
    }

    #[test]
    fn sizes_are_tiered_as_the_gateway_bills_them() {
        assert_eq!(tier_for_size(Some("1024x1024")), Tier::OneK);
        assert_eq!(tier_for_size(Some("1536x1024")), Tier::TwoK);
        assert_eq!(tier_for_size(Some("2048x1152")), Tier::TwoK);
        assert_eq!(tier_for_size(Some("3840x2160")), Tier::FourK);
        assert_eq!(tier_for_size(Some("4096x4096")), Tier::FourK);
        assert_eq!(tier_for_size(Some("4K")), Tier::FourK);
        assert_eq!(tier_for_size(Some("auto")), Tier::TwoK);
        assert_eq!(tier_for_size(None), Tier::TwoK);
        for model in ["gpt-image-1", "gpt-image-2", "grok-imagine-image"] {
            assert_eq!(size_options(model)[0], "1024x1024");
        }
    }

    #[test]
    fn the_estimate_uses_the_tier_price() {
        let mut item = catalog_item("gpt-image-2", 1, "openai", 1.0);
        item.pricing_details.media_tiers = vec![
            MediaTier {
                tier: "1K".into(),
                effective_usd: Some(0.02),
                ..MediaTier::default()
            },
            MediaTier {
                tier: "2K".into(),
                effective_usd: Some(0.05),
                is_default_tier: true,
                ..MediaTier::default()
            },
        ];
        let estimate = estimate_usd(&item, Some("1024x1024"), 3).unwrap();
        assert!((estimate - 0.06).abs() < 1e-9);
        assert!((estimate_usd(&item, None, 1).unwrap() - 0.05).abs() < 1e-9);
        // A tier the catalog has no price for is not guessed.
        assert_eq!(estimate_usd(&item, Some("3840x2160"), 1), None);

        let mut flat = catalog_item("gpt-image-1", 1, "openai", 1.0);
        flat.effective_pricing_usd = Price {
            per_image_usd: Some(0.04),
            ..Price::default()
        };
        assert!((estimate_usd(&flat, Some("1024x1024"), 2).unwrap() - 0.08).abs() < 1e-9);
    }

    #[test]
    fn a_generation_is_json_and_openai_models_get_no_response_format() {
        let spec = ImageSpec {
            model: "gpt-image-2".into(),
            prompt: "a \"red\" fox".into(),
            size: Some("1536x1024".into()),
            quality: Some("high".into()),
            count: 2,
            ..ImageSpec::default()
        };
        let config = format!("{:?}", build_request(&route(), &spec, 2, false));
        assert!(config.contains(r#"\"n\":2"#), "{config}");
        assert!(config.contains(r#"\"quality\":\"high\""#));
        assert!(!config.contains("response_format"));
        assert!(!config.contains("stream"));

        let streamed = format!("{:?}", build_request(&route(), &spec, 1, true));
        assert!(streamed.contains(r#"\"stream\":true"#), "{streamed}");
    }

    #[test]
    fn grok_asks_for_base64_and_sends_no_quality() {
        let spec = ImageSpec {
            model: "grok-imagine-image".into(),
            prompt: "p".into(),
            quality: Some("high".into()),
            count: 1,
            ..ImageSpec::default()
        };
        let config = format!("{:?}", build_request(&route(), &spec, 1, false));
        assert!(
            config.contains(r#"\"response_format\":\"b64_json\""#),
            "{config}"
        );
        assert!(!config.contains("quality"));
    }

    #[test]
    fn an_edit_is_multipart_with_every_picture() {
        let spec = ImageSpec {
            model: String::new(),
            prompt: "make it blue".into(),
            count: 1,
            references: vec![PathBuf::from("a.png"), PathBuf::from("b.jpg")],
            ..ImageSpec::default()
        };
        assert!(spec.is_edit());
        assert_eq!(spec.path(), "images/edits");
        let debug = format!("{:?}", build_request(&route(), &spec, 1, false));
        assert!(
            debug.contains("name: \"image[]\", path: \"a.png\", mime: \"image/png\""),
            "{debug}"
        );
        assert!(debug.contains("mime: \"image/jpeg\""));
        assert!(
            debug.contains("value: \"gpt-image-2\""),
            "the default model is named"
        );
    }

    #[test]
    fn outputs_are_read_from_base64_data_urls_and_links() {
        let body = r#" {"data":[
            {"b64_json":"aGk=","revised_prompt":"hi"},
            {"url":"data:image/png;base64,aGk="},
            {"url":"https://bucket.example/x.png"},
            {}
        ]}"#;
        let outputs = outputs_from_body(body).unwrap();
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].source, ImageSource::Base64("aGk=".into()));
        assert_eq!(outputs[0].revised_prompt.as_deref(), Some("hi"));
        assert_eq!(outputs[1].source, ImageSource::Base64("aGk=".into()));
        assert_eq!(
            outputs[2].source,
            ImageSource::Url("https://bucket.example/x.png".into())
        );
        assert_eq!(
            outputs_from_body(r#"{"data":[]}"#).unwrap_err().kind,
            ImageErrorKind::Other
        );
    }

    #[test]
    fn a_200_carrying_an_error_is_a_failure() {
        let body = "  \n{\"error\":{\"type\":\"permission_error\",\"code\":\"content_policy_violation\",\"message\":\"Request blocked by content policy\"}}";
        let error = outputs_from_body(body).unwrap_err();
        assert_eq!(error.kind, ImageErrorKind::ContentPolicy);
    }

    #[test]
    fn a_stream_yields_its_completed_pictures() {
        let body = ": keep-alive\n\n\
            event: image_generation.partial_image\ndata: {\"type\":\"image_generation.partial_image\",\"b64_json\":\"cGFydA==\"}\n\n\
            event: image_generation.completed\ndata: {\"type\":\"image_generation.completed\",\"b64_json\":\"ZG9uZQ==\"}\n\n";
        let outputs = outputs_from_stream(body).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].source, ImageSource::Base64("ZG9uZQ==".into()));

        let failed = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"upstream_error\",\"message\":\"upstream image stream idle\"}}\n\n";
        assert_eq!(
            outputs_from_stream(failed).unwrap_err().message,
            "upstream image stream idle"
        );
        assert!(outputs_from_stream(": keep-alive\n\n").is_err());
        // An unstreamed answer still reads.
        assert_eq!(
            outputs_from_stream(r#"{"data":[{"b64_json":"aGk="}]}"#)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn task_records_are_read_in_every_state() {
        assert_eq!(
            parse_task(r#"{"id":"imgtask_1","status":"processing"}"#).unwrap(),
            TaskState::Processing
        );
        let done = parse_task(
            r#"{"status":"completed","image_url":"https://s/1.png","result":{"data":[{"url":"https://s/1.png"},{"url":"https://s/2.png"}]}}"#,
        )
        .unwrap();
        let TaskState::Completed(outputs) = done else {
            panic!("{done:?}")
        };
        assert_eq!(outputs.len(), 2);

        let only_link =
            parse_task(r#"{"status":"completed","image_url":"https://s/1.png"}"#).unwrap();
        assert!(matches!(only_link, TaskState::Completed(ref outputs) if outputs.len() == 1));

        let failed = parse_task(
            r#"{"status":"failed","http_status":403,"error":{"type":"permission_error","message":"Image generation is not enabled for this group"}}"#,
        )
        .unwrap();
        let TaskState::Failed(error) = failed else {
            panic!()
        };
        assert_eq!(error.kind, ImageErrorKind::NoImagePermission);
        assert_eq!(error.status, Some(403));

        // A late error recorded as a completed task.
        let late = parse_task(
            r#"{"status":"completed","result":{"error":{"type":"rate_limit_error","message":"slow down"}}}"#,
        )
        .unwrap();
        assert!(matches!(late, TaskState::Failed(ref error) if error.kind == ImageErrorKind::Busy));

        let empty = parse_task(r#"{"status":"completed","result":{"data":[]}}"#).unwrap();
        assert!(matches!(empty, TaskState::Failed(_)));
    }

    #[test]
    fn failures_are_told_apart() {
        let kind = |status, body: &str| classify_error(status, body).kind;
        use ImageErrorKind::*;
        assert_eq!(
            kind(
                404,
                r#"{"error":{"type":"not_found_error","message":"async image tasks are not enabled"}}"#
            ),
            AsyncUnavailable
        );
        assert_eq!(
            kind(
                404,
                r#"{"error":{"type":"not_found_error","message":"Images API is not supported for this platform"}}"#
            ),
            WrongPlatform
        );
        assert_eq!(
            kind(
                403,
                r#"{"error":{"type":"permission_error","message":"Image generation is not enabled for this group"}}"#
            ),
            NoImagePermission
        );
        assert_eq!(
            kind(
                402,
                r#"{"error":{"code":"insufficient_balance","message":"Upstream image account has insufficient balance"}}"#
            ),
            UpstreamBalance
        );
        assert_eq!(
            kind(
                403,
                r#"{"error":{"type":"billing_error","message":"no money"}}"#
            ),
            OwnBalance
        );
        assert_eq!(
            kind(
                400,
                r#"{"error":{"code":"moderation_blocked","message":"x"}}"#
            ),
            ContentPolicy
        );
        assert_eq!(
            kind(
                429,
                r#"{"error":{"type":"rate_limit_error","message":"x"}}"#
            ),
            Busy
        );
        assert_eq!(
            kind(
                404,
                r#"{"error":{"type":"invalid_request_error","message":"Model \"x\" is not available for this group"}}"#
            ),
            ModelUnavailable
        );
        assert_eq!(kind(524, "<html>timeout</html>"), Timeout);
        assert_eq!(
            kind(401, r#"{"error":{"message":"bad key"}}"#),
            Unauthorized
        );
        assert_eq!(kind(500, ""), Other);
        assert_eq!(classify_error(500, "").message, "HTTP 500");
    }

    #[test]
    fn candidates_put_the_known_group_first_and_skip_platforms_without_images() {
        let catalog = vec![
            catalog_item("gpt-image-2", 5, "openai", 1.5),
            catalog_item("gpt-image-2", 6, "composite", 0.5),
            catalog_item("gpt-image-2", 7, "openai", 1.0),
            catalog_item("gpt-image-2", 8, "anthropic", 0.1),
            catalog_item("gpt-image-1", 9, "openai", 0.1),
        ];
        let mut credentials = Credentials {
            codex_group_id: Some(3),
            ..Credentials::default()
        };
        let ids = |credentials: &Credentials| {
            image_route_candidates(credentials, &catalog, "gpt-image-2")
                .iter()
                .map(|group| group.group_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&credentials), vec![7, 5, 6, 3]);

        credentials.model_routes = BTreeMap::from([("gpt-image-2".into(), 5)]);
        credentials.image_groups = BTreeMap::from([("gpt-image-2".into(), 6)]);
        assert_eq!(ids(&credentials), vec![6, 5, 7, 3]);
        let first = &image_route_candidates(&credentials, &catalog, "gpt-image-2")[0];
        assert_eq!(first.name, "group 6");
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

    /// A one-request-per-connection server answering `replies` in order and
    /// keeping each request's head and body.
    fn serve(replies: Vec<(u16, Vec<u8>)>) -> (String, Seen) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Default::default();
        let record = seen.clone();
        std::thread::spawn(move || {
            for (status, reply) in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut raw = Vec::new();
                let mut buffer = [0_u8; 8192];
                let head_end = loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break raw.len();
                    }
                    raw.extend_from_slice(&buffer[..read]);
                    if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                while raw.len() < head_end + length {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..read]);
                }
                record
                    .lock()
                    .unwrap()
                    .push((head, raw[head_end..].to_vec()));
                let mut response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.len()
                )
                .into_bytes();
                response.extend_from_slice(&reply);
                let _ = stream.write_all(&response);
            }
        });
        (origin, seen)
    }

    #[test]
    fn an_edit_uploads_its_pictures_and_a_result_downloads() {
        let dir = std::env::temp_dir().join(format!("sub2api-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A `;` in the name would end the form value if it were not quoted.
        let picture = dir.join("in;put.png");
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3, 4];
        std::fs::write(&picture, png).unwrap();

        let (origin, seen) = serve(vec![
            (
                202,
                br#"{"id":"imgtask_7","task_id":"t","status":"processing"}"#.to_vec(),
            ),
            (200, png.to_vec()),
        ]);
        let route = ImageRoute {
            origin: origin.clone(),
            api_key: "secret".into(),
        };
        let spec = ImageSpec {
            model: "gpt-image-2".into(),
            prompt: "make it \"blue\"; now".into(),
            count: 1,
            references: vec![picture],
            ..ImageSpec::default()
        };
        assert_eq!(submit_async(&route, &spec).unwrap(), "imgtask_7");

        let output = ImageOutput {
            source: ImageSource::Url(format!("{origin}/files/x")),
            revised_prompt: None,
        };
        let saved = save_output(&output, &dir.join("out")).unwrap();
        assert_eq!(saved, dir.join("out.png"));
        assert_eq!(std::fs::read(&saved).unwrap(), png);

        let seen = seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(head.starts_with("POST /v1/images/edits/async "), "{head}");
        assert!(head.contains("Authorization: Bearer secret"));
        assert!(head.contains("multipart/form-data"));
        assert!(!head.to_ascii_lowercase().contains("expect:"));
        let body_text = String::from_utf8_lossy(body);
        assert!(body_text.contains("make it \"blue\"; now"), "{body_text}");
        assert!(body_text.contains("filename=\"in;put.png\""), "{body_text}");
        assert!(body.windows(png.len()).any(|window| window == png));
        // The download carries no gateway key.
        assert!(!seen[1].0.contains("Authorization"));
        drop(seen);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_pictures_are_named_by_their_bytes() {
        let dir = std::env::temp_dir().join(format!("sub2api-images-{}", std::process::id()));
        let png = [0x89, b'P', b'N', b'G', 0, 0];
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        let output = ImageOutput {
            source: ImageSource::Base64(encoded),
            revised_prompt: None,
        };
        let path = save_output(&output, &dir.join("one-1")).unwrap();
        assert_eq!(path, dir.join("one-1.png"));
        assert_eq!(std::fs::read(&path).unwrap(), png);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(sniff_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_extension(b"nope"), None);
        assert_eq!(reference_mime(Path::new("a.JPEG")), Some("image/jpeg"));
        assert_eq!(reference_mime(Path::new("a.gif")), None);
    }
}
