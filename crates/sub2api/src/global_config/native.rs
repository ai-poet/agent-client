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
//! Only the keys below are touched. The engine's own model pin, MCP roster,
//! permission rules, hooks and agent definitions live in the same file and are
//! left exactly as the user left them.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

use super::{CliBackups, RouteTarget, atomic_write_private, capture_backup, remove_if_exists};
use crate::gateway::anthropic_base_url;

const SETTINGS_FILE: &str = "settings.json";

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

pub fn take_over(config_dir: &Path, target: &RouteTarget, backups: &mut CliBackups) -> Result<()> {
    let path = settings_path(config_dir);
    capture_backup(backups, SETTINGS_FILE, &path)?;

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

    // The engine resolves a key from the top-level field first, so the
    // provider entry alone would be outranked by a key the user typed into
    // the engine's own settings earlier.
    config.insert("api_key".to_owned(), json!(target.api_key));
    config.insert("provider".to_owned(), json!("anthropic"));

    let providers = config
        .entry("provider_configs")
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`provider_configs` in {} is not an object", path.display()))?;
    let anthropic = providers
        .entry("anthropic")
        .or_insert_with(|| Value::Object(Map::new()));
    let anthropic = anthropic
        .as_object_mut()
        .ok_or_else(|| anyhow!("the anthropic provider entry in {} is not an object", path.display()))?;
    anthropic.insert("api_key".to_owned(), json!(target.api_key));
    anthropic.insert("api_base".to_owned(), json!(anthropic_base_url(&target.base_url)));
    anthropic.insert("enabled".to_owned(), json!(true));

    write_settings(&path, &root)
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
    let previous: Map<String, Value> = backup
        .existed
        .then(|| serde_json::from_str::<Value>(&backup.content).ok())
        .flatten()
        .and_then(|value| value.get("config").and_then(Value::as_object).cloned())
        .unwrap_or_default();

    if let Some(config) = object.get_mut("config").and_then(Value::as_object_mut) {
        restore_key(config, "api_key", &previous);
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
            let previous_anthropic = previous_providers
                .get("anthropic")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(entry) = providers.get_mut("anthropic").and_then(Value::as_object_mut) {
                for key in ["api_key", "api_base", "enabled"] {
                    restore_key(entry, key, &previous_anthropic);
                }
                if entry.is_empty() {
                    providers.remove("anthropic");
                }
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
    use super::*;

    fn target() -> RouteTarget {
        RouteTarget {
            base_url: "https://gateway.example.org".into(),
            api_key: "sk-gateway".into(),
            models: Vec::new(),
        }
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn taking_over_writes_the_keys_the_engine_reads() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &target(), &mut backups).unwrap();

        let root = read(&settings_path(&dir));
        let config = root.get("config").unwrap();
        assert_eq!(config.get("api_key").unwrap(), "sk-gateway");
        assert_eq!(config.get("provider").unwrap(), "anthropic");
        let anthropic = config
            .pointer("/provider_configs/anthropic")
            .expect("the anthropic entry");
        assert_eq!(anthropic.get("api_key").unwrap(), "sk-gateway");
        assert_eq!(
            anthropic.get("api_base").unwrap(),
            &json!(anthropic_base_url("https://gateway.example.org"))
        );
    }

    #[test]
    fn the_users_own_settings_survive_a_takeover() {
        let dir = tempdir();
        std::fs::write(
            settings_path(&dir),
            r#"{"config":{"model":"claude-opus-5","auto_compact":true},"permissionRules":[{"keep":1}]}"#,
        )
        .unwrap();

        let mut backups = CliBackups::default();
        take_over(&dir, &target(), &mut backups).unwrap();

        let root = read(&settings_path(&dir));
        assert_eq!(root.pointer("/config/model").unwrap(), "claude-opus-5");
        assert_eq!(root.pointer("/config/auto_compact").unwrap(), &json!(true));
        assert!(root.get("permissionRules").is_some());
    }

    #[test]
    fn releasing_restores_a_key_the_user_had_before() {
        let dir = tempdir();
        std::fs::write(
            settings_path(&dir),
            r#"{"config":{"api_key":"sk-user-own","model":"claude-opus-5"}}"#,
        )
        .unwrap();

        let mut backups = CliBackups::default();
        take_over(&dir, &target(), &mut backups).unwrap();
        assert_eq!(
            read(&settings_path(&dir)).pointer("/config/api_key").unwrap(),
            "sk-gateway"
        );

        restore(&dir, &backups).unwrap();
        let root = read(&settings_path(&dir));
        assert_eq!(root.pointer("/config/api_key").unwrap(), "sk-user-own");
        // Untouched throughout.
        assert_eq!(root.pointer("/config/model").unwrap(), "claude-opus-5");
        // Nothing we added is left behind.
        assert!(root.pointer("/config/provider_configs").is_none());
        assert!(root.pointer("/config/provider").is_none());
    }

    #[test]
    fn releasing_a_file_we_created_deletes_it_again() {
        let dir = tempdir();
        let mut backups = CliBackups::default();
        take_over(&dir, &target(), &mut backups).unwrap();
        restore(&dir, &backups).unwrap();

        assert!(
            !settings_path(&dir).exists(),
            "a settings file that only existed because of us should be gone"
        );
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
