//! Payment-center API: the native top-up flow.
//!
//! The pay center is a separate service mounted at `{endpoint}/pay`; unlike
//! the gateway API its responses are plain JSON, not enveloped, and errors
//! come back as `{"error": ...}` with a non-2xx status. The Electron client's
//! modal drives exactly these endpoints:
//!
//! * `GET  /pay/api/orders/my`      — who is paying, balance, pending count
//! * `GET  /pay/api/user`           — payment methods, limits, exchange rates
//! * `POST /pay/api/orders`         — create an order
//! * `GET  /pay/api/orders/{id}`    — poll its status
//! * `POST /pay/api/orders/{id}/cancel`
//!
//! Amounts are US dollars credited; when the config carries a CNY rate the
//! actual charge is CNY and the UI shows both.

use anyhow::{Result, anyhow};
use serde::Deserialize;

use crate::http::Request;

/// Per-method limits, as `/pay/api/user` reports them.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MethodLimit {
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default)]
    pub remaining: Option<f64>,
    #[serde(default)]
    pub single_min: Option<f64>,
    #[serde(default)]
    pub single_max: Option<f64>,
    /// Percent, e.g. `2.5` for a 2.5% surcharge.
    #[serde(default)]
    pub fee_rate: Option<f64>,
}

fn default_true() -> bool {
    true
}

impl Default for MethodLimit {
    fn default() -> Self {
        Self {
            available: true,
            remaining: None,
            single_min: None,
            single_max: None,
            fee_rate: None,
        }
    }
}

/// Everything the top-up form needs, assembled from the two config endpoints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PayConfig {
    pub user_display_name: String,
    pub user_balance: Option<f64>,
    pub enabled_payment_types: Vec<String>,
    pub method_limits: std::collections::BTreeMap<String, MethodLimit>,
    pub min_amount: f64,
    pub max_amount: f64,
    pub max_pending_orders: i64,
    pub pending_count: i64,
    /// CNY charged per USD credited; `None` means the charge is in USD.
    pub balance_credit_cny_per_usd: Option<f64>,
    pub stripe_enabled: bool,
}

impl PayConfig {
    /// The floor for `payment_type`, folding the method's own minimum into
    /// the global one.
    pub fn effective_min(&self, payment_type: &str) -> f64 {
        let method = self
            .method_limits
            .get(payment_type)
            .and_then(|limit| limit.single_min)
            .filter(|min| *min > 0.0);
        match method {
            Some(min) => self.min_amount.max(min),
            None => self.min_amount,
        }
    }

    /// The ceiling for `payment_type`.
    pub fn effective_max(&self, payment_type: &str) -> f64 {
        self.method_limits
            .get(payment_type)
            .and_then(|limit| limit.single_max)
            .filter(|max| *max > 0.0)
            .unwrap_or(self.max_amount)
    }

    /// Percent surcharge for `payment_type`, zero when none applies.
    pub fn fee_rate(&self, payment_type: &str) -> f64 {
        self.method_limits
            .get(payment_type)
            .and_then(|limit| limit.fee_rate)
            .filter(|rate| *rate > 0.0)
            .unwrap_or(0.0)
    }
}

/// A created order.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PayOrder {
    #[serde(default)]
    pub order_id: String,
    /// USD credited on completion.
    #[serde(default)]
    pub amount: f64,
    /// What the user actually pays, in the charge currency.
    #[serde(default)]
    pub pay_amount: Option<f64>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub payment_type: String,
    #[serde(default)]
    pub pay_url: Option<String>,
    #[serde(default)]
    pub qr_code: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub status_access_token: String,
}

/// A status poll's answer.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderStatus {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub payment_success: bool,
    #[serde(default)]
    pub recharge_success: bool,
    #[serde(default)]
    pub recharge_status: String,
    #[serde(default)]
    pub failed_reason: Option<String>,
}

impl OrderStatus {
    /// The order finished — paid, failed, cancelled, or expired — and polling
    /// should stop.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.to_uppercase().as_str(),
            "FAILED" | "CANCELLED" | "EXPIRED" | "COMPLETED"
        )
    }

    /// The balance was credited.
    pub fn is_settled(&self) -> bool {
        self.recharge_success || self.status.eq_ignore_ascii_case("completed")
    }

    /// Seed shown between order creation and the first poll answer.
    pub fn seed(order: &PayOrder) -> Self {
        Self {
            id: order.order_id.clone(),
            status: order.status.clone(),
            expires_at: order.expires_at.clone(),
            payment_success: false,
            recharge_success: false,
            recharge_status: "not_paid".to_owned(),
            failed_reason: None,
        }
    }
}

/// How an order gets paid, resolved exactly as the Electron client does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayFlow {
    /// Show a QR code the user scans with a payment app.
    Qr,
    /// Open `pay_url` in the browser.
    Redirect,
    /// Stripe checkout; the native app hands this to the hosted pay center.
    Stripe,
}

/// Payment types whose `pay_url` must open in a browser rather than render
/// as a QR code.
const REDIRECT_PAYMENT_PREFIXES: [&str; 2] = ["wxpay", "bank"];

pub fn resolve_flow(order: &PayOrder) -> PayFlow {
    if order.client_secret.is_some() {
        return PayFlow::Stripe;
    }
    let payment_type = order.payment_type.trim().to_lowercase();
    if REDIRECT_PAYMENT_PREFIXES
        .iter()
        .any(|prefix| payment_type.starts_with(prefix))
    {
        return PayFlow::Redirect;
    }
    if order.qr_code.is_some() {
        return PayFlow::Qr;
    }
    PayFlow::Redirect
}

/// The user-facing name of a payment type, in the Electron client's wording.
pub fn payment_label(payment_type: &str, chinese: bool) -> String {
    let normalized = payment_type.trim().to_lowercase();
    if normalized.starts_with("alipay") {
        return if chinese { "支付宝" } else { "Alipay" }.to_owned();
    }
    if normalized.starts_with("wxpay") {
        return if chinese { "微信支付" } else { "WeChat Pay" }.to_owned();
    }
    if normalized.starts_with("usdt") {
        return "USDT".to_owned();
    }
    if normalized.starts_with("usdc") {
        return "USDC".to_owned();
    }
    if normalized.starts_with("stripe") {
        return "Stripe".to_owned();
    }
    payment_type.to_owned()
}

pub fn is_stripe(payment_type: &str) -> bool {
    payment_type.trim().to_lowercase().starts_with("stripe")
}

/// Blocking pay-center client. Callers run it off the UI thread.
#[derive(Clone, Debug)]
pub struct PayClient {
    endpoint: String,
    /// Bare language tag (`zh`, `en`, `ja`) the service localizes its own
    /// error strings with.
    lang: String,
}

impl PayClient {
    pub fn new(endpoint: impl Into<String>, lang: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
            lang: lang.into(),
        }
    }

    fn url(&self, path_and_query: &str) -> String {
        let separator = if path_and_query.contains('?') { '&' } else { '?' };
        format!(
            "{}/pay{path_and_query}{separator}lang={}",
            self.endpoint,
            percent(&self.lang)
        )
    }

    /// Both config calls, folded into one form-ready value.
    pub fn load_config(&self, access_token: &str) -> Result<PayConfig> {
        let orders: serde_json::Value = Request::new()
            .header("Accept-Language", &self.lang)
            .send(&self.url(&format!(
                "/api/orders/my?token={}&page=1&page_size=20",
                percent(access_token)
            )))?
            .json()?;
        let user = orders.get("user");
        let field = |key: &str| user.and_then(|user| user.get(key));
        let user_id = field("id")
            .and_then(serde_json::Value::as_i64)
            .filter(|id| *id > 0)
            .ok_or_else(|| anyhow!("the payment service did not identify the account"))?;
        let user_display_name = ["displayName", "username", "email"]
            .into_iter()
            .find_map(|key| field(key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_default()
            .to_owned();
        let user_balance = field("balance").and_then(serde_json::Value::as_f64);
        let pending_count = orders
            .get("summary")
            .and_then(|summary| summary.get("pending"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let payload: serde_json::Value = Request::new()
            .send(&self.url(&format!(
                "/api/user?user_id={user_id}&token={}",
                percent(access_token)
            )))?
            .json()?;
        let config = payload.get("config").cloned().unwrap_or_default();
        let enabled_payment_types = config
            .get("enabledPaymentTypes")
            .and_then(serde_json::Value::as_array)
            .map(|types| {
                types
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let method_limits = config
            .get("methodLimits")
            .and_then(serde_json::Value::as_object)
            .map(|limits| {
                limits
                    .iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            serde_json::from_value(value.clone()).unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let number = |key: &str| config.get(key).and_then(serde_json::Value::as_f64);

        Ok(PayConfig {
            user_display_name,
            user_balance,
            enabled_payment_types,
            method_limits,
            min_amount: number("minAmount").unwrap_or(1.0),
            max_amount: number("maxAmount").unwrap_or(1000.0),
            max_pending_orders: config
                .get("maxPendingOrders")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(3),
            pending_count,
            balance_credit_cny_per_usd: number("balanceCreditCnyPerUsd"),
            stripe_enabled: config
                .get("stripePublishableKey")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|key| !key.trim().is_empty()),
        })
    }

    /// Create an order for `amount` USD credited via `payment_type`.
    pub fn create_order(
        &self,
        access_token: &str,
        amount: f64,
        payment_type: &str,
    ) -> Result<PayOrder> {
        let body = serde_json::json!({
            "token": access_token,
            "amount": amount,
            "payment_type": payment_type,
            "is_mobile": false,
        });
        let order: PayOrder = Request::new()
            .header("Accept-Language", &self.lang)
            .json_body(body.to_string())
            .send(&self.url("/api/orders"))?
            .json()?;
        if order.order_id.is_empty() {
            return Err(anyhow!("the payment service returned no order id"));
        }
        if order.status_access_token.is_empty() {
            return Err(anyhow!("the payment service returned no status token"));
        }
        Ok(order)
    }

    /// Poll an order. Authenticates with the order's own status token, not
    /// the account token.
    pub fn order_status(&self, order_id: &str, status_access_token: &str) -> Result<OrderStatus> {
        Request::new()
            .header("Accept-Language", &self.lang)
            .send(&self.url(&format!(
                "/api/orders/{}?access_token={}",
                percent(order_id),
                percent(status_access_token)
            )))?
            .json()
    }

    /// Cancel a pending order.
    pub fn cancel_order(&self, access_token: &str, order_id: &str) -> Result<()> {
        let body = serde_json::json!({ "token": access_token });
        let _: serde_json::Value = Request::new()
            .header("Accept-Language", &self.lang)
            .json_body(body.to_string())
            .send(&self.url(&format!("/api/orders/{}/cancel", percent(order_id))))?
            .json()?;
        Ok(())
    }

    /// The hosted pay center, for the flows the native modal cannot carry
    /// (Stripe) and as the fallback when config loading fails.
    pub fn pay_center_url(&self, access_token: &str) -> String {
        format!(
            "{}/pay?token={}&theme=dark&ui_mode=standalone&lang={}",
            self.endpoint,
            percent(access_token),
            percent(&self.lang)
        )
    }
}

/// Percent-encode everything outside the unreserved set.
fn percent(value: &str) -> String {
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

/// Seconds until an RFC 3339 instant, or `None` when unparseable. Zero once
/// past. Offset-less timestamps are read as UTC, which is what the service
/// sends.
pub fn seconds_until(expires_at: &str) -> Option<i64> {
    let epoch = rfc3339_to_epoch(expires_at)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((epoch - now).max(0))
}

/// Parse `2026-08-27T12:34:56(.789)?(Z|±hh:mm)?` into Unix seconds.
fn rfc3339_to_epoch(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (date, rest) = raw.split_once(['T', ' '])?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Split the time from a trailing offset; fractional seconds are ignored.
    let (time, offset_seconds) = if let Some(time) = rest.strip_suffix(['Z', 'z']) {
        (time, 0)
    } else if let Some(plus) = rest.rfind(['+']) {
        (&rest[..plus], -parse_offset(&rest[plus + 1..])?)
    } else if let Some(minus) = rest.rfind('-').filter(|index| *index >= 8) {
        (&rest[..minus], parse_offset(&rest[minus + 1..])?)
    } else {
        (rest, 0)
    };
    let time = time.split('.').next()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;

    // Howard Hinnant's days-from-civil algorithm.
    let years = if month <= 2 { year - 1 } else { year };
    let era = if years >= 0 { years } else { years - 399 } / 400;
    let year_of_era = years - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;

    Some(days * 86_400 + hour * 3_600 + minute * 60 + second + offset_seconds)
}

fn parse_offset(raw: &str) -> Option<i64> {
    let (hours, minutes) = raw.split_once(':').unwrap_or((raw, "0"));
    Some(hours.parse::<i64>().ok()? * 3_600 + minutes.parse::<i64>().ok()? * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(payment_type: &str, qr: Option<&str>, secret: Option<&str>) -> PayOrder {
        PayOrder {
            order_id: "o1".into(),
            payment_type: payment_type.into(),
            qr_code: qr.map(str::to_owned),
            client_secret: secret.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn flow_resolution_matches_the_electron_client() {
        // clientSecret always wins; wxpay/bank redirect even with a QR
        // payload; anything else with a QR renders it; the rest redirect.
        assert_eq!(resolve_flow(&order("alipay", None, Some("cs"))), PayFlow::Stripe);
        assert_eq!(resolve_flow(&order("wxpay_native", Some("qr"), None)), PayFlow::Redirect);
        assert_eq!(resolve_flow(&order("bank_transfer", None, None)), PayFlow::Redirect);
        assert_eq!(resolve_flow(&order("alipay", Some("qr-data"), None)), PayFlow::Qr);
        assert_eq!(resolve_flow(&order("alipay", None, None)), PayFlow::Redirect);
    }

    #[test]
    fn payment_labels_localize() {
        assert_eq!(payment_label("alipay_f2f", true), "支付宝");
        assert_eq!(payment_label("alipay_f2f", false), "Alipay");
        assert_eq!(payment_label("wxpay_native", true), "微信支付");
        assert_eq!(payment_label("usdt_trc20", true), "USDT");
        assert_eq!(payment_label("stripe", true), "Stripe");
        assert_eq!(payment_label("mystery", true), "mystery");
        assert!(is_stripe("STRIPE_card"));
        assert!(!is_stripe("alipay"));
    }

    #[test]
    fn terminal_and_settled_states() {
        let mut status = OrderStatus {
            status: "PENDING".into(),
            ..Default::default()
        };
        assert!(!status.is_terminal());
        assert!(!status.is_settled());
        status.status = "cancelled".into();
        assert!(status.is_terminal());
        status.status = "PENDING".into();
        status.recharge_success = true;
        assert!(status.is_settled());
    }

    #[test]
    fn method_limits_fold_into_effective_bounds() {
        let mut config = PayConfig {
            min_amount: 1.0,
            max_amount: 1000.0,
            ..Default::default()
        };
        config.method_limits.insert(
            "alipay".into(),
            MethodLimit {
                single_min: Some(5.0),
                single_max: Some(200.0),
                fee_rate: Some(2.5),
                ..Default::default()
            },
        );
        assert_eq!(config.effective_min("alipay"), 5.0);
        assert_eq!(config.effective_max("alipay"), 200.0);
        assert_eq!(config.fee_rate("alipay"), 2.5);
        // Unknown method: global bounds, no fee.
        assert_eq!(config.effective_min("wxpay"), 1.0);
        assert_eq!(config.effective_max("wxpay"), 1000.0);
        assert_eq!(config.fee_rate("wxpay"), 0.0);
    }

    #[test]
    fn method_limit_parses_camel_case_payload() {
        let limit: MethodLimit = serde_json::from_str(
            r#"{"available":false,"remaining":42.5,"singleMin":5,"singleMax":200,"feeRate":3}"#,
        )
        .expect("parse");
        assert!(!limit.available);
        assert_eq!(limit.remaining, Some(42.5));
        assert_eq!(limit.single_min, Some(5.0));
        assert_eq!(limit.fee_rate, Some(3.0));
        // Absent `available` means available: the service omits it for
        // unrestricted methods.
        let limit: MethodLimit = serde_json::from_str("{}").expect("parse");
        assert!(limit.available);
    }

    #[test]
    fn order_parses_camel_case_payload() {
        let order: PayOrder = serde_json::from_str(
            r#"{"orderId":"ord_1","amount":10,"payAmount":73.5,"status":"PENDING",
                "paymentType":"alipay","payUrl":null,"qrCode":"weixin://pay",
                "clientSecret":null,"expiresAt":"2026-08-27T12:00:00Z",
                "statusAccessToken":"sat_1"}"#,
        )
        .expect("parse");
        assert_eq!(order.order_id, "ord_1");
        assert_eq!(order.pay_amount, Some(73.5));
        assert_eq!(order.qr_code.as_deref(), Some("weixin://pay"));
        assert_eq!(resolve_flow(&order), PayFlow::Qr);
    }

    #[test]
    fn pay_urls_carry_lang_and_encode_tokens() {
        let client = PayClient::new("https://cloud.example.org/", "zh");
        assert_eq!(
            client.url("/api/orders"),
            "https://cloud.example.org/pay/api/orders?lang=zh"
        );
        assert_eq!(
            client.url("/api/orders/my?token=a+b"),
            "https://cloud.example.org/pay/api/orders/my?token=a+b&lang=zh"
        );
        assert!(client.pay_center_url("tok/1").contains("token=tok%2F1"));
    }

    #[test]
    fn rfc3339_parsing_handles_the_service_formats() {
        // 2026-08-27T00:00:00Z == 1787788800 (verified against `date -d`).
        assert_eq!(rfc3339_to_epoch("2026-08-27T00:00:00Z"), Some(1_787_788_800));
        assert_eq!(
            rfc3339_to_epoch("2026-08-27T00:00:00.123Z"),
            Some(1_787_788_800)
        );
        // No offset reads as UTC.
        assert_eq!(rfc3339_to_epoch("2026-08-27T00:00:00"), Some(1_787_788_800));
        // +08:00 is eight hours before the same wall clock in UTC.
        assert_eq!(
            rfc3339_to_epoch("2026-08-27T08:00:00+08:00"),
            Some(1_787_788_800)
        );
        assert_eq!(rfc3339_to_epoch("not a date"), None);
        // The Unix epoch itself.
        assert_eq!(rfc3339_to_epoch("1970-01-01T00:00:00Z"), Some(0));
    }
}
