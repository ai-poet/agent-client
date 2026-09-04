//! Managed cloud account, gateway routing, and agent CLI installation.
//!
//! Everything this fork adds on top of upstream Waku lives here, in a crate of
//! its own. Upstream files get small hook points that call in; nothing of ours
//! is scattered through them. That is what keeps the weekly upstream merge to a
//! handful of one-line conflicts at worst — see `docs/FORK.md`.
//!
//! The crate is deliberately free of GPUI and of the upstream crates, so its
//! logic compiles and tests in seconds without building the UI.
//!
//! ```text
//! desktop process                        daemon process
//! ┌──────────────────────────┐           ┌────────────────────────────┐
//! │ auth: browser sign-in    │           │ gateway::env_for(provider) │
//! │ client: /auth/me, /keys  │           │   ↓ applied at spawn       │
//! │ credentials  (local file)│           │ agent CLI → gateway        │
//! └───────────┬──────────────┘           └──────────────┬─────────────┘
//!             │ gateway keys only, via DaemonSettings.extra
//!             └───────────────────────────────────────┘
//! ```
//!
//! OAuth tokens never reach the daemon; only derived gateway keys do.

pub mod auth;
pub mod brand;
pub mod cli_detect;
pub mod cli_install;
pub mod client;
pub mod codex_compat;
pub mod custom_api;
pub mod env_conflicts;
pub mod gateway;
pub mod global_config;
pub mod http;
pub mod migrate;
pub mod node_install;
pub mod onboarding;
pub mod pay;

pub use auth::Credentials;
pub use client::Client;
pub use gateway::GatewayConfig;

/// Convenience constructor for a client bound to the branded service.
pub fn default_client() -> Client {
    Client::new(brand::MANAGED_SERVICE_URL)
}

/// The stored session cannot be renewed any more: the service refused the
/// refresh token itself.
///
/// The service rotates refresh tokens — each renewal invalidates the token it
/// was made with — so this is what a token looks like after another holder
/// used it, after the account was revoked, or after it simply expired. None
/// of those heal on retry; the only way forward is a fresh sign-in, and the
/// desktop should say so instead of failing every request from now on.
#[derive(Clone, Debug)]
pub struct SessionEnded {
    /// The service's reason code, e.g. `REFRESH_TOKEN_INVALID`.
    pub reason: String,
    pub message: String,
}

impl std::fmt::Display for SessionEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the sign-in is no longer valid: {}", self.message)?;
        if !self.reason.is_empty() {
            write!(f, " ({})", self.reason)?;
        }
        Ok(())
    }
}

impl std::error::Error for SessionEnded {}

/// Whether `error` (from any authenticated call) means the session is over
/// rather than the network being flaky.
pub fn session_ended(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<SessionEnded>().is_some())
}

/// Where the session is persisted. The desktop uses the credential file; the
/// tests use memory so a refresh probe never touches the real sign-in.
pub trait CredentialStore {
    fn load(&self) -> Option<Credentials>;
    fn save(&self, credentials: &Credentials) -> anyhow::Result<()>;
}

/// The store the app uses: `~/.cheaprouter/cloud-account.json`.
pub struct FileCredentialStore;

impl CredentialStore for FileCredentialStore {
    fn load(&self) -> Option<Credentials> {
        Credentials::load()
    }

    fn save(&self, credentials: &Credentials) -> anyhow::Result<()> {
        credentials.save()
    }
}

/// Refresh `credentials` when the access token is close to expiring.
///
/// Returns `true` when a refresh happened and the caller should persist the
/// updated credentials. A failed refresh is reported as an error; when the
/// service refused the token itself the error carries [`SessionEnded`], which
/// [`session_ended`] recognises, so the caller can prompt for sign-in again
/// rather than retrying forever.
///
/// Single-flight across the process: the balance poll, the details load, and a
/// user action can all hold stale clones concurrently, and if the service
/// rotates refresh tokens, two racing refreshes would invalidate each other —
/// the loser then overwrites the credential file with a dead token pair and
/// the session is gone on the next restart. Inside the lock the caller's clone
/// is first reconciled with the file, so whoever lost the race adopts the
/// winner's tokens instead of refreshing again.
pub fn refresh_if_needed(credentials: &mut Credentials) -> anyhow::Result<bool> {
    refresh_if_needed_with(credentials, &FileCredentialStore)
}

/// [`refresh_if_needed`] against an explicit store.
pub fn refresh_if_needed_with(
    credentials: &mut Credentials,
    store: &dyn CredentialStore,
) -> anyhow::Result<bool> {
    static REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = REFRESH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut changed = false;
    if let Some(stored) = store.load()
        && stored.endpoint == credentials.endpoint
        && stored.expires_at > credentials.expires_at
    {
        *credentials = stored;
        changed = true;
    }

    let now = auth::now_unix();
    if !credentials.needs_refresh(now) {
        return Ok(changed);
    }
    let client = Client::new(credentials.endpoint.clone());
    let pair = match client.refresh(&credentials.refresh_token) {
        Ok(pair) => pair,
        Err(error) => {
            if let Some(api) = error.downcast_ref::<http::ApiError>()
                && api.is_auth_rejection()
            {
                return Err(SessionEnded {
                    reason: api.reason.clone(),
                    message: api.message.clone(),
                }
                .into());
            }
            return Err(error);
        }
    };
    // A shape mismatch would deserialize to empty strings; storing those
    // would sign the user out on the next launch with no error anywhere.
    if pair.access_token.trim().is_empty() {
        anyhow::bail!("the service answered the refresh without an access token");
    }
    credentials.apply_refresh(&pair, now);
    store.save(credentials)?;
    Ok(true)
}

/// Renew the session if needed, then hand back a client bound to it.
///
/// Every authenticated call goes through here. Calling the API with a token
/// that expired minutes ago is the common case for a desktop app that sits
/// open — without this, the first action after lunch fails with a 401 and the
/// user has no idea why.
pub fn authenticated(credentials: &mut Credentials) -> anyhow::Result<Client> {
    refresh_if_needed(credentials)?;
    Ok(Client::new(credentials.endpoint.clone()))
}

/// Find or mint a gateway key bound to `group_id`.
///
/// Reuses an existing active key for that group before creating one. Minting
/// on every selection would leave a trail of dead keys on the account, and the
/// user has no way to clean them up from this app.
pub fn ensure_key_for_group(
    credentials: &mut Credentials,
    group_id: Option<i64>,
) -> anyhow::Result<client::ApiKey> {
    let client = authenticated(credentials)?;
    let existing = client.list_keys(&credentials.access_token)?;
    let reusable = existing.items.into_iter().find(|key| {
        key.group_id == group_id && !key.key.is_empty() && !key.status.eq_ignore_ascii_case("disabled")
    });
    match reusable {
        Some(key) => Ok(key),
        None => client.create_key(
            &credentials.access_token,
            &format!("{} desktop", brand::DISPLAY_NAME),
            group_id,
        ),
    }
}

#[cfg(test)]
mod platform_binding_tests {
    use super::*;

    #[test]
    fn platform_bindings_map_to_the_right_slots() {
        let credentials = Credentials {
            claude_group_id: Some(3),
            codex_group_id: Some(7),
            group_id: Some(11),
            ..Credentials::default()
        };
        assert_eq!(bound_group_for_platform(&credentials, "anthropic"), Some(3));
        assert_eq!(bound_group_for_platform(&credentials, "openai"), Some(7));
        assert_eq!(bound_group_for_platform(&credentials, "misc"), Some(11));
    }
}

/// Bind a platform's routing to a group, or back to the account default.
///
/// Groups are platform-scoped on the service (`anthropic`, `openai`, …), so
/// "switch the group" naturally means "switch it for that CLI": an
/// `anthropic` group rebinds Claude's key, an `openai` group rebinds Codex's,
/// anything else rebinds the general fallback key. `None` clears the
/// platform-specific binding, which drops that CLI back to the general key
/// from sign-in — the gateway env lookup already falls back that way.
///
/// Saves on success; the caller publishes the refreshed daemon settings.
pub fn bind_group_for_platform(
    credentials: &mut Credentials,
    platform: &str,
    group_id: Option<i64>,
) -> anyhow::Result<()> {
    let key = match group_id {
        Some(id) => Some(ensure_key_for_group(credentials, Some(id))?.key),
        None => None,
    };
    match platform {
        "anthropic" => {
            credentials.claude_api_key = key;
            credentials.claude_group_id = group_id;
        }
        "openai" => {
            credentials.codex_api_key = key;
            credentials.codex_group_id = group_id;
        }
        _ => {
            // No dedicated slot for this platform; route the general key.
            if key.is_some() {
                credentials.api_key = key;
            }
            credentials.group_id = group_id;
        }
    }
    credentials.save()?;
    Ok(())
}

/// The group currently bound for a platform, if any.
pub fn bound_group_for_platform(credentials: &Credentials, platform: &str) -> Option<i64> {
    match platform {
        "anthropic" => credentials.claude_group_id,
        "openai" => credentials.codex_group_id,
        _ => credentials.group_id,
    }
}

/// Build the routing configuration the daemon needs from a signed-in session.
pub fn gateway_config_from(credentials: &Credentials, enabled: bool) -> GatewayConfig {
    GatewayConfig {
        enabled,
        endpoint: credentials.endpoint.clone(),
        api_key: credentials.api_key.clone(),
        claude_api_key: credentials.claude_api_key.clone(),
        codex_api_key: credentials.codex_api_key.clone(),
        codex_model: None,
    }
}

/// The refresh flow against a stand-in for the service, with the service's
/// real semantics: presenting a live refresh token rotates it and the old
/// one dies at once; presenting a dead one is refused with `401
/// REFRESH_TOKEN_INVALID`. This is the loop that reproduces "signed in, then
/// signed out": two clients holding one refresh token.
#[cfg(test)]
mod refresh_tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    use super::*;

    struct MemoryStore(Mutex<Option<Credentials>>);

    impl MemoryStore {
        fn empty() -> Self {
            Self(Mutex::new(None))
        }

        fn saved(&self) -> Option<Credentials> {
            self.0.lock().unwrap().clone()
        }
    }

    impl CredentialStore for MemoryStore {
        fn load(&self) -> Option<Credentials> {
            self.0.lock().unwrap().clone()
        }

        fn save(&self, credentials: &Credentials) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = Some(credentials.clone());
            Ok(())
        }
    }

    /// The refresh endpoint, rotating: one live token at a time per family.
    struct RotatingRefreshServer {
        endpoint: String,
        /// Every refresh token the server has been shown, in order.
        presented: Arc<Mutex<Vec<String>>>,
    }

    fn spawn_server(initial_token: &str) -> RotatingRefreshServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let live = Arc::new(Mutex::new(vec![initial_token.to_owned()]));
        let presented = Arc::new(Mutex::new(Vec::new()));
        let seen = presented.clone();
        std::thread::spawn(move || {
            let mut generation = 1u32;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some((path, body)) = read_request(&mut stream) else { continue };
                if path != "/api/v1/auth/refresh" {
                    respond(&mut stream, 404, r#"{"code":404,"message":"not found"}"#);
                    continue;
                }
                let token = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| value.get("refresh_token")?.as_str().map(str::to_owned))
                    .unwrap_or_default();
                seen.lock().unwrap().push(token.clone());
                let mut live = live.lock().unwrap();
                if let Some(index) = live.iter().position(|candidate| *candidate == token) {
                    live.remove(index);
                    generation += 1;
                    let next = format!("rt_{generation}");
                    live.push(next.clone());
                    respond(
                        &mut stream,
                        200,
                        &format!(
                            r#"{{"code":0,"message":"ok","data":{{"access_token":"at_{generation}","refresh_token":"{next}","expires_in":900,"token_type":"Bearer"}}}}"#
                        ),
                    );
                } else {
                    respond(
                        &mut stream,
                        401,
                        r#"{"code":401,"reason":"REFRESH_TOKEN_INVALID","message":"invalid refresh token"}"#,
                    );
                }
            }
        });
        RotatingRefreshServer {
            endpoint,
            presented,
        }
    }

    fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
        let mut reader = BufReader::new(stream.try_clone().ok()?);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).ok()?;
        let path = request_line.split_whitespace().nth(1)?.to_owned();
        let mut content_length = 0usize;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? == 0 {
                break;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            reader.read_exact(&mut body).ok()?;
        }
        Some((path, String::from_utf8_lossy(&body).into_owned()))
    }

    fn respond(stream: &mut TcpStream, status: u16, body: &str) {
        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            _ => "Not Found",
        };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn stale_desktop_credentials(endpoint: &str, refresh_token: &str) -> Credentials {
        Credentials {
            access_token: "at_stale".into(),
            refresh_token: refresh_token.into(),
            // Long expired, so a refresh is due.
            expires_at: 1,
            endpoint: endpoint.to_owned(),
            ..Credentials::default()
        }
    }

    #[test]
    fn refresh_is_refused_once_another_client_rotated_the_token() {
        let server = spawn_server("rt_1");
        // The browser session, which the login bridge used to hand the
        // desktop, refreshes first — it rotates rt_1 away.
        let browser = Client::new(server.endpoint.clone())
            .refresh("rt_1")
            .expect("the browser's refresh succeeds");
        assert_eq!(browser.refresh_token, "rt_2");

        // The desktop still holds rt_1.
        let store = MemoryStore::empty();
        let mut desktop = stale_desktop_credentials(&server.endpoint, "rt_1");
        let error = refresh_if_needed_with(&mut desktop, &store).expect_err("rt_1 is dead");

        // The symptom the user sees is "signed out"; the app must be able to
        // tell this apart from a network hiccup and say so.
        assert!(session_ended(&error), "{error:#}");
        assert!(error.to_string().contains("invalid refresh token"), "{error:#}");
        // Nothing was written: the dead pair is not re-saved.
        assert!(store.saved().is_none());
        assert_eq!(
            server.presented.lock().unwrap().as_slice(),
            ["rt_1", "rt_1"]
        );
    }

    #[test]
    fn refresh_rotates_and_persists_the_new_pair() {
        let server = spawn_server("rt_1");
        let store = MemoryStore::empty();
        let mut desktop = stale_desktop_credentials(&server.endpoint, "rt_1");

        assert!(refresh_if_needed_with(&mut desktop, &store).expect("refresh"));

        assert_eq!(desktop.access_token, "at_2");
        assert_eq!(desktop.refresh_token, "rt_2");
        assert!(desktop.expires_at > auth::now_unix() + 800);
        assert_eq!(store.saved().as_ref(), Some(&desktop));
        // And the renewed pair is accepted next time round. Expire the
        // stored copy too, or the reconcile step would rightly hand back the
        // still-valid renewal instead of refreshing.
        desktop.expires_at = 1;
        store.save(&desktop).unwrap();
        assert!(refresh_if_needed_with(&mut desktop, &store).expect("second refresh"));
        assert_eq!(desktop.refresh_token, "rt_3");
        assert_eq!(
            server.presented.lock().unwrap().as_slice(),
            ["rt_1", "rt_2"]
        );
    }

    #[test]
    fn a_stale_clone_adopts_the_stored_renewal_instead_of_refreshing_again() {
        let server = spawn_server("rt_1");
        let store = MemoryStore::empty();
        let mut first = stale_desktop_credentials(&server.endpoint, "rt_1");
        let mut second = first.clone();

        assert!(refresh_if_needed_with(&mut first, &store).expect("first"));
        // The second caller still holds rt_1 — the token the first just
        // burned. It must adopt the stored renewal rather than present it.
        assert!(refresh_if_needed_with(&mut second, &store).expect("second"));
        assert_eq!(second.refresh_token, "rt_2");
        assert_eq!(server.presented.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_network_failure_is_not_the_end_of_the_session() {
        // A port nothing listens on.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let store = MemoryStore::empty();
        let mut desktop = stale_desktop_credentials(&endpoint, "rt_1");

        let error = refresh_if_needed_with(&mut desktop, &store).expect_err("unreachable");
        assert!(!session_ended(&error), "{error:#}");
        assert_eq!(desktop.refresh_token, "rt_1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_client_targets_the_branded_service() {
        assert_eq!(default_client().endpoint(), brand::MANAGED_SERVICE_URL);
    }

    #[test]
    fn gateway_config_carries_keys_but_never_tokens() {
        let credentials = Credentials {
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            expires_at: 0,
            endpoint: "https://a.org".into(),
            api_key: Some("sk-general".into()),
            claude_api_key: Some("sk-claude".into()),
            codex_api_key: None,
            ..Credentials::default()
        };
        let config = gateway_config_from(&credentials, true);
        assert!(config.is_usable());

        // The whole point of the split: nothing published to the daemon may
        // contain the OAuth tokens.
        let encoded = serde_json::to_string(&config).expect("encode");
        assert!(!encoded.contains("secret-access"));
        assert!(!encoded.contains("secret-refresh"));
        assert!(encoded.contains("sk-claude"));
    }

    #[test]
    fn a_fresh_session_is_not_refreshed() {
        let mut credentials = Credentials {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: auth::now_unix() + 3600,
            endpoint: "https://a.org".into(),
            ..Credentials::default()
        };
        // Far from expiry, so this must return without touching the network.
        assert!(!refresh_if_needed(&mut credentials).expect("no refresh"));
    }
}
