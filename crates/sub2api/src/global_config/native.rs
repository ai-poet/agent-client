//! The built-in agent: the engine's own `settings.json`, `config` block only.
//!
//! The built-in agent is not a CLI, but its routing is written the same way
//! every CLI's is — into the file the engine itself reads. That keeps one
//! model for the whole feature: routing is always something you can verify by
//! opening a file, and taking over always has a backup to restore from.
//!
//! Doing it in memory at session start instead would have been shorter, but
//! it would mean the built-in agent alone had invisible routing, and it would
//! need the daemon to hold gateway keys — which the fork deliberately avoids
//! (see the crate docs).
//!
//! # What is written
//!
//! Three provider entries, one per wire format the engine can speak against
//! the gateway — `anthropic` (Messages), `openai` (Chat Completions) and
//! `codex` (Responses) — all pointed at the gateway origin, plus every gateway
//! key the account has, filed by the platform it authorizes under
//! `provider_configs.anthropic.options.gateway_keys`. A session picks the key
//! for its model's platform and the entry for its wire format at start; the
//! file itself names no default provider, so the engine's own precedence
//! (`config.provider`, then a `provider/` prefix, then Anthropic) is
//! undisturbed for anyone running the engine outside Waku.
//!
//! Only the keys below are touched. The engine's own model pin, MCP roster,
//! permission rules, hooks and agent definitions live in the same file and are
//! left exactly as the user left them.
//!
//! A partial `config` block is only legal because the vendored engine's
//! `Config` carries a container-level `#[serde(default)]` — a recorded fork
//! departure in `crates/waku-agent/core/src/lib.rs`. A future engine re-sync
//! that drops it would make every file this writer creates unreadable to the
//! engine again; `waku-agent-bridge`'s `a_fresh_takeover_file_parses_and_routes_every_format`
//! is the test that would catch it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

use super::{
    CliBackups, FileBackup, NativeRoutes, atomic_write_private, capture_backup,
    remove_if_exists,
};
use crate::gateway::anthropic_base_url;

const SETTINGS_FILE: &str = "settings.json";

/// The engine provider entries this writer owns, one per wire format.
pub const MANAGED_PROVIDERS: [&str; 3] = ["anthropic", "openai", "codex"];

/// Where the per-platform keys live inside the anthropic entry's free-form
/// options. The bridge reads the same path.
pub const GATEWAY_KEYS_OPTION: &str = "gateway_keys";

/// The member of that table holding the per-model keys. Nested rather than
/// an option of its own so it is written, cleared and restored with the
/// platform keys as one unit — the invariant that neither table survives
/// next to somebody else's endpoint then holds for both without a second
/// code path. No platform is called `models`.
pub const MODEL_KEYS_MEMBER: &str = "models";

/// The member holding the pay-as-you-go keys, per model — what the picker's
/// "pay as you go" row goes out with. Same unit, same reason. No platform is
/// called `payg_models` either.
pub const PAYG_MODEL_KEYS_MEMBER: &str = "payg_models";

/// The table as written: platform keys, plus the per-model ones when there
/// are any.
fn gateway_keys_value(routes: &NativeRoutes) -> Value {
    let mut table: Map<String, Value> = routes
        .platform_keys
        .iter()
        .map(|(platform, key)| (platform.clone(), json!(key)))
        .collect();
    if !routes.model_keys.is_empty() {
        table.insert(MODEL_KEYS_MEMBER.to_owned(), json!(routes.model_keys));
    }
    if !routes.payg_model_keys.is_empty() {
        table.insert(
            PAYG_MODEL_KEYS_MEMBER.to_owned(),
            json!(routes.payg_model_keys),
        );
    }
    Value::Object(table)
}

/// Where the engine looks for its global settings.
///
/// Mirrors the engine's own resolver exactly — an explicit `CLAURST_HOME`, then
/// an existing legacy `~/.claurst`, then the XDG location. Guessing wrong here
/// would write routing into a file nothing reads, which is the one failure
/// mode this feature cannot afford: it looks like it worked.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CLAURST_HOME")
        && !explicit.is_empty()
    {
        return Some(PathBuf::from(explicit));
    }
    let home = dirs::home_dir()?;
    let legacy = home.join(".claurst");
    if legacy.is_dir() {
        return Some(legacy);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let xdg = PathBuf::from(xdg);
        // Per the XDG spec a relative $XDG_CONFIG_HOME is ignored.
        if xdg.is_absolute() {
            return Some(xdg.join("claurst"));
        }
    }
    Some(home.join(".config").join("claurst"))
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE)
}

pub fn take_over(
    config_dir: &Path,
    routes: &NativeRoutes,
    backups: &mut CliBackups,
) -> Result<()> {
    let path = settings_path(config_dir);
    capture_backup(backups, SETTINGS_FILE, &path)?;
    let previous = previous_config(backups.get(SETTINGS_FILE));

    let mut root = read_settings(&path)?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", path.display()))?;
    let config = object
        .entry("config")
        .or_insert_with(|| Value::Object(Map::new()));
    let config = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("`config` in {} is not an object", path.display()))?;

    // The engine resolves the top-level key first, for whichever provider
    // the session selected — so any value here outranks all three provider
    // entries and collapses three routes into one. Earlier builds wrote our
    // own key here precisely to outrank one the user had typed in; removing
    // it achieves that without also outranking ourselves. `restore` reads
    // the backup, so the user's value comes back when routing is released.
    config.remove("api_key");

    let previous_providers = previous
        .get("provider_configs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let providers = config
        .entry("provider_configs")
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`provider_configs` in {} is not an object", path.display()))?;
    for provider in MANAGED_PROVIDERS {
        // A route the user cleared goes back to what it was before we ever
        // touched it. Releasing the whole file only happens when all three
        // are gone, so a single line retiring has to be handled here.
        let Some(target) = routes.target(provider) else {
            restore_provider_entry(providers, provider, &previous_providers);
            continue;
        };
        let entry = providers
            .entry(provider)
            .or_insert_with(|| Value::Object(Map::new()));
        let entry = entry.as_object_mut().ok_or_else(|| {
            anyhow!("the {provider} provider entry in {} is not an object", path.display())
        })?;
        entry.insert("api_key".to_owned(), json!(target.api_key));
        // The adapters append their own paths (`/v1/messages`,
        // `/v1/chat/completions`, `/v1/responses`), so every entry gets the
        // bare origin.
        entry.insert(
            "api_base".to_owned(),
            json!(anthropic_base_url(&target.base_url)),
        );
        entry.insert("enabled".to_owned(), json!(true));
    }

    // The per-platform key table rides on the anthropic entry. It is empty
    // whenever any route is the user's own — it is consulted by platform,
    // and a leftover table from a previous all-gateway state would hand a
    // gateway key to a request aimed at somebody else's server.
    let anthropic_exists = providers.contains_key("anthropic");
    if !routes.platform_keys.is_empty() || anthropic_exists {
        let entry = providers
            .entry("anthropic")
            .or_insert_with(|| Value::Object(Map::new()));
        let entry = entry.as_object_mut().ok_or_else(|| {
            anyhow!("the anthropic provider entry in {} is not an object", path.display())
        })?;
        if routes.platform_keys.is_empty() {
            let previous_options = previous_providers
                .get("anthropic")
                .and_then(Value::as_object)
                .and_then(|entry| entry.get("options"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(options) = entry.get_mut("options").and_then(Value::as_object_mut) {
                restore_key(options, GATEWAY_KEYS_OPTION, &previous_options);
                if options.is_empty() {
                    entry.remove("options");
                }
            }
        } else {
            let options = entry
                .entry("options")
                .or_insert_with(|| Value::Object(Map::new()));
            let options = options.as_object_mut().ok_or_else(|| {
                anyhow!("the anthropic provider options in {} are not an object", path.display())
            })?;
            options.insert(GATEWAY_KEYS_OPTION.to_owned(), gateway_keys_value(routes));
        }
        if entry.is_empty() {
            providers.remove("anthropic");
        }
    }
    if providers.is_empty() {
        config.remove("provider_configs");
    }

    write_settings(&path, &root)
}

/// The `config` block as it stood before we took over, from the backup.
fn previous_config(backup: Option<&FileBackup>) -> Map<String, Value> {
    backup
        .filter(|backup| backup.existed)
        .and_then(|backup| serde_json::from_str::<Value>(&backup.content).ok())
        .and_then(|value| value.get("config").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// Put one provider entry back the way the backup had it, removing it
/// outright when the backup had nothing there.
fn restore_provider_entry(
    providers: &mut Map<String, Value>,
    provider: &str,
    previous_providers: &Map<String, Value>,
) {
    let previous_entry = previous_providers
        .get(provider)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let Some(entry) = providers.get_mut(provider).and_then(Value::as_object_mut) else {
        return;
    };
    for key in ["api_key", "api_base", "enabled"] {
        restore_key(entry, key, &previous_entry);
    }
    let previous_options = previous_entry
        .get("options")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(options) = entry.get_mut("options").and_then(Value::as_object_mut) {
        restore_key(options, GATEWAY_KEYS_OPTION, &previous_options);
        if options.is_empty() {
            entry.remove("options");
        }
    }
    if entry.is_empty() {
        providers.remove(provider);
    }
}

/// Put the managed keys back to what the backup recorded, leaving everything
/// the user changed while managed intact. Named `restore` to match the other
/// switching-mode writers.
pub fn restore(config_dir: &Path, backups: &CliBackups) -> Result<()> {
    let Some(backup) = backups.get(SETTINGS_FILE) else {
        return Ok(());
    };
    let path = settings_path(config_dir);
    let mut root = match read_settings(&path) {
        Ok(root) => root,
        // The file vanished while managed; put the original back wholesale.
        Err(_) if !path.exists() => {
            if backup.existed {
                return atomic_write_private(&path, backup.content.as_bytes());
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let Some(object) = root.as_object_mut() else {
        return Err(anyhow!("{} is not a JSON object", path.display()));
    };

    // What our keys held before we took over, if the file existed at all.
    let previous = previous_config(Some(backup));

    if let Some(config) = object.get_mut("config").and_then(Value::as_object_mut) {
        restore_key(config, "api_key", &previous);
        // Older builds of this writer pinned the provider; clear that too.
        restore_key(config, "provider", &previous);

        let previous_providers = previous
            .get("provider_configs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(providers) = config
            .get_mut("provider_configs")
            .and_then(Value::as_object_mut)
        {
            for provider in MANAGED_PROVIDERS {
                restore_provider_entry(providers, provider, &previous_providers);
            }
            if providers.is_empty() {
                config.remove("provider_configs");
            }
        }
        if config.is_empty() {
            object.remove("config");
        }
    }

    // A file that only ever existed because of us disappears again.
    if !backup.existed && object.is_empty() {
        return remove_if_exists(&path);
    }
    write_settings(&path, &root)
}

/// Restore one key from the backup, or remove it when the backup did not have
/// it. Removing rather than blanking matters: an empty `api_base` is not the
/// same as no override.
fn restore_key(object: &mut Map<String, Value>, key: &str, previous: &Map<String, Value>) {
    match previous.get(key) {
        Some(value) => {
            object.insert(key.to_owned(), value.clone());
        }
        None => {
            object.remove(key);
        }
    }
}

fn read_settings(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(&raw).with_context(|| format!("could not parse {}", path.display()))
}

fn write_settings(path: &Path, root: &Value) -> Result<()> {
    let mut rendered = serde_json::to_string_pretty(root)?;
    rendered.push('\n');
    atomic_write_private(path, rendered.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::global_config::RouteTarget;

    fn at(base: &str, key: &str) -> Option<RouteTarget> {
        Some(RouteTarget {
            base_url: base.into(),
            api_key: key.into(),
            models: Vec::new(),
            managed_gateway: false,
        })
    }

    /// Every route on the managed gateway, which is what signing in and
    /// touching nothing else produces.
    fn routes() -> NativeRoutes {
        NativeRoutes {
            messages: at("https://gateway.example.org", "sk-claude"),
            responses: at("https://gateway.example.org", "sk-claude"),
            chat: at("https://gateway.example.org", "sk-claude"),
            platform_keys: keys(),
            model_keys: BTreeMap::new(),
            payg_model_keys: BTreeMap::new(),
        }
    }

    fn keys() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("anthropic".to_owned(), "sk-claude".to_owned()),
            ("openai".to_owned(), "sk-codex".to_owned()),
            ("default".to_owned(), "sk-general".to_owned()),
        ])
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn taking_over_writes_one_entry_per_wire_format_and_every_key() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();

        let root = read(&settings_path(&dir));
        let config = root.get("config").unwrap();
        // No top-level key: the engine resolves that one first, for whichever
        // provider the session picked, so a value here would outrank all
        // three entries and collapse the three routes into one.
        assert!(config.get("api_key").is_none());
        // No provider pin either: the session chooses.
        assert!(config.get("provider").is_none());
        for provider in MANAGED_PROVIDERS {
            let entry = config
                .pointer(&format!("/provider_configs/{provider}"))
                .unwrap_or_else(|| panic!("{provider} entry"));
            assert_eq!(entry.get("api_base").unwrap(), "https://gateway.example.org");
            assert_eq!(entry.get("enabled").unwrap(), &json!(true));
        }
        assert_eq!(
            config
                .pointer("/provider_configs/anthropic/options/gateway_keys/openai")
                .unwrap(),
            "sk-codex"
        );
    }

    #[test]
    fn the_users_own_settings_survive_a_takeover() {
        let dir = tempdir();
        std::fs::write(
            settings_path(&dir),
            r#"{"config":{"model":"claude-opus-5","auto_compact":true,"provider_configs":{"google":{"api_key":"g"}}},"permissionRules":[{"keep":1}]}"#,
        )
        .unwrap();

        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();

        let root = read(&settings_path(&dir));
        assert_eq!(root.pointer("/config/model").unwrap(), "claude-opus-5");
        assert_eq!(root.pointer("/config/auto_compact").unwrap(), &json!(true));
        assert_eq!(root.pointer("/config/provider_configs/google/api_key").unwrap(), "g");
        assert!(root.get("permissionRules").is_some());
    }

    #[test]
    fn releasing_restores_a_key_the_user_had_before() {
        let dir = tempdir();
        std::fs::write(
            settings_path(&dir),
            r#"{"config":{"api_key":"sk-user-own","model":"claude-opus-5","provider":"openai"}}"#,
        )
        .unwrap();

        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();
        // Out of the way while managed — left in place it would outrank the
        // provider entries and every route would use it.
        assert!(
            read(&settings_path(&dir))
                .pointer("/config/api_key")
                .is_none()
        );

        restore(&dir, &backups).unwrap();
        let root = read(&settings_path(&dir));
        assert_eq!(root.pointer("/config/api_key").unwrap(), "sk-user-own");
        assert_eq!(root.pointer("/config/provider").unwrap(), "openai");
        assert_eq!(root.pointer("/config/model").unwrap(), "claude-opus-5");
        assert!(root.pointer("/config/provider_configs").is_none());
    }

    /// The document the bridge's tests load as engine settings. Kept in
    /// step with `waku-agent-bridge/src/config.rs::FRESH_TAKEOVER_SETTINGS`;
    /// the bridge cannot depend on this crate, so the contract is a literal
    /// on each side and this assertion.
    #[test]
    fn a_fresh_takeover_matches_the_bridges_fixture() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();
        let written = read(&settings_path(&dir));
        let expected: Value = serde_json::from_str(
            r#"{"config":{"provider_configs":{
              "anthropic":{"api_key":"sk-claude","api_base":"https://gateway.example.org","enabled":true,
                "options":{"gateway_keys":{"anthropic":"sk-claude","default":"sk-general","openai":"sk-codex"}}},
              "codex":{"api_key":"sk-claude","api_base":"https://gateway.example.org","enabled":true},
              "openai":{"api_key":"sk-claude","api_base":"https://gateway.example.org","enabled":true}}}}"#,
        )
        .unwrap();
        assert_eq!(written, expected);
    }

    /// The point of three slots: three addresses and three keys that do not
    /// bleed into one another.
    #[test]
    fn three_routes_write_three_keys_and_three_bases() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        let routes = NativeRoutes {
            messages: at("https://anthropic.example.org", "sk-messages"),
            responses: at("https://responses.example.org", "sk-responses"),
            chat: at("https://chat.example.org", "sk-chat"),
            platform_keys: BTreeMap::new(),
            model_keys: BTreeMap::new(),
            payg_model_keys: BTreeMap::new(),
        };
        take_over(&dir, &routes, &mut backups).unwrap();

        let root = read(&settings_path(&dir));
        for (provider, base, key) in [
            ("anthropic", "https://anthropic.example.org", "sk-messages"),
            ("codex", "https://responses.example.org", "sk-responses"),
            ("openai", "https://chat.example.org", "sk-chat"),
        ] {
            let entry = root
                .pointer(&format!("/config/provider_configs/{provider}"))
                .unwrap_or_else(|| panic!("{provider} entry"));
            assert_eq!(entry.get("api_base").unwrap(), base, "{provider}");
            assert_eq!(entry.get("api_key").unwrap(), key, "{provider}");
        }
        assert!(root.pointer("/config/api_key").is_none());
        // No per-platform table beside somebody else's endpoint: it is read
        // by platform and would hand that server a gateway key.
        assert!(
            root.pointer("/config/provider_configs/anthropic/options")
                .is_none()
        );
    }

    /// The per-model keys ride inside the platform table, so the bridge finds
    /// both in one place and one release clears both.
    #[test]
    fn model_keys_are_written_beside_the_platform_keys_and_leave_with_them() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        let routes = NativeRoutes {
            model_keys: BTreeMap::from([("deepseek-v4.1-flash".to_owned(), "sk-sub".to_owned())]),
            payg_model_keys: BTreeMap::from([(
                "deepseek-v4.1-flash".to_owned(),
                "sk-payg".to_owned(),
            )]),
            ..routes()
        };
        take_over(&dir, &routes, &mut backups).unwrap();
        let table = read(&settings_path(&dir))
            .pointer("/config/provider_configs/anthropic/options/gateway_keys")
            .cloned()
            .expect("key table");
        assert_eq!(table.pointer("/models/deepseek-v4.1-flash").unwrap(), "sk-sub");
        assert_eq!(
            table.pointer("/payg_models/deepseek-v4.1-flash").unwrap(),
            "sk-payg"
        );
        assert_eq!(table.get("openai").unwrap(), "sk-codex");

        let custom = NativeRoutes {
            messages: at("https://mine.example.org", "sk-mine"),
            ..NativeRoutes::default()
        };
        take_over(&dir, &custom, &mut backups).unwrap();
        assert!(
            read(&settings_path(&dir))
                .pointer("/config/provider_configs/anthropic/options/gateway_keys")
                .is_none()
        );
    }

    /// Switching from the managed gateway to an endpoint of your own has to
    /// drop the key table the gateway left behind, or every session keeps
    /// authenticating with the gateway's key against your server.
    #[test]
    fn a_stale_gateway_key_table_is_dropped_when_a_route_goes_custom() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();
        assert!(
            read(&settings_path(&dir))
                .pointer("/config/provider_configs/anthropic/options/gateway_keys")
                .is_some()
        );

        let custom = NativeRoutes {
            messages: at("https://mine.example.org", "sk-mine"),
            responses: None,
            chat: None,
            platform_keys: BTreeMap::new(),
            model_keys: BTreeMap::new(),
            payg_model_keys: BTreeMap::new(),
        };
        take_over(&dir, &custom, &mut backups).unwrap();
        let root = read(&settings_path(&dir));
        assert!(
            root.pointer("/config/provider_configs/anthropic/options")
                .is_none()
        );
        assert_eq!(
            root.pointer("/config/provider_configs/anthropic/api_key")
                .unwrap(),
            "sk-mine"
        );
    }

    /// Clearing one route retires that entry alone. Releasing the whole file
    /// only happens when all three are gone, so this has to work on its own.
    #[test]
    fn clearing_one_route_reverts_only_its_entry() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();

        let without_chat = NativeRoutes {
            chat: None,
            ..routes()
        };
        take_over(&dir, &without_chat, &mut backups).unwrap();
        let root = read(&settings_path(&dir));
        // The backup had nothing there, so the entry goes away entirely.
        assert!(root.pointer("/config/provider_configs/openai").is_none());
        // The other two are untouched.
        assert_eq!(
            root.pointer("/config/provider_configs/codex/api_key").unwrap(),
            "sk-claude"
        );
        assert_eq!(
            root.pointer("/config/provider_configs/anthropic/api_key")
                .unwrap(),
            "sk-claude"
        );
    }

    /// A file written by a build that still pinned the top-level key heals
    /// on the next reconcile rather than staying hijacked.
    #[test]
    fn an_upgrade_removes_the_top_level_key_an_earlier_build_wrote() {
        let dir = tempdir();
        std::fs::write(
            settings_path(&dir),
            r#"{"config":{"api_key":"sk-old-managed","provider_configs":{"anthropic":{"api_key":"sk-old-managed"}}}}"#,
        )
        .unwrap();
        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();
        assert!(
            read(&settings_path(&dir))
                .pointer("/config/api_key")
                .is_none()
        );
    }

    #[test]
    fn releasing_a_file_we_created_deletes_it_again() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &routes(), &mut backups).unwrap();
        restore(&dir, &backups).unwrap();
        assert!(!settings_path(&dir).exists());
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "waku-native-routing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
