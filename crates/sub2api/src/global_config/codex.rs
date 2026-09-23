//! Codex: `~/.codex/config.toml`, edited surgically with `toml_edit`.
//!
//! Our top-level routing keys, one `[model_providers.OpenAI]` table and two
//! `[features]` switches — everything else, comments, key order and the
//! user's MCP servers, intact. The shape is the one the service documents
//! for Codex, so a hand-configured machine and a routed one look alike.
//!
//! The key rides the provider table as `experimental_bearer_token` with
//! `requires_openai_auth = false`, so `auth.json` is not ours: the user's
//! ChatGPT sign-in stays where it is. Earlier builds replaced that file with
//! the routed key; the first takeover or release after upgrading puts the
//! user's original back (see [`release_auth_json`]).

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use toml_edit::{DocumentMut, Item, Table, value};

use super::{
    CliBackups, PROVIDER_ID, RouteTarget, atomic_write_private, capture_backup, remove_if_exists,
    set_toml_value,
};
use crate::gateway::{anthropic_base_url, openai_base_url};

const AUTH_FILE: &str = "auth.json";
const CONFIG_FILE: &str = "config.toml";
/// Codex's cache of the provider's model manifest (`GET /models`). It is
/// keyed by provider, not by key, and a different key can mean a different
/// group with a different model list — so it goes whenever the route changes.
const MODELS_CACHE_FILE: &str = "models_cache.json";

/// The provider entry we own, and its display name.
const CODEX_PROVIDER: &str = "OpenAI";

/// Where earlier builds filed the entry. Ours by construction, so it is
/// removed wherever it is found.
const LEGACY_PROVIDER: &str = PROVIDER_ID;

/// The models pinned while routed.
const DEFAULT_MODEL: &str = "gpt-5.6-sol";
const REVIEW_MODEL: &str = "gpt-5.6-terra";

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

/// `[features]` switches we own. The table itself is the user's too.
const MANAGED_FEATURES: [&str; 2] = ["image_generation", "remote_compaction_v2"];

/// Sent with every request: the gateway lets Codex's built-in image tool
/// through on it.
const IMAGE_EXTENSION_HEADER: (&str, &str) =
    ("x-openai-actor-authorization", "local-image-extension");

/// Where the provider table points. The managed gateway also answers the
/// OpenAI paths at its root (`/responses`, `/models`), which is how the
/// service documents it; anybody else's endpoint gets the versioned path it
/// has always had.
fn provider_base_url(target: &RouteTarget) -> String {
    if target.managed_gateway {
        anthropic_base_url(&target.base_url)
    } else {
        openai_base_url(&target.base_url)
    }
}

pub fn take_over(codex_dir: &Path, target: &RouteTarget, backups: &mut CliBackups) -> Result<()> {
    let config_path = codex_dir.join(CONFIG_FILE);

    // Parse before touching anything: an unparseable config must abort the
    // whole takeover, not leave auth.json half-restored.
    let mut document = read_document(&config_path)?;
    let route_changed = route_changed(&document, target);
    capture_backup(backups, CONFIG_FILE, &config_path)?;
    release_auth_json(codex_dir, backups)?;

    let root = document.as_table_mut();
    set_toml_value(root, "model_provider", value(CODEX_PROVIDER));
    set_toml_value(root, "model", value(DEFAULT_MODEL));
    set_toml_value(root, "review_model", value(REVIEW_MODEL));
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
    // Implicit: render only [model_providers.OpenAI], no bare header.
    providers.set_implicit(true);
    providers.remove(LEGACY_PROVIDER);
    let mut headers = Table::new();
    headers.insert(IMAGE_EXTENSION_HEADER.0, value(IMAGE_EXTENSION_HEADER.1));
    let mut table = Table::new();
    table.insert("name", value(CODEX_PROVIDER));
    table.insert("base_url", value(provider_base_url(target)));
    table.insert("wire_api", value("responses"));
    table.insert("requires_openai_auth", value(false));
    table.insert("experimental_bearer_token", value(target.api_key.as_str()));
    table.insert("http_headers", Item::Table(headers));
    providers.insert(CODEX_PROVIDER, Item::Table(table));

    let features = root.entry("features").or_insert(Item::Table(Table::new()));
    let features = features
        .as_table_mut()
        .ok_or_else(|| anyhow!("`features` in {} is not a table", config_path.display()))?;
    for feature in MANAGED_FEATURES {
        set_toml_value(features, feature, value(true));
    }

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
        .and_then(|providers| providers.get(CODEX_PROVIDER))
        .and_then(Item::as_table)
    else {
        return true;
    };
    let current = |key: &str| table.get(key).and_then(Item::as_str);
    current("experimental_bearer_token") != Some(target.api_key.as_str())
        || current("base_url") != Some(provider_base_url(target).as_str())
}

/// Drop Codex's model-manifest cache so its next start asks the gateway
/// again with the current key. Best effort: a cache that will not delete
/// only costs a stale list until Codex's own refresh.
fn forget_models_cache(codex_dir: &Path) {
    let _ = remove_if_exists(&codex_dir.join(MODELS_CACHE_FILE));
}

/// Give `auth.json` back to the user, once, if an earlier build took it.
///
/// Those builds backed the file up and replaced it with `{"OPENAI_API_KEY":
/// <routed key>}`. The original goes back only while the file is still
/// exactly that shape — one the user signed into ChatGPT over since is
/// theirs and stays — and the backup is dropped either way, so nothing
/// restores it later over something newer.
fn release_auth_json(codex_dir: &Path, backups: &mut CliBackups) -> Result<()> {
    let Some(backup) = backups.remove(AUTH_FILE) else {
        return Ok(());
    };
    let auth_path = codex_dir.join(AUTH_FILE);
    let ours = std::fs::read_to_string(&auth_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| object.len() == 1 && object.contains_key("OPENAI_API_KEY"));
    if !ours {
        return Ok(());
    }
    if backup.existed {
        atomic_write_private(&auth_path, backup.content.as_bytes())
    } else {
        remove_if_exists(&auth_path)
    }
}

pub fn restore(codex_dir: &Path, backups: &CliBackups) -> Result<()> {
    // Back to the user's own provider: its manifest is not ours to keep.
    forget_models_cache(codex_dir);
    let mut backups = backups.clone();
    release_auth_json(codex_dir, &mut backups)?;

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
    let backup_table = backup_document.as_ref().map(DocumentMut::as_table);
    let original = |path: &[&str]| -> Option<Item> {
        let mut item = backup_table?.get(path[0])?;
        for key in &path[1..] {
            item = item.as_table()?.get(key)?;
        }
        Some(item.clone())
    };

    let root = document.as_table_mut();
    if let Some(providers) = root.get_mut("model_providers").and_then(Item::as_table_mut) {
        providers.remove(LEGACY_PROVIDER);
        // A table of that name the user had before is theirs again; one
        // they did not have was ours.
        match original(&["model_providers", CODEX_PROVIDER]) {
            Some(table) => {
                providers.insert(CODEX_PROVIDER, table);
            }
            None => {
                providers.remove(CODEX_PROVIDER);
            }
        }
        if providers.is_empty() {
            root.remove("model_providers");
        }
    }
    if let Some(features) = root.get_mut("features").and_then(Item::as_table_mut) {
        for feature in MANAGED_FEATURES {
            match original(&["features", feature]) {
                Some(value) => {
                    set_toml_value(features, feature, value);
                }
                None => {
                    features.remove(feature);
                }
            }
        }
        if features.is_empty() && original(&["features"]).is_none() {
            root.remove("features");
        }
    }
    for key in MANAGED_TOP_KEYS {
        match original(&[key]) {
            Some(value) => {
                set_toml_value(root, key, value);
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
            managed_gateway: false,
        }
    }

    fn gateway(key: &str) -> RouteTarget {
        RouteTarget {
            managed_gateway: true,
            ..target(key)
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
        assert!(live.contains("[model_providers.OpenAI]"));
        // Somebody else's endpoint keeps the versioned path.
        assert!(live.contains(r#"base_url = "https://gw.example.org/v1""#));
        assert!(live.contains(r#"experimental_bearer_token = "sk-gw""#));
        assert!(live.contains(r#"model_provider = "OpenAI""#));
        // auth.json is the user's: the ChatGPT sign-in is left alone.
        assert_eq!(
            std::fs::read_to_string(dir.join(AUTH_FILE)).unwrap(),
            r#"{"tokens":{"access_token":"oauth"}}"#
        );

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
        assert!(!restored.contains("[model_providers.OpenAI]"));
        assert!(!restored.contains("[features]"));
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
        assert!(!dir.join(AUTH_FILE).exists(), "auth.json is never written");
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

    /// The shape the service documents for Codex, on the managed gateway:
    /// the bare origin (it answers `/responses` and `/models` at its root),
    /// the key as a bearer token with no `auth.json` involved, the image
    /// extension's header, and the two feature switches.
    #[test]
    fn the_gateway_gets_the_documented_shape() {
        let dir = temp_dir("documented");
        std::fs::write(
            dir.join(CONFIG_FILE),
            "[features]\nweb_search = true\n",
        )
        .unwrap();
        let mut backups = BTreeMap::new();
        take_over(&dir, &gateway("sk-gw"), &mut backups).expect("take over");

        let document: DocumentMut = std::fs::read_to_string(dir.join(CONFIG_FILE))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(document["model_provider"].as_str(), Some("OpenAI"));
        assert_eq!(document["model"].as_str(), Some("gpt-5.6-sol"));
        assert_eq!(document["review_model"].as_str(), Some("gpt-5.6-terra"));
        assert_eq!(document["model_reasoning_effort"].as_str(), Some("xhigh"));
        assert_eq!(document["model_context_window"].as_integer(), Some(1_000_000));
        assert_eq!(document["model_auto_compact_token_limit"].as_integer(), Some(900_000));
        let provider = &document["model_providers"]["OpenAI"];
        assert_eq!(provider["name"].as_str(), Some("OpenAI"));
        assert_eq!(provider["base_url"].as_str(), Some("https://gw.example.org"));
        assert_eq!(provider["wire_api"].as_str(), Some("responses"));
        assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
        assert_eq!(provider["experimental_bearer_token"].as_str(), Some("sk-gw"));
        assert_eq!(
            provider["http_headers"]["x-openai-actor-authorization"].as_str(),
            Some("local-image-extension")
        );
        assert_eq!(document["features"]["image_generation"].as_bool(), Some(true));
        assert_eq!(document["features"]["remote_compaction_v2"].as_bool(), Some(true));
        assert_eq!(document["features"]["web_search"].as_bool(), Some(true));
        assert!(!dir.join(AUTH_FILE).exists());

        // Released: the user's own switch stays, ours go.
        restore(&dir, &backups).expect("restore");
        let restored = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(restored.contains("web_search = true"));
        assert!(!restored.contains("image_generation"));
        assert!(!restored.contains("remote_compaction_v2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A machine configured by hand the documented way already has an
    /// `OpenAI` table; taking over replaces it, releasing puts it back.
    #[test]
    fn a_hand_written_openai_table_comes_back_on_release() {
        let dir = temp_dir("hand-written");
        let original = "model_provider = \"OpenAI\"\n\n[model_providers.OpenAI]\nname = \"OpenAI\"\nbase_url = \"https://mine.example.org\"\nexperimental_bearer_token = \"sk-mine\"\n\n[features]\nimage_generation = false\n";
        std::fs::write(dir.join(CONFIG_FILE), original).unwrap();
        let mut backups = BTreeMap::new();
        take_over(&dir, &gateway("sk-gw"), &mut backups).expect("take over");
        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(live.contains(r#"experimental_bearer_token = "sk-gw""#));
        assert!(!live.contains("sk-mine"));

        restore(&dir, &backups).expect("restore");
        let document: DocumentMut = std::fs::read_to_string(dir.join(CONFIG_FILE))
            .unwrap()
            .parse()
            .unwrap();
        let provider = &document["model_providers"]["OpenAI"];
        assert_eq!(provider["experimental_bearer_token"].as_str(), Some("sk-mine"));
        assert_eq!(provider["base_url"].as_str(), Some("https://mine.example.org"));
        assert_eq!(document["features"]["image_generation"].as_bool(), Some(false));
        assert!(document["features"].get("remote_compaction_v2").is_none());
        assert_eq!(document["model_provider"].as_str(), Some("OpenAI"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An earlier build replaced `auth.json` with the routed key and filed
    /// its provider under another name. The next takeover gives the file
    /// back and drops the old table; a file the user signed in over since is
    /// theirs and stays.
    #[test]
    fn an_earlier_builds_auth_json_and_table_are_given_back() {
        let dir = temp_dir("legacy");
        std::fs::write(
            dir.join(CONFIG_FILE),
            format!("model_provider = \"{PROVIDER_ID}\"\n\n[model_providers.{PROVIDER_ID}]\nname = \"old\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join(AUTH_FILE), r#"{"OPENAI_API_KEY":"sk-old"}"#).unwrap();
        let mut backups = BTreeMap::from([(
            AUTH_FILE.to_owned(),
            super::super::FileBackup {
                existed: true,
                content: r#"{"tokens":{"access_token":"oauth"}}"#.to_owned(),
            },
        )]);
        take_over(&dir, &gateway("sk-gw"), &mut backups).expect("take over");
        assert_eq!(
            std::fs::read_to_string(dir.join(AUTH_FILE)).unwrap(),
            r#"{"tokens":{"access_token":"oauth"}}"#
        );
        assert!(!backups.contains_key(AUTH_FILE));
        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(!live.contains(&format!("[model_providers.{PROVIDER_ID}]")), "{live}");

        // Signed in over since: the backup is dropped, the file kept.
        let dir2 = temp_dir("legacy-signed-in");
        std::fs::write(dir2.join(AUTH_FILE), r#"{"tokens":{"access_token":"newer"}}"#).unwrap();
        let mut backups = BTreeMap::from([(
            AUTH_FILE.to_owned(),
            super::super::FileBackup {
                existed: true,
                content: r#"{"tokens":{"access_token":"older"}}"#.to_owned(),
            },
        )]);
        take_over(&dir2, &gateway("sk-gw"), &mut backups).expect("take over");
        assert_eq!(
            std::fs::read_to_string(dir2.join(AUTH_FILE)).unwrap(),
            r#"{"tokens":{"access_token":"newer"}}"#
        );
        assert!(!backups.contains_key(AUTH_FILE));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }
}
