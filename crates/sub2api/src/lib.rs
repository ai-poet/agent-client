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
pub mod cli_install;
pub mod client;
pub mod codex_compat;
pub mod custom_api;
pub mod gateway;
pub mod global_config;
pub mod http;
pub mod migrate;
pub mod node_install;
pub mod pay;

pub use auth::Credentials;
pub use client::Client;
pub use gateway::GatewayConfig;

/// Convenience constructor for a client bound to the branded service.
pub fn default_client() -> Client {
    Client::new(brand::MANAGED_SERVICE_URL)
}

/// Refresh `credentials` when the access token is close to expiring.
///
/// Returns `true` when a refresh happened and the caller should persist the
/// updated credentials. A failed refresh is reported as an error so the caller
/// can prompt for sign-in again rather than retrying forever.
///
/// Single-flight across the process: the balance poll, the details load, and a
/// user action can all hold stale clones concurrently, and if the service
/// rotates refresh tokens, two racing refreshes would invalidate each other —
/// the loser then overwrites the credential file with a dead token pair and
/// the session is gone on the next restart. Inside the lock the caller's clone
/// is first reconciled with the file, so whoever lost the race adopts the
/// winner's tokens instead of refreshing again.
pub fn refresh_if_needed(credentials: &mut Credentials) -> anyhow::Result<bool> {
    static REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = REFRESH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut changed = false;
    if let Some(on_disk) = Credentials::load()
        && on_disk.endpoint == credentials.endpoint
        && on_disk.expires_at > credentials.expires_at
    {
        *credentials = on_disk;
        changed = true;
    }

    let now = auth::now_unix();
    if !credentials.needs_refresh(now) {
        return Ok(changed);
    }
    let client = Client::new(credentials.endpoint.clone());
    let pair = client.refresh(&credentials.refresh_token)?;
    credentials.apply_refresh(&pair, now);
    credentials.save()?;
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
