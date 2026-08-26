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
    pub per_request_usd: Option<f64>,
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
}

/// One model in the catalog. Trimmed to what the desktop renders: the full
/// payload also carries tiered pricing intervals and per-group companions that
/// only the web console displays.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ModelCatalogItem {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub best_group: GroupRef,
    #[serde(default)]
    pub effective_pricing_usd: Price,
    #[serde(default)]
    pub comparison: Comparison,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ModelCatalog {
    #[serde(default, deserialize_with = "null_to_default")]
    pub items: Vec<ModelCatalogItem>,
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
