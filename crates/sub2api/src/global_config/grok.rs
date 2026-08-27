//! Grok: `~/.grok/config.toml`, edited surgically with `toml_edit`.
//!
//! Grok's native shape (verified against cc-switch's validator and the
//! Electron client's builder): a `[models]` table naming the default profile
//! and one `[model."<id>"]` table per profile carrying `base_url`/`api_key`.
//! We own the two fallback model profiles and the `[models]` defaults;
//! everything else — the user's own profiles, MCP servers, session state
//! keys — is preserved.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use toml_edit::{Array, DocumentMut, Item, Table, value};

use super::{
    CliBackups, RouteTarget, atomic_write_private, capture_backup, remove_if_exists,
    set_toml_value,
};
use crate::gateway::openai_base_url;

const CONFIG_FILE: &str = "config.toml";

/// Model profiles written when no explicit list is given — the CLI's own
/// fallback pair, same as the Electron client used.
const FALLBACK_MODELS: [&str; 2] = ["grok-4.6", "grok-4.5"];

/// Keys we own inside `[models]`.
const MANAGED_MODELS_KEYS: [&str; 2] = ["default", "default_reasoning_effort"];

fn managed_models(target: &RouteTarget) -> Vec<String> {
    if target.models.is_empty() {
        FALLBACK_MODELS.iter().map(|id| (*id).to_owned()).collect()
    } else {
        target.models.clone()
    }
}

pub fn take_over(grok_dir: &Path, target: &RouteTarget, backups: &mut CliBackups) -> Result<()> {
    let path = grok_dir.join(CONFIG_FILE);
    let mut document = read_document(&path)?;
    capture_backup(backups, CONFIG_FILE, &path)?;

    let models = managed_models(target);
    let base_url = openai_base_url(&target.base_url);
    let root = document.as_table_mut();

    // Remove profiles from a previous takeover that are no longer listed,
    // so an edited model list does not leave orphans behind.
    let previous: Vec<String> = root
        .get("model")
        .and_then(Item::as_table)
        .map(|table| table.iter().map(|(key, _)| key.to_owned()).collect())
        .unwrap_or_default();

    let profiles = root.entry("model").or_insert(Item::Table(Table::new()));
    let profiles = profiles
        .as_table_mut()
        .ok_or_else(|| anyhow!("`model` in {} is not a table", path.display()))?;
    profiles.set_implicit(true);
    for id in &models {
        let mut table = Table::new();
        table.insert("model", value(id.as_str()));
        table.insert("name", value(id.as_str()));
        table.insert("base_url", value(base_url.as_str()));
        table.insert("api_key", value(target.api_key.as_str()));
        table.insert("context_window", value(500_000_i64));
        table.insert("api_backend", value("responses"));
        table.insert("supports_reasoning_effort", value(true));
        let mut efforts = Array::new();
        for effort in ["low", "medium", "high"] {
            efforts.push(effort);
        }
        table.insert("reasoning_efforts", value(efforts));
        profiles.insert(id, Item::Table(table));
    }
    for stale in previous {
        // Only prune profiles that carry OUR api key wiring — a profile the
        // user wrote themselves is never removed.
        if models.contains(&stale) {
            continue;
        }
        let ours = profiles
            .get(&stale)
            .and_then(Item::as_table)
            .and_then(|table| table.get("api_backend"))
            .and_then(Item::as_str)
            == Some("responses")
            && backup_lacks_profile(backups, &stale);
        if ours {
            profiles.remove(&stale);
        }
    }

    let defaults = root.entry("models").or_insert(Item::Table(Table::new()));
    let defaults = defaults
        .as_table_mut()
        .ok_or_else(|| anyhow!("`models` in {} is not a table", path.display()))?;
    set_toml_value(defaults, "default", value(models[0].as_str()));
    set_toml_value(defaults, "default_reasoning_effort", value("high"));

    write_document(&path, &document)
}

/// True when the pre-takeover backup did not contain `model.<id>` — i.e. the
/// profile can only have come from us.
fn backup_lacks_profile(backups: &CliBackups, id: &str) -> bool {
    let Some(backup) = backups.get(CONFIG_FILE) else {
        return true;
    };
    if !backup.existed {
        return true;
    }
    let Ok(document) = backup.content.parse::<DocumentMut>() else {
        return false;
    };
    document
        .as_table()
        .get("model")
        .and_then(Item::as_table)
        .is_none_or(|table| !table.contains_key(id))
}

pub fn restore(grok_dir: &Path, backups: &CliBackups) -> Result<()> {
    let Some(backup) = backups.get(CONFIG_FILE) else {
        return Ok(());
    };
    let path = grok_dir.join(CONFIG_FILE);
    let mut document = match read_document(&path) {
        Ok(document) => document,
        Err(_) if !path.exists() => {
            if backup.existed {
                return atomic_write_private(&path, backup.content.as_bytes());
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

    let root = document.as_table_mut();
    if let Some(profiles) = root.get_mut("model").and_then(Item::as_table_mut) {
        // Every profile that was not in the backup is ours; the ones that
        // were go back to their original definition.
        let ids: Vec<String> = profiles.iter().map(|(key, _)| key.to_owned()).collect();
        for id in ids {
            match backup_table
                .and_then(|table| table.get("model"))
                .and_then(Item::as_table)
                .and_then(|table| table.get(&id))
            {
                Some(original) => {
                    profiles.insert(&id, original.clone());
                }
                None => {
                    profiles.remove(&id);
                }
            }
        }
        if profiles.is_empty() {
            root.remove("model");
        }
    }
    if let Some(defaults) = root.get_mut("models").and_then(Item::as_table_mut) {
        for key in MANAGED_MODELS_KEYS {
            match backup_table
                .and_then(|table| table.get("models"))
                .and_then(Item::as_table)
                .and_then(|table| table.get(key))
            {
                Some(original) => {
                    set_toml_value(defaults, key, original.clone());
                }
                None => {
                    defaults.remove(key);
                }
            }
        }
        if defaults.is_empty() {
            root.remove("models");
        }
    }

    if !backup.existed && document.to_string().trim().is_empty() {
        return remove_if_exists(&path);
    }
    write_document(&path, &document)
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
            "sub2api-grok-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn target() -> RouteTarget {
        RouteTarget {
            base_url: "https://gw.example.org/".to_owned(),
            api_key: "sk-grok".to_owned(),
            models: Vec::new(),
        }
    }

    #[test]
    fn writes_fallback_profiles_and_preserves_user_profiles() {
        let dir = temp_dir("profiles");
        std::fs::write(
            dir.join(CONFIG_FILE),
            "# user note\n[models]\ndefault = \"my-model\"\n\n[model.\"my-model\"]\nmodel = \"my-model\"\nbase_url = \"https://mine.example.org/v1\"\napi_key = \"user-key\"\n",
        )
        .unwrap();
        let mut backups = BTreeMap::new();
        take_over(&dir, &target(), &mut backups).expect("take over");

        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(live.contains("# user note"));
        assert!(live.contains(r#"[model."grok-4.6"]"#));
        assert!(live.contains(r#"[model."grok-4.5"]"#));
        assert!(live.contains(r#"[model."my-model"]"#), "user profile kept: {live}");
        assert!(live.contains(r#"default = "grok-4.6""#));
        assert!(live.contains(r#"base_url = "https://gw.example.org/v1""#));

        restore(&dir, &backups).expect("restore");
        let restored = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(restored.contains("# user note"));
        assert!(restored.contains(r#"default = "my-model""#));
        assert!(restored.contains("user-key"));
        assert!(!restored.contains("grok-4.6"));
        assert!(!restored.contains("sk-grok"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_we_created_disappears_on_restore() {
        let dir = temp_dir("fresh");
        let mut backups = BTreeMap::new();
        take_over(&dir, &target(), &mut backups).expect("take over");
        assert!(dir.join(CONFIG_FILE).exists());
        restore(&dir, &backups).expect("restore");
        assert!(!dir.join(CONFIG_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_model_list_replaces_the_fallback_pair() {
        let dir = temp_dir("custom-models");
        let mut backups = BTreeMap::new();
        let custom = RouteTarget {
            models: vec!["grok-code-fast".to_owned()],
            ..target()
        };
        take_over(&dir, &custom, &mut backups).expect("take over");
        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        // Dash-only ids are valid TOML bare keys, so no quotes.
        assert!(live.contains("[model.grok-code-fast]"));
        assert!(!live.contains("grok-4.6"));
        // Switching back to the fallback pair prunes the stale profile.
        take_over(&dir, &target(), &mut backups).expect("switch");
        let live = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(live.contains("grok-4.6"));
        assert!(!live.contains("grok-code-fast"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
