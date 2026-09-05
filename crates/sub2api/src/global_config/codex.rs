//! Codex: `~/.codex/auth.json` + `~/.codex/config.toml`.
//!
//! `auth.json` is wholly credential, so it is backed up as a file (the
//! user's ChatGPT OAuth cache must survive a sign-out) and replaced with the
//! routed API key. `config.toml` is edited surgically with `toml_edit`: our
//! top-level routing keys and one `[model_providers.cheaprouter]` table,
//! everything else — comments, key order, the user's MCP servers — intact.
//!
//! Codex ≥0.149 no longer lets a custom provider inherit `auth.json`
//! credentials, so the key also rides the provider table as
//! `experimental_bearer_token` (cc-switch's approach). Older Codex uses
//! `requires_openai_auth` + `auth.json`; both paths are covered.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use toml_edit::{DocumentMut, Item, Table, value};

use super::{
    CliBackups, PROVIDER_ID, RouteTarget, atomic_write_private, capture_backup, remove_if_exists,
    set_toml_value,
};
use crate::brand;
use crate::gateway::openai_base_url;

const AUTH_FILE: &str = "auth.json";
const CONFIG_FILE: &str = "config.toml";
/// Codex's cache of the provider's model manifest (`GET /models`). It is
/// keyed by provider, not by key, and a different key can mean a different
/// group with a different model list — so it goes whenever the route changes.
const MODELS_CACHE_FILE: &str = "models_cache.json";

/// Codex model pinned while routed, matching the Electron client's template.
const DEFAULT_MODEL: &str = "gpt-5.4";

/// Top-level keys we own. Restore rolls exactly these back.
const MANAGED_TOP_KEYS: [&str; 9] = [
    "model_provider",
    "model",
    "review_model",
    "model_reasoning_effort",
    "disable_response_storage",
    "network_access",
    "windows_wsl_setup_acknowledged",
    "model_context_window",
    "model_auto_compact_token_limit",
];

pub fn take_over(codex_dir: &Path, target: &RouteTarget, backups: &mut CliBackups) -> Result<()> {
    let config_path = codex_dir.join(CONFIG_FILE);
    let auth_path = codex_dir.join(AUTH_FILE);

    // Parse before touching anything: an unparseable config must abort the
    // whole takeover, not leave auth.json half-switched.
    let mut document = read_document(&config_path)?;
    let route_changed = route_changed(&document, target);
    capture_backup(backups, CONFIG_FILE, &config_path)?;
    capture_backup(backups, AUTH_FILE, &auth_path)?;

    let auth = serde_json::json!({ "OPENAI_API_KEY": target.api_key });
    let mut auth_encoded = serde_json::to_string_pretty(&auth).context("encode auth.json")?;
    auth_encoded.push('\n');
    atomic_write_private(&auth_path, auth_encoded.as_bytes())?;

    let root = document.as_table_mut();
    set_toml_value(root, "model_provider", value(PROVIDER_ID));
    set_toml_value(root, "model", value(DEFAULT_MODEL));
    set_toml_value(root, "review_model", value(DEFAULT_MODEL));
    set_toml_value(root, "model_reasoning_effort", value("xhigh"));
    set_toml_value(root, "disable_response_storage", value(true));
    set_toml_value(root, "network_access", value("enabled"));
    set_toml_value(root, "windows_wsl_setup_acknowledged", value(true));
    set_toml_value(root, "model_context_window", value(1_000_000_i64));
    set_toml_value(root, "model_auto_compact_token_limit", value(900_000_i64));

    let providers = root
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    let providers = providers
        .as_table_mut()
        .ok_or_else(|| anyhow!("`model_providers` in {} is not a table", config_path.display()))?;
    // Implicit: render only [model_providers.cheaprouter], no bare header.
    providers.set_implicit(true);
    let mut table = Table::new();
    table.insert("name", value(brand::DISPLAY_NAME));
    table.insert("base_url", value(openai_base_url(&target.base_url)));
    table.insert("wire_api", value("responses"));
    table.insert("requires_openai_auth", value(true));
    table.insert("experimental_bearer_token", value(target.api_key.as_str()));
    providers.insert(PROVIDER_ID, Item::Table(table));

    write_document(&config_path, &document)?;
    if route_changed {
        forget_models_cache(codex_dir);
    }
    Ok(())
}

/// Whether `target` differs from the route the live config already carries
/// — a first takeover, another gateway, or (the common case) another
/// group's key.
fn route_changed(document: &DocumentMut, target: &RouteTarget) -> bool {
    let Some(table) = document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(PROVIDER_ID))
        .and_then(Item::as_table)
    else {
        return true;
    };
    let current = |key: &str| table.get(key).and_then(Item::as_str);
    current("experimental_bearer_token") != Some(target.api_key.as_str())
        || current("base_url") != Some(openai_base_url(&target.base_url).as_str())
}

/// Drop Codex's model-manifest cache so its next start asks the gateway
/// again with the current key. Best effort: a cache that will not delete
/// only costs a stale list until Codex's own refresh.
fn forget_models_cache(codex_dir: &Path) {
    let _ = remove_if_exists(&codex_dir.join(MODELS_CACHE_FILE));
}

pub fn restore(codex_dir: &Path, backups: &CliBackups) -> Result<()> {
    // Back to the user's own provider: its manifest is not ours to keep.
    forget_models_cache(codex_dir);
    // auth.json: the original bytes, or gone if it never existed.
    if let Some(backup) = backups.get(AUTH_FILE) {
        let auth_path = codex_dir.join(AUTH_FILE);
        if backup.existed {
            atomic_write_private(&auth_path, backup.content.as_bytes())?;
        } else {
            remove_if_exists(&auth_path)?;
        }
    }

    let Some(backup) = backups.get(CONFIG_FILE) else {
        return Ok(());
    };
    let config_path = codex_dir.join(CONFIG_FILE);
    let mut document = match read_document(&config_path) {
        Ok(document) => document,
        Err(_) if !config_path.exists() => {
            if backup.existed {
                return atomic_write_private(&config_path, backup.content.as_bytes());
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let backup_document: Option<DocumentMut> = backup
        .existed
        .then(|| backup.content.parse().ok())
        .flatten();

    let root = document.as_table_mut();
    if let Some(providers) = root.get_mut("model_providers").and_then(Item::as_table_mut) {
        providers.remove(PROVIDER_ID);
        if providers.is_empty() {
            root.remove("model_providers");
        }
    }
    for key in MANAGED_TOP_KEYS {
        match backup_document
            .as_ref()
            .and_then(|backup| backup.as_table().get(key))
        {
            Some(original) => {
                set_toml_value(root, key, original.clone());
            }
            None => {
                root.remove(key);
            }
        }
    }

    if !backup.existed && document.to_string().trim().is_empty() {
        return remove_if_exists(&config_path);
    }
    write_document(&config_path, &document)
}

fn read_document(path: &Path) -> Result<DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(raw) => raw
            .parse()
            .with_context(|| format!("{} is not valid TOML", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn write_document(path: &Path, document: &DocumentMut) -> Result<()> {
    atomic_write_private(path, document.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-codex-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn target(key: &str) -> RouteTarget {
        RouteTarget {
            base_url: "https://gw.example.org".to_owned(),
            api_key: key.to_owned(),
            models: Vec::new(),
        }
    }

    #[test]
    fn config_edit_preserves_comments_and_user_tables() {
        let dir = temp_dir("surgical");
        std::fs::write(
            dir.join(CONFIG_FILE),
            "# keep this comment\nmodel = \"my-own\"\n\n[mcp_servers.files]\ncommand = \"fs\"\n\n[model_providers.mine]\nname = \"Mine\"\nbase_url = \"https://mine.example.org/v1\"\n",
        )
        .unwrap();
        std::fs::write(dir.join(AUTH_FILE), r#"{"tokens":{"access_token":"oauth"}}"#).unwrap();

        let mut backups = BTreeMap::new();
        take_over(&dir, &target("sk-gw"), &mut backups).expect("take over");

        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(live.contains("# keep this comment"));
        assert!(live.contains("[mcp_servers.files]"));
        assert!(live.contains("[model_providers.mine]"), "user table kept: {live}");
        assert!(live.contains(&format!("[model_providers.{PROVIDER_ID}]")));
        assert!(live.contains(r#"base_url = "https://gw.example.org/v1""#));
        assert!(live.contains(r#"experimental_bearer_token = "sk-gw""#));
        assert!(live.contains(&format!(r#"model_provider = "{PROVIDER_ID}""#)));
        let auth: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(AUTH_FILE)).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "sk-gw");

        // A group switch rewrites the key without disturbing the backup.
        take_over(&dir, &target("sk-second"), &mut backups).expect("switch");
        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(live.contains(r#"experimental_bearer_token = "sk-second""#));
        assert!(!live.contains("sk-gw"));

        restore(&dir, &backups).expect("restore");
        let restored = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(restored.contains("# keep this comment"));
        assert!(restored.contains("model = \"my-own\""));
        assert!(restored.contains("[model_providers.mine]"));
        assert!(!restored.contains(PROVIDER_ID));
        assert!(!restored.contains("model_reasoning_effort"));
        assert_eq!(
            std::fs::read_to_string(dir.join(AUTH_FILE)).unwrap(),
            r#"{"tokens":{"access_token":"oauth"}}"#
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_cache_is_dropped_only_when_the_route_changes() {
        let dir = temp_dir("models-cache");
        let cache = dir.join(MODELS_CACHE_FILE);
        let mut backups = BTreeMap::new();

        std::fs::write(&cache, "{}").unwrap();
        take_over(&dir, &target("sk-gw"), &mut backups).expect("take over");
        assert!(!cache.exists(), "first takeover is a route change");

        std::fs::write(&cache, "{}").unwrap();
        take_over(&dir, &target("sk-gw"), &mut backups).expect("same route");
        assert!(cache.exists(), "an unchanged route keeps Codex's cache");

        take_over(&dir, &target("sk-other-group"), &mut backups).expect("switch");
        assert!(!cache.exists(), "a new key is a new group: cache dropped");

        std::fs::write(&cache, "{}").unwrap();
        restore(&dir, &backups).expect("restore");
        assert!(!cache.exists(), "restore drops it too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_created_from_nothing_disappear_on_restore() {
        let dir = temp_dir("fresh");
        let mut backups = BTreeMap::new();
        take_over(&dir, &target("sk"), &mut backups).expect("take over");
        assert!(dir.join(CONFIG_FILE).exists());
        assert!(dir.join(AUTH_FILE).exists());
        restore(&dir, &backups).expect("restore");
        assert!(!dir.join(CONFIG_FILE).exists());
        assert!(!dir.join(AUTH_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unparseable_config_aborts_before_touching_auth() {
        let dir = temp_dir("corrupt");
        std::fs::write(dir.join(CONFIG_FILE), "not = = toml").unwrap();
        std::fs::write(dir.join(AUTH_FILE), r#"{"tokens":{}}"#).unwrap();
        let mut backups = BTreeMap::new();
        assert!(take_over(&dir, &target("sk"), &mut backups).is_err());
        // Neither file was modified.
        assert_eq!(
            std::fs::read_to_string(dir.join(AUTH_FILE)).unwrap(),
            r#"{"tokens":{}}"#
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap(),
            "not = = toml"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
