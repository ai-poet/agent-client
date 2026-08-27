//! Managed-service API client.
//!
//! Mirrors the endpoints the Electron client already depends on, so the two
//! stay interchangeable against one backend. Every response is wrapped in an
//! envelope whose `code` must be `0`; a non-zero code is an application-level
//! error even when the HTTP status is 200.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::http::{Request, Response};

/// Envelope every endpoint returns.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Envelope<T> {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub data: Option<T>,
}

/// A page of results.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Paginated<T> {
    #[serde(
        default,
        deserialize_with = "null_to_default",
        bound(deserialize = "T: serde::Deserialize<'de>")
    )]
    pub items: Vec<T>,
    #[serde(default)]
    pub total: i64,
}

/// `#[serde(default)]` only covers an *absent* field; the service reports
/// empty collections as explicit `null` (`"allowed_groups":null` on a live
/// `/auth/me`), which fails a plain `Vec` field. Every container therefore
/// deserializes through this.
fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// The signed-in user. Fields default so that a backend that adds or drops
/// optional properties cannot break an older desktop build.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct User {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub balance: f64,
    #[serde(default)]
    pub status: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub allowed_groups: Vec<i64>,
}

/// A model group the account may use.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Group {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub rate_multiplier: f64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub subscription_type: String,
}

/// A gateway API key. `key` is the secret the agent CLIs authenticate with.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ApiKey {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub status: String,
}

/// Per-million-token prices. Every field is optional because a model may bill
/// per request or per image instead, and a missing figure must render as "—"
/// rather than as zero.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Price {
    #[serde(default)]
    pub input_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub output_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub cache_write_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub cache_read_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub per_request_usd: Option<f64>,
    #[serde(default)]
    pub per_image_usd: Option<f64>,
    /// Where the official figure comes from (e.g. `litellm`), when known.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub has_reference: bool,
}

/// How the gateway's price compares with the vendor's list price.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Comparison {
    #[serde(default)]
    pub savings_percent: Option<f64>,
    #[serde(default)]
    pub is_cheaper_than_official: bool,
}

/// The group a catalog entry routes through.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GroupRef {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub rate_multiplier: f64,
    /// `group_default` or `user_override`.
    #[serde(default)]
    pub rate_source: String,
}

/// One tier of a tiered price schedule.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct PriceInterval {
    #[serde(default)]
    pub min_tokens: i64,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub tier_label: String,
    #[serde(default)]
    pub input_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub output_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub cache_write_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub cache_read_per_mtok_usd: Option<f64>,
    #[serde(default)]
    pub per_request_usd: Option<f64>,
    #[serde(default)]
    pub per_image_usd: Option<f64>,
}

/// Detail flags and schedules the catalog card surfaces.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct PricingDetails {
    #[serde(default)]
    pub supports_prompt_caching: bool,
    #[serde(default)]
    pub has_long_context_multiplier: bool,
    #[serde(default)]
    pub long_context_input_threshold: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub intervals: Vec<PriceInterval>,
}

/// Another group a catalog entry is available through, with its own pricing.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GroupCompanion {
    #[serde(default)]
    pub group: GroupRef,
    #[serde(default)]
    pub effective_pricing_usd: Price,
    #[serde(default)]
    pub comparison: Comparison,
}

/// One model in the catalog, in the shape the model-plaza page renders: both
/// price columns (official struck through, effective beside it), the billing
/// mode, and the other groups the model is reachable through.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ModelCatalogItem {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub platform: String,
    /// `token`, `per_request`, or `image`; empty means `token`.
    #[serde(default)]
    pub billing_mode: String,
    #[serde(default)]
    pub best_group: GroupRef,
    #[serde(default)]
    pub available_group_count: i64,
    #[serde(default)]
    pub official_pricing: Price,
    #[serde(default)]
    pub effective_pricing_usd: Price,
    #[serde(default)]
    pub comparison: Comparison,
    #[serde(default, deserialize_with = "null_to_default")]
    pub pricing_details: PricingDetails,
    #[serde(default, deserialize_with = "null_to_default")]
    pub other_groups: Vec<GroupCompanion>,
}

/// Headline figures above the catalog.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct CatalogSummary {
    #[serde(default)]
    pub total_models: i64,
    #[serde(default)]
    pub token_models: i64,
    #[serde(default)]
    pub non_token_models: i64,
    #[serde(default)]
    pub max_savings_percent: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ModelCatalog {
    #[serde(default, deserialize_with = "null_to_default")]
    pub items: Vec<ModelCatalogItem>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub summary: Option<CatalogSummary>,
}

/// Health of one group's upstream, as `/group-status` reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupStatusItem {
    pub group_id: i64,
    pub group_name: String,
    pub latest_status: String,
    pub stable_status: String,
    pub latency_ms: Option<f64>,
    pub availability_24h: Option<f64>,
    pub availability_7d: Option<f64>,
}

impl GroupStatusItem {
    /// The status the UI should describe: the smoothed one, falling back to
    /// the latest sample.
    pub fn effective_status(&self) -> &str {
        if !self.stable_status.is_empty() {
            &self.stable_status
        } else {
            &self.latest_status
        }
    }
}

/// `/group-status` has shipped two shapes: a flat item, and a nested
/// `{summary: {...}, group: {...}}`. Normalize either — the Electron client
/// does the same.
fn normalize_group_status(value: &serde_json::Value) -> GroupStatusItem {
    let summary = value.get("summary");
    let group = value.get("group");
    let number = |keys: [Option<&serde_json::Value>; 2]| {
        keys.into_iter()
            .flatten()
            .find_map(serde_json::Value::as_f64)
    };
    let string = |keys: [Option<&serde_json::Value>; 2]| {
        keys.into_iter()
            .flatten()
            .find_map(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let nested = |key: &str| summary.and_then(|summary| summary.get(key));
    GroupStatusItem {
        group_id: number([value.get("group_id"), nested("group_id")])
            .or_else(|| group.and_then(|group| group.get("id")).and_then(serde_json::Value::as_f64))
            .unwrap_or_default() as i64,
        group_name: {
            let name = string([value.get("group_name"), group.and_then(|group| group.get("name"))]);
            if name.is_empty() {
                string([nested("group_name"), None])
            } else {
                name
            }
        },
        latest_status: string([value.get("latest_status"), nested("latest_status")]),
        stable_status: string([value.get("stable_status"), nested("stable_status")]),
        latency_ms: number([value.get("latency_ms"), nested("latency_ms")]),
        availability_24h: number([value.get("availability_24h"), value.get("availability24")]),
        availability_7d: number([value.get("availability_7d"), value.get("availability7d")]),
    }
}

/// Result of redeeming a code.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct RedeemResult {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub new_balance: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ReferralStats {
    #[serde(default)]
    pub total_referrals: i64,
}

/// Referral code and share link.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ReferralInfo {
    #[serde(default)]
    pub referral_code: String,
    #[serde(default)]
    pub referral_link: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub stats: ReferralStats,
}

/// One service announcement targeted at the signed-in account.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Announcement {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    /// Markdown body.
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub read_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl Announcement {
    pub fn is_unread(&self) -> bool {
        self.read_at.is_none()
    }

    /// The calendar date of publication, for compact list rows.
    pub fn created_date(&self) -> Option<&str> {
        self.created_at
            .as_deref()
            .map(|stamp| stamp.split('T').next().unwrap_or(stamp))
    }
}

/// Aggregate usage for a period, as `/usage/stats` reports it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct UsageStats {
    #[serde(default)]
    pub total_requests: i64,
    #[serde(default)]
    pub total_input_tokens: i64,
    #[serde(default)]
    pub total_output_tokens: i64,
    #[serde(default)]
    pub total_cache_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub total_cost: f64,
    #[serde(default)]
    pub total_actual_cost: f64,
    #[serde(default)]
    pub average_duration_ms: f64,
}

/// One request in the usage log. Trimmed to what the desktop renders.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct UsageLog {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_creation_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub total_cost: f64,
    #[serde(default)]
    pub actual_cost: f64,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub first_token_ms: Option<i64>,
    #[serde(default)]
    pub rate_multiplier: f64,
    #[serde(default)]
    pub long_context_billing_applied: bool,
    #[serde(default)]
    pub image_count: i64,
    #[serde(default)]
    pub request_type: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub group: Option<Group>,
}

/// Filters for the request log, matching the web console's `/usage` params.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageLogQuery {
    /// 1-based.
    pub page: u32,
    pub page_size: u32,
    pub model: Option<String>,
    pub group_id: Option<i64>,
}

/// Access/refresh pair returned by login and refresh.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct TokenPair {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_in: i64,
}

/// Blocking client. Callers run it off the UI thread.
#[derive(Clone, Debug)]
pub struct Client {
    endpoint: String,
}

impl Client {
    /// `endpoint` is the service origin, with or without a trailing slash.
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into().trim_end_matches('/').to_owned();
        Self { endpoint }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Absolute URL for an API path such as `/auth/me`.
    pub fn api_url(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("{}/api/v1/{path}", self.endpoint)
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str, access_token: &str) -> Result<T> {
        let response = Request::new().bearer(access_token).send(&self.api_url(path))?;
        unwrap_envelope(&response)
    }

    fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        access_token: Option<&str>,
        body: serde_json::Value,
    ) -> Result<T> {
        let mut request = Request::new().json_body(body.to_string());
        if let Some(token) = access_token {
            request = request.bearer(token);
        }
        let response = request.send(&self.api_url(path))?;
        unwrap_envelope(&response)
    }

    /// The signed-in user, including the balance shown in the status bar.
    pub fn me(&self, access_token: &str) -> Result<User> {
        self.get("/auth/me", access_token)
    }

    /// Groups this account may route to.
    pub fn available_groups(&self, access_token: &str) -> Result<Vec<Group>> {
        self.get("/groups/available", access_token)
    }

    /// Existing gateway keys, used to reuse a key rather than minting one per
    /// launch.
    pub fn list_keys(&self, access_token: &str) -> Result<Paginated<ApiKey>> {
        self.get("/keys?page=1&page_size=50", access_token)
    }

    /// Mint a gateway key. Only this key — never the OAuth tokens — is handed
    /// to the daemon.
    pub fn create_key(
        &self,
        access_token: &str,
        name: &str,
        group_id: Option<i64>,
    ) -> Result<ApiKey> {
        let mut body = serde_json::json!({ "name": name });
        if let Some(group_id) = group_id {
            body["group_id"] = serde_json::json!(group_id);
        }
        self.post("/keys", Some(access_token), body)
    }

    /// Exchange a refresh token for a fresh pair. Unauthenticated by design:
    /// it is called precisely when the access token has expired.
    pub fn refresh(&self, refresh_token: &str) -> Result<TokenPair> {
        self.post(
            "/auth/refresh",
            None,
            serde_json::json!({ "refresh_token": refresh_token }),
        )
    }

    /// Model catalog with gateway pricing.
    pub fn model_catalog(&self, access_token: &str) -> Result<ModelCatalog> {
        self.get("/models/catalog", access_token)
    }

    /// Health of every group's upstream: status, latency, availability.
    pub fn group_statuses(&self, access_token: &str) -> Result<Vec<GroupStatusItem>> {
        let raw: Vec<serde_json::Value> = self.get("/group-status", access_token)?;
        Ok(raw.iter().map(normalize_group_status).collect())
    }

    /// Aggregate usage for `period` (`today` / `week` / `month`).
    pub fn usage_stats(&self, access_token: &str, period: &str) -> Result<UsageStats> {
        self.get(&format!("/usage/stats?period={period}"), access_token)
    }

    /// A page of the request log, newest first, optionally filtered.
    pub fn usage_logs(
        &self,
        access_token: &str,
        query: &UsageLogQuery,
    ) -> Result<Paginated<UsageLog>> {
        let mut path = format!(
            "/usage?page={}&page_size={}",
            query.page.max(1),
            query.page_size.max(1)
        );
        if let Some(model) = query.model.as_deref().filter(|model| !model.is_empty()) {
            path.push_str(&format!("&model={}", percent_encode(model)));
        }
        if let Some(group_id) = query.group_id {
            path.push_str(&format!("&group_id={group_id}"));
        }
        self.get(&path, access_token)
    }

    /// Redeem a top-up or gift code.
    pub fn redeem_code(&self, access_token: &str, code: &str) -> Result<RedeemResult> {
        self.post(
            "/redeem",
            Some(access_token),
            serde_json::json!({ "code": code }),
        )
    }

    /// The user's referral code and share link.
    pub fn referral_info(&self, access_token: &str) -> Result<ReferralInfo> {
        self.get("/referral/info", access_token)
    }

    /// Active service announcements for this account, newest first.
    pub fn announcements(&self, access_token: &str) -> Result<Vec<Announcement>> {
        self.get("/announcements", access_token)
    }

    /// Mark one announcement read; the server keeps per-user read state.
    pub fn mark_announcement_read(&self, access_token: &str, id: i64) -> Result<()> {
        let _: serde_json::Value = self.post(
            &format!("/announcements/{id}/read"),
            Some(access_token),
            serde_json::json!({}),
        )?;
        Ok(())
    }

    /// URL of the hosted top-up page, to be opened in the user's browser.
    ///
    /// Top-up is a web flow on the service, not something the client
    /// implements: the Electron client embeds the same page in a webview
    /// (`ui_mode=embedded`). A native window has no webview on every platform,
    /// and payment is exactly the kind of flow that should run in a real
    /// browser the user can inspect — so this asks for the standalone layout
    /// and hands it to the system browser.
    ///
    /// The access token rides in the query string because the page
    /// authenticates with it; it is a short-lived token to the user's own
    /// account, on an origin they already trust.
    pub fn top_up_url(&self, access_token: &str, language: &str) -> String {
        format!(
            "{}/pay?token={}&theme=dark&ui_mode=standalone&lang={}",
            self.endpoint,
            percent_encode(access_token),
            percent_encode(language)
        )
    }
}

/// Percent-encode everything outside the unreserved set.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

/// Unwrap an envelope, turning both transport and application errors into one
/// error type.
fn unwrap_envelope<T: serde::de::DeserializeOwned>(response: &Response) -> Result<T> {
    let envelope: Envelope<T> = response.json()?;
    if envelope.code != 0 {
        let reason = envelope
            .reason
            .as_deref()
            .filter(|reason| !reason.is_empty())
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        let message = if envelope.message.is_empty() {
            "the service rejected the request".to_owned()
        } else {
            envelope.message
        };
        return Err(anyhow!("{message}{reason}"));
    }
    envelope
        .data
        .ok_or_else(|| anyhow!("the service returned an empty payload"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(body: &str) -> Response {
        Response {
            status: 200,
            body: body.to_owned(),
        }
    }

    #[test]
    fn api_url_joins_without_double_slashes() {
        let client = Client::new("https://example.org/");
        assert_eq!(client.api_url("/auth/me"), "https://example.org/api/v1/auth/me");
        assert_eq!(client.api_url("auth/me"), "https://example.org/api/v1/auth/me");
        assert_eq!(client.endpoint(), "https://example.org");
    }

    #[test]
    fn explicit_nulls_fall_back_like_absent_fields() {
        // Regression: the live /auth/me reports empty collections as null
        // ("allowed_groups":null), which broke sign-in with
        // "invalid type: null, expected a sequence".
        let user: User = unwrap_envelope(&ok(
            r#"{"code":0,"message":"success","data":{"id":1,"email":"a@b.c",
                "username":"a","role":"admin","balance":999980.6,
                "frozen_balance":0,"concurrency":5,"status":"active",
                "allowed_groups":null}}"#,
        ))
        .expect("unwrap");
        assert!(user.allowed_groups.is_empty());
        assert_eq!(user.balance, 999980.6);

        let page: Paginated<ApiKey> =
            unwrap_envelope(&ok(r#"{"code":0,"data":{"items":null,"total":0}}"#))
                .expect("unwrap");
        assert!(page.items.is_empty());

        let catalog: ModelCatalog =
            unwrap_envelope(&ok(r#"{"code":0,"data":{"items":null}}"#)).expect("unwrap");
        assert!(catalog.items.is_empty());

        let referral: ReferralInfo = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"referral_code":"X","referral_link":"","stats":null}}"#,
        ))
        .expect("unwrap");
        assert_eq!(referral.stats.total_referrals, 0);
    }

    #[test]
    fn unwraps_a_successful_envelope() {
        let user: User = unwrap_envelope(&ok(
            r#"{"code":0,"message":"ok","data":{"id":7,"email":"a@b.c","balance":12.5}}"#,
        ))
        .expect("unwrap");
        assert_eq!(user.id, 7);
        assert_eq!(user.email, "a@b.c");
        assert_eq!(user.balance, 12.5);
        // Absent fields fall back rather than failing the whole response.
        assert_eq!(user.username, "");
        assert!(user.allowed_groups.is_empty());
    }

    #[test]
    fn nonzero_code_is_an_error_even_on_http_200() {
        let error = unwrap_envelope::<User>(&ok(
            r#"{"code":40101,"message":"token expired","reason":"expired","data":null}"#,
        ))
        .expect_err("should reject");
        assert!(error.to_string().contains("token expired"));
        assert!(error.to_string().contains("expired"));
    }

    #[test]
    fn missing_data_on_success_is_an_error() {
        let error = unwrap_envelope::<User>(&ok(r#"{"code":0,"message":"ok"}"#))
            .expect_err("should reject");
        assert!(error.to_string().contains("empty payload"));
    }

    #[test]
    fn non_2xx_surfaces_the_http_status() {
        let error = unwrap_envelope::<User>(&Response {
            status: 502,
            body: "bad gateway".to_owned(),
        })
        .expect_err("should reject");
        assert!(error.to_string().contains("502"));
    }

    #[test]
    fn unknown_envelope_fields_are_ignored() {
        // The backend adding a field must not break an older desktop build.
        let key: ApiKey = unwrap_envelope(&ok(
            r#"{"code":0,"message":"ok","metadata":{"x":"y"},"data":{"id":1,"key":"sk-test","brand_new_field":123}}"#,
        ))
        .expect("unwrap");
        assert_eq!(key.key, "sk-test");
    }

    #[test]
    fn model_catalog_keeps_optional_prices_optional() {
        // A per-request model has no per-token price; rendering 0.00 there
        // would claim it is free.
        let catalog: ModelCatalog = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"items":[
                {"model":"gpt-x","display_name":"GPT X","platform":"openai",
                 "best_group":{"id":3,"name":"Std"},
                 "effective_pricing_usd":{"input_per_mtok_usd":1.5,"output_per_mtok_usd":null},
                 "comparison":{"savings_percent":42.0,"is_cheaper_than_official":true}}
            ]}}"#,
        ))
        .expect("unwrap");
        let item = &catalog.items[0];
        assert_eq!(item.display_name, "GPT X");
        assert_eq!(item.best_group.name, "Std");
        assert_eq!(item.effective_pricing_usd.input_per_mtok_usd, Some(1.5));
        assert_eq!(item.effective_pricing_usd.output_per_mtok_usd, None);
        assert_eq!(item.comparison.savings_percent, Some(42.0));
    }

    #[test]
    fn catalog_tolerates_an_entry_missing_everything_optional() {
        let catalog: ModelCatalog =
            unwrap_envelope(&ok(r#"{"code":0,"data":{"items":[{"model":"m"}]}}"#))
                .expect("unwrap");
        assert_eq!(catalog.items[0].model, "m");
        assert!(catalog.items[0].display_name.is_empty());
    }

    #[test]
    fn catalog_carries_both_price_columns_and_companions() {
        // The plaza renders official struck through beside effective; losing
        // either column silently would misstate the price.
        let catalog: ModelCatalog = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"items":[
                {"model":"claude-x","display_name":"Claude X","platform":"anthropic",
                 "billing_mode":"token",
                 "best_group":{"id":1,"name":"Fast","rate_multiplier":0.5},
                 "available_group_count":2,
                 "official_pricing":{"input_per_mtok_usd":3.0,"output_per_mtok_usd":15.0,
                    "cache_write_per_mtok_usd":3.75,"cache_read_per_mtok_usd":0.3},
                 "effective_pricing_usd":{"input_per_mtok_usd":1.5,"output_per_mtok_usd":7.5},
                 "comparison":{"savings_percent":50.0,"is_cheaper_than_official":true},
                 "pricing_details":{"supports_prompt_caching":true},
                 "other_groups":[{"group":{"id":2,"name":"Std","rate_multiplier":1.0},
                    "effective_pricing_usd":{"input_per_mtok_usd":3.0},
                    "comparison":{"savings_percent":0.0,"is_cheaper_than_official":false}}]}
            ],"summary":{"total_models":10,"token_models":8,"non_token_models":2,
                "max_savings_percent":72.5}}}"#,
        ))
        .expect("unwrap");
        let item = &catalog.items[0];
        assert_eq!(item.official_pricing.input_per_mtok_usd, Some(3.0));
        assert_eq!(item.official_pricing.cache_read_per_mtok_usd, Some(0.3));
        assert_eq!(item.best_group.rate_multiplier, 0.5);
        assert!(item.pricing_details.supports_prompt_caching);
        assert_eq!(item.other_groups[0].group.name, "Std");
        assert_eq!(catalog.summary.as_ref().map(|s| s.total_models), Some(10));

        // Explicit nulls degrade like absences, as everywhere else.
        let catalog: ModelCatalog = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"items":[{"model":"m","pricing_details":null,
                "other_groups":null}],"summary":null}}"#,
        ))
        .expect("unwrap");
        assert!(catalog.items[0].other_groups.is_empty());
        assert!(catalog.summary.is_none());
    }

    #[test]
    fn group_status_normalizes_flat_and_nested_payloads() {
        let flat: serde_json::Value = serde_json::from_str(
            r#"{"group_id":7,"group_name":"Fast","latest_status":"up",
                "stable_status":"up","latency_ms":312.5,
                "availability_24h":99.2,"availability_7d":98.7}"#,
        )
        .expect("json");
        let item = normalize_group_status(&flat);
        assert_eq!(item.group_id, 7);
        assert_eq!(item.group_name, "Fast");
        assert_eq!(item.effective_status(), "up");
        assert_eq!(item.availability_24h, Some(99.2));

        // The other shipped shape nests the figures under `summary`/`group`.
        let nested: serde_json::Value = serde_json::from_str(
            r#"{"group":{"id":9,"name":"Std"},
                "summary":{"latest_status":"degraded","latency_ms":900.0}}"#,
        )
        .expect("json");
        let item = normalize_group_status(&nested);
        assert_eq!(item.group_id, 9);
        assert_eq!(item.group_name, "Std");
        assert_eq!(item.effective_status(), "degraded");
        assert_eq!(item.latency_ms, Some(900.0));
        assert_eq!(item.availability_24h, None);
    }

    #[test]
    fn redeem_and_referral_parse() {
        let redeem: RedeemResult = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"message":"ok","value":10.0,"new_balance":25.5}}"#,
        ))
        .expect("unwrap");
        assert_eq!(redeem.new_balance, Some(25.5));

        let referral: ReferralInfo = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"referral_code":"ABC","referral_link":"https://x/r/ABC","stats":{"total_referrals":4}}}"#,
        ))
        .expect("unwrap");
        assert_eq!(referral.referral_code, "ABC");
        assert_eq!(referral.stats.total_referrals, 4);
    }

    #[test]
    fn top_up_url_carries_an_encoded_token() {
        let url = Client::new("https://cloud.example.org/").top_up_url("tok en/+1", "zh");
        assert!(url.starts_with("https://cloud.example.org/pay?"));
        // An unencoded token would break the query string at the first `/`
        // or `+` and land the user on a page that cannot authenticate them.
        assert!(url.contains("token=tok%20en%2F%2B1"));
        assert!(url.contains("ui_mode=standalone"));
        assert!(url.contains("lang=zh"));
    }

    #[test]
    fn token_pair_parses_refresh_response() {
        let pair: TokenPair = unwrap_envelope(&ok(
            r#"{"code":0,"data":{"access_token":"a","refresh_token":"r","expires_in":3600}}"#,
        ))
        .expect("unwrap");
        assert_eq!(pair.access_token, "a");
        assert_eq!(pair.expires_in, 3600);
    }
}
