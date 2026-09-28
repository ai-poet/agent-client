//! Managed cloud sign-in over a loopback redirect or a pasted one-time code,
//! and credential storage.
//!
//! # Why loopback
//!
//! The desktop opens the system browser at the service's login bridge and
//! listens on `127.0.0.1:<ephemeral>` for the redirect. Compared with
//! registering a custom URL scheme this needs no per-platform installer work
//! and no OS registration, and it behaves identically on macOS, Windows, and
//! Linux.
//!
//! # Why a one-time code
//!
//! Some machines cannot open `http://127.0.0.1` from the browser at all — a
//! proxy that captures loopback, security software, a browser on another
//! device. So the flow carries a PKCE challenge ([`Pkce`]): the bridge trades
//! it for a short one-time code, shows that code on the page, and redirects
//! with only the code in the fragment. The desktop redeems the code together
//! with the verifier only it holds ([`exchange_login_code`]), whether the code
//! arrived over loopback or was pasted into the sign-in window
//! ([`parse_pasted_login`]). A code seen by anyone else is useless without the
//! verifier, and no token ever appears in a URL.
//!
//! A bridge that predates the code flow ignores the challenge and still sends
//! the session itself in the fragment; that is accepted too.
//!
//! # Why a relay page
//!
//! Browsers never send a URL *fragment* to the server, so the loopback
//! listener cannot read it directly. The callback therefore serves a small
//! page that copies `location.hash` back to the same local server with a
//! `POST`.
//!
//! # What is stored where
//!
//! OAuth access and refresh tokens stay in this process' own credential file.
//! The CLIs only ever see the derived gateway keys, written into their own
//! configuration by `global_config`.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::brand;
use crate::client::{Client, DesktopSession};

/// Path on the login bridge that starts the browser flow.
const LOGIN_BRIDGE_PATH: &str = "/auth/paseo";

/// Refresh this long before expiry so a request never races the deadline.
const REFRESH_SKEW_SECONDS: i64 = 120;

/// The service's reason for a code it will not redeem: unknown, expired,
/// already used, or not issued for this verifier. Deliberately one reason.
const CODE_INVALID_REASON: &str = "DESKTOP_LOGIN_CODE_INVALID";

/// Serializes every read-compare-write of the credential file in this
/// process, so a background save can never land between a sign-out's delete
/// and the next sign-in's write.
static CREDENTIALS_LOCK: Mutex<()> = Mutex::new(());

fn credentials_lock() -> MutexGuard<'static, ()> {
    CREDENTIALS_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The session a background task was working for is no longer the stored
/// one: the user signed out, or signed in again, while it ran. Whatever it
/// produced belongs to nobody and must be dropped, not written back.
#[derive(Clone, Copy, Debug)]
pub struct SignedOut;

impl std::fmt::Display for SignedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the cloud account was signed out")
    }
}

impl std::error::Error for SignedOut {}

/// The browser sign-in was cancelled from the app.
#[derive(Clone, Copy, Debug)]
pub struct SignInCancelled;

impl std::fmt::Display for SignInCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the sign-in was cancelled")
    }
}

impl std::error::Error for SignInCancelled {}

/// Nobody delivered a session before the deadline.
#[derive(Clone, Copy, Debug)]
pub struct SignInTimedOut;

impl std::fmt::Display for SignInTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("timed out waiting for the browser sign-in")
    }
}

impl std::error::Error for SignInTimedOut {}

/// Pasted text that holds neither a sign-in code nor a callback link.
#[derive(Clone, Copy, Debug)]
pub struct NotALoginCode;

impl std::fmt::Display for NotALoginCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("that is not a sign-in code or sign-in link")
    }
}

impl std::error::Error for NotALoginCode {}

/// The service would not redeem the code: it is unknown, expired, already
/// used, or was issued to another sign-in attempt.
#[derive(Clone, Copy, Debug)]
pub struct LoginCodeRejected;

impl std::fmt::Display for LoginCodeRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the sign-in code is invalid or has expired")
    }
}

impl std::error::Error for LoginCodeRejected {}

/// Whether `error` carries the typed cause `T` anywhere in its chain.
pub fn error_is<T: std::error::Error + Send + Sync + 'static>(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.downcast_ref::<T>().is_some())
}

/// Stored session. Serialized to `~/.cheaprouter/cloud-account.json`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds at which `access_token` stops being valid.
    pub expires_at: i64,
    /// Service origin this session belongs to.
    pub endpoint: String,
    /// Gateway keys minted by the bridge at sign-in.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub claude_api_key: Option<String>,
    #[serde(default)]
    pub codex_api_key: Option<String>,
    /// Model group `api_key` is bound to, if the user picked one.
    ///
    /// Persisted alongside the key because the key alone does not say which
    /// group it belongs to: without this the settings page would show "account
    /// default" as selected after a restart while requests kept routing
    /// through the group.
    #[serde(default)]
    pub group_id: Option<i64>,
    /// Group `claude_api_key` is bound to, when the user picked one per CLI.
    #[serde(default)]
    pub claude_group_id: Option<i64>,
    /// Group `codex_api_key` is bound to, when the user picked one per CLI.
    #[serde(default)]
    pub codex_group_id: Option<i64>,
    /// The Chinese models' group (DeepSeek, GLM, Kimi, …): a subscription or
    /// a pay-as-you-go group, picked on Settings → Cloud Account. No CLI
    /// holds it — its key lives in `group_keys` — and only the built-in
    /// agent's routing reads it ([`crate::model_routing::Bindings::domestic`]).
    #[serde(default)]
    pub domestic_group_id: Option<i64>,
    /// Keys for groups no CLI slot is bound to — the domestic group,
    /// subscriptions and the pay-as-you-go groups beside them — by group id.
    /// The built-in agent sends each model through the group that serves it,
    /// which is often not one of the three slots.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub group_keys: BTreeMap<i64, String>,
    /// The group each of the built-in agent's models goes through, as
    /// [`crate::model_routing::resolve`] last worked it out from the catalog
    /// and the user's subscriptions. A model missing here keeps the key of
    /// its platform's slot.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_routes: BTreeMap<String, i64>,
    /// The group each image model was last seen to draw through, learned by
    /// the image studio. A group has to be granted image generation on its
    /// own, and the catalog lists image models under groups that were not,
    /// so this is found by asking and kept: it outranks `model_routes` for
    /// its models, which sends the agents' image calls there too.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub image_groups: BTreeMap<String, i64>,
    /// Each catalog model's context window in tokens, as the catalog last
    /// reported it. Written into the built-in agent's settings, where it
    /// sizes the usage meter and the auto-compact threshold: the engine's own
    /// model table predates most of the models the gateway serves.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_windows: BTreeMap<String, u64>,
    /// The user turned gateway routing off without signing out. Stored
    /// inverted so the serde default (false) means the common case: signing
    /// in routes.
    #[serde(default)]
    pub routing_disabled: bool,
    /// Identity of this sign-in, minted when it was established and kept
    /// through every token refresh. A background task's copy is written
    /// back — into memory or into the file — only while it still names the
    /// stored session, so a request that was in flight when the user signed
    /// out (or signed in as someone else) cannot bring the old session back.
    /// Empty in files written before it existed; the app assigns one at
    /// startup.
    #[serde(default)]
    pub session_id: String,
}

impl Credentials {
    pub fn path() -> Option<PathBuf> {
        brand::data_dir().map(|dir| dir.join("cloud-account.json"))
    }

    /// Load the stored session, or `None` when signed out.
    pub fn load() -> Option<Self> {
        let _guard = credentials_lock();
        Self::read()
    }

    fn read() -> Option<Self> {
        let path = Self::path()?;
        let raw = std::fs::read_to_string(path).ok()?;
        let parsed: Self = serde_json::from_str(&raw).ok()?;
        if parsed.access_token.is_empty() || parsed.endpoint.is_empty() {
            return None;
        }
        Some(parsed)
    }

    /// Update the stored session.
    ///
    /// Refused with [`SignedOut`] unless the file still holds this same
    /// session: a copy a background task took before the user signed out
    /// must not write the account back to disk, and one taken before a
    /// sign-in as someone else must not overwrite the new account.
    pub fn save(&self) -> Result<()> {
        let _guard = credentials_lock();
        match Self::read() {
            Some(stored) if stored.session_id == self.session_id => self.write(),
            _ => Err(SignedOut.into()),
        }
    }

    /// Store this session as the signed-in one, replacing whatever was
    /// there. Only a completed sign-in (and the startup migration that gives
    /// an old file its session id) may do this.
    pub fn establish(&self) -> Result<()> {
        let _guard = credentials_lock();
        self.write()
    }

    /// Persist with owner-only permissions. Callers hold the lock.
    fn write(&self) -> Result<()> {
        let path = Self::path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, body).with_context(|| format!("could not write {}", path.display()))?;
        restrict_to_owner(&path);
        Ok(())
    }

    /// Remove the stored session.
    ///
    /// A file that cannot be deleted (held open by a scanner on Windows, say)
    /// is emptied instead: [`Self::load`] reads an empty file as signed out,
    /// where leaving it would sign the user back in on the next launch.
    pub fn clear() -> Result<()> {
        let _guard = credentials_lock();
        let Some(path) = Self::path() else {
            return Ok(());
        };
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => match std::fs::write(&path, "") {
                Ok(()) => Ok(()),
                Err(_) => Err(anyhow::Error::from(error)
                    .context(format!("could not remove {}", path.display()))),
            },
        }
    }

    /// A signed-in session from what the login bridge handed over.
    ///
    /// Rejects an incomplete one instead of storing it: empty tokens would
    /// sign the user out on the next launch with nothing to explain why.
    pub fn from_desktop_session(session: DesktopSession, endpoint: &str) -> Result<Self> {
        let access_token = session.access_token.trim();
        let refresh_token = session.refresh_token.trim();
        if access_token.is_empty() || refresh_token.is_empty() || session.expires_in <= 0 {
            return Err(anyhow!("the sign-in callback did not include a session"));
        }
        let key = |value: Option<String>| {
            value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        Ok(Credentials {
            access_token: access_token.to_owned(),
            refresh_token: refresh_token.to_owned(),
            expires_at: now_unix() + session.expires_in,
            endpoint: normalize_endpoint(endpoint)?,
            api_key: key(session.api_key),
            claude_api_key: key(session.claude_api_key),
            codex_api_key: key(session.codex_api_key),
            // The bridge hands out account-level keys; group bindings are a
            // later, explicit choice in the app.
            group_id: None,
            claude_group_id: None,
            codex_group_id: None,
            domestic_group_id: None,
            group_keys: BTreeMap::new(),
            model_routes: BTreeMap::new(),
            image_groups: BTreeMap::new(),
            model_windows: BTreeMap::new(),
            // Signing in is an explicit request to route through the service.
            routing_disabled: false,
            session_id: new_session_id()?,
        })
    }

    /// True when the access token is expired or close enough that it should be
    /// refreshed before the next request.
    pub fn needs_refresh(&self, now_unix: i64) -> bool {
        self.expires_at - REFRESH_SKEW_SECONDS <= now_unix
    }

    /// Apply a refreshed token pair, keeping the old refresh token when the
    /// service does not rotate it.
    pub fn apply_refresh(&mut self, pair: &crate::client::TokenPair, now_unix: i64) {
        self.access_token = pair.access_token.clone();
        if !pair.refresh_token.is_empty() {
            self.refresh_token = pair.refresh_token.clone();
        }
        self.expires_at = now_unix + pair.expires_in;
    }
}

/// Current unix time in seconds.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(unix)]
fn restrict_to_owner(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &std::path::Path) {
    // Windows inherits the user profile ACL, which is already owner-only.
}

/// Random bytes from the operating system's CSPRNG.
fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow!("could not read random bytes from the system: {error}"))?;
    Ok(bytes)
}

/// A fresh [`Credentials::session_id`].
pub fn new_session_id() -> Result<String> {
    Ok(random_bytes::<16>()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// A PKCE pair (RFC 7636, `S256`) for one sign-in attempt.
///
/// The challenge travels through the browser; the verifier never leaves this
/// process, and only it can redeem the one-time code the bridge issues.
/// Deliberately not `Debug`: the verifier must not end up in a log.
#[derive(Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Result<Self> {
        let verifier = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
        let challenge = pkce_challenge(&verifier);
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

/// `BASE64URL(SHA256(verifier))`, unpadded.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// What reached the app: a one-time code to redeem, or — from a bridge that
/// predates the code flow — the session itself.
#[derive(Debug)]
pub enum Delivery {
    Code(String),
    Session(Credentials),
}

/// Text on the page the loopback callback serves, in the app's language.
#[derive(Clone, Debug, Serialize)]
pub struct RelayText {
    /// `lang` attribute of the page, e.g. `zh-CN`.
    pub lang: String,
    pub finishing: String,
    pub finishing_detail: String,
    pub done: String,
    pub done_detail: String,
    pub failed: String,
    /// Shown under a failure: the code on the sign-in page still works when
    /// pasted into the app.
    pub failed_hint: String,
    pub missing: String,
    pub missing_detail: String,
    /// The app stopped listening before the page could deliver.
    pub gone_detail: String,
    /// The service would not redeem the delivered code.
    pub code_rejected: String,
}

impl Default for RelayText {
    fn default() -> Self {
        Self {
            lang: "en".into(),
            finishing: "Finishing sign-in…".into(),
            finishing_detail: "You can close this tab in a moment.".into(),
            done: "Signed in".into(),
            done_detail: "You can close this tab and return to the app.".into(),
            failed: "Sign-in failed".into(),
            failed_hint: "If the sign-in page showed a code, copy it and paste it into \
                          the app's sign-in window."
                .into(),
            missing: "Sign-in did not complete".into(),
            missing_detail: "No session was returned. Start again from the app.".into(),
            gone_detail: "The app is no longer listening. Start again from the app.".into(),
            code_rejected: "The sign-in code is invalid or has expired.".into(),
        }
    }
}

/// An in-progress browser sign-in.
pub struct LoginFlow {
    listener: TcpListener,
    endpoint: String,
    port: u16,
    pkce: Pkce,
    relay: RelayText,
    relay_page: String,
    cancel: Arc<AtomicBool>,
}

impl LoginFlow {
    /// Bind the loopback listener. Binding before opening the browser means the
    /// redirect can never arrive at a closed port.
    pub fn start(endpoint: &str, relay: RelayText) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint)?;
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .context("could not open a loopback port for the sign-in redirect")?;
        let port = listener.local_addr()?.port();
        listener
            .set_nonblocking(true)
            .context("could not configure the loopback listener")?;
        Ok(Self {
            listener,
            endpoint,
            port,
            pkce: Pkce::generate()?,
            relay_page: relay_page(&relay),
            relay,
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The PKCE pair; the verifier redeems a pasted code for this attempt.
    pub fn pkce(&self) -> &Pkce {
        &self.pkce
    }

    /// Set it to stop [`Self::wait`] and release the port.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// URL to open in the user's browser.
    pub fn login_url(&self) -> String {
        build_login_url(&self.endpoint, self.port, &self.pkce.challenge)
    }

    /// Block until the browser delivers a session, the attempt is cancelled,
    /// or the deadline passes.
    pub fn wait(&self, timeout: Duration) -> Result<Credentials> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(SignInCancelled.into());
            }
            if Instant::now() >= deadline {
                return Err(SignInTimedOut.into());
            }
            match self.listener.accept() {
                // One bad connection — a probe, a tab closed mid-request — is
                // that connection's problem, not the sign-in's.
                Ok((stream, _)) => match self.serve(stream) {
                    Ok(Some(credentials)) => return Ok(credentials),
                    Ok(None) => {}
                    Err(error) => eprintln!("warning: sign-in callback: {error:#}"),
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    return Err(anyhow::Error::from(error)
                        .context("the loopback listener failed while awaiting sign-in"));
                }
            }
        }
    }

    /// Handle one connection. Returns `Some` once the relay delivers a session.
    fn serve(&self, mut stream: TcpStream) -> Result<Option<Credentials>> {
        stream.set_nonblocking(false).ok();
        stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let Some(request) = read_request(&mut stream)? else {
            return Ok(None);
        };
        if request.method == "POST" && request.path.starts_with("/deliver") {
            // The relay page posts from this very origin. Anything else is
            // another site trying to push a session of its choosing.
            if let Some(origin) = request.origin.as_deref()
                && !self.is_own_origin(origin)
            {
                respond(&mut stream, "403 Forbidden", "text/plain; charset=utf-8", "forbidden");
                return Ok(None);
            }
            let delivered = parse_delivery(&request.body, &self.endpoint).and_then(|delivery| {
                match delivery {
                    Delivery::Session(credentials) => Ok(credentials),
                    Delivery::Code(code) => {
                        exchange_login_code(&self.endpoint, &code, &self.pkce.verifier)
                    }
                }
            });
            match delivered {
                Ok(credentials) => {
                    respond(&mut stream, "200 OK", "text/plain; charset=utf-8", "ok");
                    return Ok(Some(credentials));
                }
                Err(error) => {
                    let message = if error_is::<LoginCodeRejected>(&error) {
                        self.relay.code_rejected.clone()
                    } else {
                        format!("{error:#}")
                    };
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "text/plain; charset=utf-8",
                        &message,
                    );
                    return Ok(None);
                }
            }
        }
        respond(&mut stream, "200 OK", "text/html; charset=utf-8", &self.relay_page);
        Ok(None)
    }

    fn is_own_origin(&self, origin: &str) -> bool {
        let origin = origin.trim().trim_end_matches('/');
        origin == format!("http://127.0.0.1:{}", self.port)
            || origin == format!("http://localhost:{}", self.port)
    }
}

/// Build the bridge URL the browser opens.
///
/// `endpoint` is echoed back in the callback, which lets the relay verify that
/// the session belongs to the service the user actually chose. The PKCE
/// challenge asks the bridge for a one-time code instead of the session.
pub fn build_login_url(endpoint: &str, port: u16, code_challenge: &str) -> String {
    let redirect = format!("http://127.0.0.1:{port}/callback");
    format!(
        "{endpoint}{LOGIN_BRIDGE_PATH}?endpoint={}&redirect_to={}&code_challenge={}&code_challenge_method=S256",
        percent_encode(endpoint),
        percent_encode(&redirect),
        percent_encode(code_challenge)
    )
}

/// The bridge URL when no loopback listener could be opened: the page shows
/// the one-time code and leaves the rest to a paste.
pub fn build_code_login_url(endpoint: &str, code_challenge: &str) -> String {
    format!(
        "{endpoint}{LOGIN_BRIDGE_PATH}?endpoint={}&code_challenge={}&code_challenge_method=S256",
        percent_encode(endpoint),
        percent_encode(code_challenge)
    )
}

/// Turn a callback fragment carrying the session itself into credentials.
///
/// Accepts the fragment with or without its leading `#`.
pub fn credentials_from_fragment(fragment: &str, expected_endpoint: &str) -> Result<Credentials> {
    let params = parse_query(fragment.trim().trim_start_matches('#'));
    session_from_params(&params, expected_endpoint)
}

fn session_from_params(
    params: &HashMap<String, String>,
    expected_endpoint: &str,
) -> Result<Credentials> {
    let take = |key: &str| params.get(key).map(String::as_str).unwrap_or("").trim();
    let optional = |key: &str| {
        let value = take(key);
        (!value.is_empty()).then(|| value.to_owned())
    };
    let endpoint = checked_endpoint(params, expected_endpoint)?;
    Credentials::from_desktop_session(
        DesktopSession {
            access_token: take("access_token").to_owned(),
            refresh_token: take("refresh_token").to_owned(),
            expires_in: take("expires_in").parse().unwrap_or_default(),
            api_key: optional("api_key"),
            claude_api_key: optional("claude_api_key"),
            codex_api_key: optional("codex_api_key"),
        },
        &endpoint,
    )
}

/// The callback's `endpoint`, which must be the service the flow was started
/// against — a callback naming a different one means the browser was
/// redirected somewhere we did not send it.
fn checked_endpoint(params: &HashMap<String, String>, expected_endpoint: &str) -> Result<String> {
    let endpoint = params.get("endpoint").map(|value| value.trim()).unwrap_or("");
    if endpoint.is_empty() {
        return Ok(expected_endpoint.to_owned());
    }
    let normalized = normalize_endpoint(endpoint)?;
    if normalized != expected_endpoint {
        return Err(anyhow!(
            "the sign-in callback came from {normalized}, not {expected_endpoint}"
        ));
    }
    Ok(normalized)
}

fn delivery_from_params(
    params: &HashMap<String, String>,
    expected_endpoint: &str,
) -> Result<Delivery> {
    if params.contains_key("access_token") {
        return session_from_params(params, expected_endpoint).map(Delivery::Session);
    }
    if let Some(code) = params.get("code") {
        checked_endpoint(params, expected_endpoint)?;
        return normalize_login_code(code).map(Delivery::Code);
    }
    Err(anyhow!("the sign-in callback did not include a session"))
}

/// Read what the relay page posted: the callback fragment.
pub fn parse_delivery(fragment: &str, expected_endpoint: &str) -> Result<Delivery> {
    delivery_from_params(
        &parse_query(fragment.trim().trim_start_matches('#')),
        expected_endpoint,
    )
}

/// Read what the user pasted into the sign-in window.
///
/// Accepts the code shown on the sign-in page (any case, with or without its
/// dash or spaces), or the whole address-bar link of the callback page that
/// failed to load — `http://127.0.0.1:…/callback#code=…`, or the session
/// fragment an older bridge puts there.
pub fn parse_pasted_login(text: &str, expected_endpoint: &str) -> Result<Delivery> {
    let text = text.trim();
    if text.is_empty() {
        return Err(NotALoginCode.into());
    }
    if !text.contains('=') {
        return normalize_login_code(text).map(Delivery::Code);
    }
    let mut params = HashMap::new();
    let (before_fragment, fragment) = match text.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (text, None),
    };
    match before_fragment.split_once('?') {
        Some((_, query)) => params.extend(parse_query(query)),
        // Bare `code=…&endpoint=…`, as copied from a fragment.
        None if fragment.is_none() => params.extend(parse_query(before_fragment)),
        None => {}
    }
    if let Some(fragment) = fragment {
        params.extend(parse_query(fragment));
    }
    if !params.contains_key("code") && !params.contains_key("access_token") {
        return Err(NotALoginCode.into());
    }
    delivery_from_params(&params, expected_endpoint)
}

/// The code as the service reads it: upper case, no dash or spaces.
pub fn normalize_login_code(raw: &str) -> Result<String> {
    let code: String = raw
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .map(|character| character.to_ascii_uppercase())
        .collect();
    if (6..=32).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(code)
    } else {
        Err(NotALoginCode.into())
    }
}

/// Redeem a one-time code with the verifier of the attempt it was issued to.
pub fn exchange_login_code(endpoint: &str, code: &str, verifier: &str) -> Result<Credentials> {
    let session = Client::new(endpoint)
        .exchange_desktop_code(code, verifier)
        .map_err(|error| {
            let rejected = error.chain().any(|cause| {
                cause
                    .downcast_ref::<crate::http::ApiError>()
                    .is_some_and(|api| api.reason == CODE_INVALID_REASON)
            });
            if rejected {
                anyhow::Error::from(LoginCodeRejected)
            } else {
                error
            }
        })?;
    Credentials::from_desktop_session(session, endpoint)
}

/// Normalize a service origin: absolute http(s) URL, no trailing slash.
pub fn normalize_endpoint(endpoint: &str) -> Result<String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(anyhow!("the service address is required"));
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(anyhow!("the service address must start with http:// or https://"));
    }
    let without_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    if without_scheme.is_empty() || without_scheme.starts_with('/') {
        return Err(anyhow!("the service address is missing a host"));
    }
    Ok(trimmed.to_owned())
}

/// The page served at the loopback callback.
///
/// It copies the fragment — which the browser withheld from the request — back
/// to this server, then tells the user they can return to the app. Its text
/// arrives as JSON so it can be in the app's language without any escaping
/// games in the markup.
fn relay_page(text: &RelayText) -> String {
    let strings = serde_json::to_string(text)
        .unwrap_or_else(|_| "{}".into())
        // Never let a string close the script element early.
        .replace("</", "<\\/");
    RELAY_PAGE
        .replace("__LANG__", &html_escape(&text.lang))
        .replace("__TITLE__", &html_escape(&text.finishing))
        .replace("__TEXT__", &strings)
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const RELAY_PAGE: &str = r#"<!doctype html>
<html lang="__LANG__">
<head><meta charset="utf-8"><title>__TITLE__</title>
<style>
body{font:15px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif;margin:0;
display:flex;align-items:center;justify-content:center;height:100vh;
background:#0f1115;color:#e6e8eb}
main{text-align:center;max-width:30rem;padding:2rem}
h1{font-size:1.1rem;font-weight:600;margin:0 0 .5rem}
p{margin:0;color:#9aa1ab}
p+p{margin-top:.75rem;font-size:13px}
</style></head>
<body><main id="status"></main>
<script>
(function () {
  var T = __TEXT__;
  var status = document.getElementById("status");
  function show(title, detail, hint) {
    status.innerHTML = "";
    var heading = document.createElement("h1");
    heading.textContent = title;
    status.appendChild(heading);
    var body = document.createElement("p");
    body.textContent = detail || "";
    status.appendChild(body);
    if (hint) {
      var extra = document.createElement("p");
      extra.textContent = hint;
      status.appendChild(extra);
    }
  }
  var hash = window.location.hash || "";
  if (!hash || hash.length < 2) {
    show(T.missing, T.missing_detail);
    return;
  }
  show(T.finishing, T.finishing_detail);
  fetch("/deliver", { method: "POST", body: hash })
    .then(function (response) {
      if (response.ok) {
        show(T.done, T.done_detail);
      } else {
        return response.text().then(function (text) {
          show(T.failed, text, T.failed_hint);
        });
      }
    })
    .catch(function () {
      show(T.failed, T.gone_detail, T.failed_hint);
    });
})();
</script></body></html>"#;

struct HttpRequest {
    method: String,
    path: String,
    origin: Option<String>,
    body: String,
}

/// Read one request. Returns `None` when the peer sends nothing usable.
fn read_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream.try_clone().context("could not read the request")?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return Ok(None);
    };
    let Some(path) = parts.next() else {
        return Ok(None);
    };

    let mut content_length = 0usize;
    let mut origin = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("origin") {
                origin = Some(value.trim().to_owned());
            }
        }
    }

    // Cap the body: the relay posts a fragment, never bulk data.
    let mut body = vec![0u8; content_length.min(64 * 1024)];
    if !body.is_empty() {
        reader.read_exact(&mut body).context("could not read the request body")?;
    }

    Ok(Some(HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        origin,
        body: String::from_utf8_lossy(&body).into_owned(),
    }))
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Parse `a=1&b=2` into a map, percent-decoding both sides.
fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((percent_decode(key), percent_decode(value)))
        })
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(decoded) => {
                        out.push(decoded);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = "https://cloud.example.org";

    #[test]
    fn normalizes_endpoints() {
        assert_eq!(normalize_endpoint("https://a.org/").unwrap(), "https://a.org");
        assert_eq!(normalize_endpoint("  http://a.org  ").unwrap(), "http://a.org");
        assert!(normalize_endpoint("").is_err());
        assert!(normalize_endpoint("a.org").is_err());
        assert!(normalize_endpoint("ftp://a.org").is_err());
        assert!(normalize_endpoint("https://").is_err());
    }

    #[test]
    fn login_url_carries_an_encoded_loopback_redirect() {
        let url = build_login_url(SERVICE, 51789, "abc_DEF-123");
        assert!(url.starts_with("https://cloud.example.org/auth/paseo?"));
        assert!(url.contains("endpoint=https%3A%2F%2Fcloud.example.org"));
        assert!(url.contains("redirect_to=http%3A%2F%2F127.0.0.1%3A51789%2Fcallback"));
        assert!(url.contains("&code_challenge=abc_DEF-123&code_challenge_method=S256"));
    }

    #[test]
    fn parses_a_complete_callback_fragment() {
        let fragment = "#access_token=at&refresh_token=rt&expires_in=3600\
                        &endpoint=https%3A%2F%2Fcloud.example.org\
                        &api_key=sk-gateway&claude_api_key=sk-claude&codex_api_key=sk-codex";
        let credentials = credentials_from_fragment(fragment, SERVICE).expect("parse");
        assert_eq!(credentials.access_token, "at");
        assert_eq!(credentials.refresh_token, "rt");
        assert_eq!(credentials.endpoint, SERVICE);
        assert_eq!(credentials.api_key.as_deref(), Some("sk-gateway"));
        assert_eq!(credentials.claude_api_key.as_deref(), Some("sk-claude"));
        assert_eq!(credentials.codex_api_key.as_deref(), Some("sk-codex"));
        assert!(credentials.expires_at > now_unix());
    }

    #[test]
    fn fragment_without_leading_hash_is_accepted() {
        let credentials =
            credentials_from_fragment("access_token=at&refresh_token=rt&expires_in=60", SERVICE)
                .expect("parse");
        assert_eq!(credentials.endpoint, SERVICE);
        assert!(credentials.api_key.is_none());
    }

    #[test]
    fn incomplete_callbacks_are_rejected() {
        for fragment in [
            "",
            "#access_token=at",
            "#access_token=at&refresh_token=rt",
            "#access_token=at&refresh_token=rt&expires_in=0",
            "#access_token=&refresh_token=rt&expires_in=60",
        ] {
            assert!(
                credentials_from_fragment(fragment, SERVICE).is_err(),
                "should reject: {fragment}"
            );
        }
    }

    #[test]
    fn a_callback_from_another_service_is_rejected() {
        let fragment = "#access_token=at&refresh_token=rt&expires_in=60\
                        &endpoint=https%3A%2F%2Fevil.example.net";
        let error = credentials_from_fragment(fragment, SERVICE).expect_err("should reject");
        assert!(error.to_string().contains("evil.example.net"));
    }

    #[test]
    fn group_binding_survives_a_round_trip() {
        // Without this the settings page shows "account default" as selected
        // after a restart while requests keep routing through the group.
        let credentials = Credentials {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: 42,
            endpoint: "https://a.org".into(),
            api_key: Some("sk-group".into()),
            group_id: Some(7),
            ..Credentials::default()
        };
        let encoded = serde_json::to_string(&credentials).expect("encode");
        let decoded: Credentials = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, credentials);
        assert_eq!(decoded.group_id, Some(7));
    }

    #[test]
    fn credentials_written_before_group_support_still_load() {
        // Files written by an earlier build have no `group_id`.
        let decoded: Credentials = serde_json::from_str(
            r#"{"access_token":"at","refresh_token":"rt","expires_at":1,"endpoint":"https://a.org"}"#,
        )
        .expect("decode");
        assert_eq!(decoded.group_id, None);
        assert_eq!(decoded.access_token, "at");
    }

    #[test]
    fn refresh_window_opens_before_expiry() {
        let credentials = Credentials {
            expires_at: 1_000_000,
            ..Credentials::default()
        };
        assert!(!credentials.needs_refresh(1_000_000 - REFRESH_SKEW_SECONDS - 1));
        assert!(credentials.needs_refresh(1_000_000 - REFRESH_SKEW_SECONDS));
        assert!(credentials.needs_refresh(1_000_001));
    }

    #[test]
    fn refresh_keeps_the_old_token_when_the_service_does_not_rotate_it() {
        let mut credentials = Credentials {
            access_token: "old".into(),
            refresh_token: "keep-me".into(),
            expires_at: 0,
            ..Credentials::default()
        };
        credentials.apply_refresh(
            &crate::client::TokenPair {
                access_token: "new".into(),
                refresh_token: String::new(),
                expires_in: 3600,
            },
            1_000,
        );
        assert_eq!(credentials.access_token, "new");
        assert_eq!(credentials.refresh_token, "keep-me");
        assert_eq!(credentials.expires_at, 4_600);
    }

    #[test]
    fn percent_round_trip() {
        let raw = "https://a.org/x?y=1 2&z=✓";
        assert_eq!(percent_decode(&percent_encode(raw)), raw);
        assert_eq!(percent_decode("a+b"), "a b");
        // A stray percent must not panic or eat the rest of the string.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn relay_page_posts_the_fragment_back() {
        let page = relay_page(&RelayText::default());
        assert!(page.contains("location.hash"));
        assert!(page.contains(r#"fetch("/deliver""#));
        assert!(page.contains("Finishing sign-in"));
    }

    #[test]
    fn relay_page_text_cannot_break_out_of_its_script() {
        let text = RelayText {
            lang: r#"zh-CN"><script>"#.into(),
            finishing: "</script><script>alert(1)</script>".into(),
            ..RelayText::default()
        };
        let page = relay_page(&text);
        assert!(!page.contains("</script><script>alert(1)"));
        assert!(page.contains("&lt;/script&gt;"));
        assert!(!page.contains(r#"lang="zh-CN"><script>"#));
    }

    #[test]
    fn flow_binds_loopback_and_builds_a_matching_url() {
        let flow = LoginFlow::start(SERVICE, RelayText::default()).expect("bind");
        assert!(flow.port() > 0);
        let url = flow.login_url();
        assert!(url.contains(&format!("127.0.0.1%3A{}", flow.port())));
        assert!(url.contains(&format!("code_challenge={}", flow.pkce().challenge)));
    }

    #[test]
    fn wait_times_out_without_a_callback() {
        let flow = LoginFlow::start(SERVICE, RelayText::default()).expect("bind");
        let error = flow
            .wait(Duration::from_millis(120))
            .expect_err("should time out");
        assert!(error.to_string().contains("timed out"));
        assert!(error_is::<SignInTimedOut>(&error));
    }

    #[test]
    fn a_cancelled_wait_returns_at_once() {
        let flow = LoginFlow::start(SERVICE, RelayText::default()).expect("bind");
        flow.cancel_handle().store(true, Ordering::Relaxed);
        let started = Instant::now();
        let error = flow.wait(Duration::from_secs(30)).expect_err("cancelled");
        assert!(error_is::<SignInCancelled>(&error));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn pkce_pair_follows_rfc_7636() {
        // RFC 7636, Appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let pair = Pkce::generate().expect("random");
        assert_eq!(pair.verifier.len(), 43);
        assert_eq!(pair.challenge.len(), 43);
        assert_eq!(pkce_challenge(&pair.verifier), pair.challenge);
        assert_ne!(Pkce::generate().expect("random").verifier, pair.verifier);
    }

    #[test]
    fn each_sign_in_is_its_own_session() {
        let fragment = "#access_token=at&refresh_token=rt&expires_in=60";
        let first = credentials_from_fragment(fragment, SERVICE).expect("parse");
        let second = credentials_from_fragment(fragment, SERVICE).expect("parse");
        assert_eq!(first.session_id.len(), 32);
        assert_ne!(first.session_id, second.session_id);
    }

    #[test]
    fn a_code_callback_is_a_code_delivery() {
        let delivery = parse_delivery(
            "#code=K7QM-3XPD&endpoint=https%3A%2F%2Fcloud.example.org",
            SERVICE,
        )
        .expect("parse");
        assert!(matches!(delivery, Delivery::Code(code) if code == "K7QM3XPD"));

        let foreign = parse_delivery("#code=K7QM3XPD&endpoint=https%3A%2F%2Fevil.net", SERVICE);
        assert!(foreign.is_err());
    }

    #[test]
    fn an_old_bridge_still_delivers_the_session_itself() {
        let delivery = parse_delivery("#access_token=at&refresh_token=rt&expires_in=60", SERVICE)
            .expect("parse");
        assert!(
            matches!(delivery, Delivery::Session(credentials) if credentials.access_token == "at")
        );
    }

    #[test]
    fn pasted_codes_are_forgiving_about_case_and_spacing() {
        for pasted in ["K7QM-3XPD", "k7qm-3xpd", "  K7QM 3XPD\n", "K7QM3XPD"] {
            let delivery = parse_pasted_login(pasted, SERVICE).expect(pasted);
            assert!(
                matches!(&delivery, Delivery::Code(code) if code == "K7QM3XPD"),
                "{pasted}: {delivery:?}"
            );
        }
    }

    #[test]
    fn a_pasted_address_bar_link_yields_its_code() {
        let pasted = "http://127.0.0.1:51789/callback#code=K7QM-3XPD\
                      &endpoint=https%3A%2F%2Fcloud.example.org";
        let delivery = parse_pasted_login(pasted, SERVICE).expect("parse");
        assert!(matches!(delivery, Delivery::Code(code) if code == "K7QM3XPD"));

        let bare_fragment = parse_pasted_login("code=K7QM3XPD", SERVICE).expect("parse");
        assert!(matches!(bare_fragment, Delivery::Code(_)));
    }

    #[test]
    fn a_pasted_legacy_link_yields_its_session() {
        let pasted = "http://127.0.0.1:51789/callback#access_token=at&refresh_token=rt\
                      &expires_in=60&endpoint=https%3A%2F%2Fcloud.example.org&api_key=sk-1";
        let delivery = parse_pasted_login(pasted, SERVICE).expect("parse");
        assert!(matches!(
            delivery,
            Delivery::Session(credentials) if credentials.api_key.as_deref() == Some("sk-1")
        ));
    }

    #[test]
    fn pasted_nonsense_is_not_a_code() {
        for pasted in [
            "",
            "   ",
            "abc",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOjF9.sig",
            "https://cloud.example.org/auth/paseo?endpoint=x&code_challenge=abc",
        ] {
            let error = parse_pasted_login(pasted, SERVICE).expect_err(pasted);
            assert!(error_is::<NotALoginCode>(&error), "{pasted}: {error:#}");
        }
    }

    /// The exchange endpoint, redeeming one code for one verifier and
    /// refusing everything else the way the service does.
    fn spawn_exchange_server(code: &'static str, verifier: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Ok(Some(request)) = read_request(&mut stream) else {
                    continue;
                };
                let body: serde_json::Value =
                    serde_json::from_str(&request.body).unwrap_or_default();
                let accepted = request.path == "/api/v1/auth/desktop-session/exchange"
                    && body["code"] == code
                    && body["code_verifier"] == verifier.as_str();
                if accepted {
                    respond(
                        &mut stream,
                        "200 OK",
                        "application/json",
                        r#"{"code":0,"message":"ok","data":{"access_token":"at","refresh_token":"rt","expires_in":900,"token_type":"Bearer","api_key":"sk-general","claude_api_key":"sk-claude"}}"#,
                    );
                } else {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "application/json",
                        r#"{"code":400,"reason":"DESKTOP_LOGIN_CODE_INVALID","message":"invalid or expired sign-in code"}"#,
                    );
                }
            }
        });
        endpoint
    }

    #[test]
    fn a_code_is_redeemed_with_its_verifier() {
        let pair = Pkce::generate().expect("random");
        let endpoint = spawn_exchange_server("K7QM3XPD", pair.verifier.clone());

        let credentials =
            exchange_login_code(&endpoint, "K7QM3XPD", &pair.verifier).expect("exchange");
        assert_eq!(credentials.access_token, "at");
        assert_eq!(credentials.endpoint, endpoint);
        assert_eq!(credentials.api_key.as_deref(), Some("sk-general"));
        assert_eq!(credentials.claude_api_key.as_deref(), Some("sk-claude"));
        assert!(credentials.codex_api_key.is_none());
        assert!(!credentials.session_id.is_empty());

        let error =
            exchange_login_code(&endpoint, "K7QM3XPD", "someone-else").expect_err("wrong");
        assert!(error_is::<LoginCodeRejected>(&error), "{error:#}");
    }

    fn post_delivery(port: u16, origin: &str, body: &'static str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write!(
            stream,
            "POST /deliver HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: {origin}\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn the_loopback_redeems_a_delivered_code() {
        let flow = LoginFlow::start(SERVICE, RelayText::default()).expect("bind");
        // Point the flow at a stand-in service that knows this attempt.
        let endpoint = spawn_exchange_server("K7QM3XPD", flow.pkce().verifier.clone());
        let flow = LoginFlow { endpoint, ..flow };
        let port = flow.port();
        let waiter = std::thread::spawn(move || flow.wait(Duration::from_secs(30)));

        let response =
            post_delivery(port, &format!("http://127.0.0.1:{port}"), "#code=K7QM-3XPD");
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let credentials = waiter.join().unwrap().expect("delivered");
        assert_eq!(credentials.access_token, "at");
    }

    #[test]
    fn another_site_cannot_push_a_delivery() {
        let flow = LoginFlow::start(SERVICE, RelayText::default()).expect("bind");
        let port = flow.port();
        let cancel = flow.cancel_handle();
        let waiter = std::thread::spawn(move || flow.wait(Duration::from_secs(30)));

        let response = post_delivery(
            port,
            "https://evil.example.net",
            "#access_token=at&refresh_token=rt&expires_in=60",
        );
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");

        // Refused without ending the attempt: it is still waiting.
        cancel.store(true, Ordering::Relaxed);
        let error = waiter.join().unwrap().expect_err("still waiting");
        assert!(error_is::<SignInCancelled>(&error));
    }
}
