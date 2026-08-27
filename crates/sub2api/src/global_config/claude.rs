//! Claude Code: `~/.claude/settings.json`, the `env` block only.
//!
//! Everything else in the file — permissions, hooks, statusLine, the model
//! pin — is the user's and is never touched. Restore puts the managed keys
//! back to their pre-takeover values from the file backup, so edits the
//! user made *while* managed survive too.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

use super::{CliBackups, RouteTarget, atomic_write_private, capture_backup, remove_if_exists};
use crate::gateway::anthropic_base_url;

const SETTINGS_FILE: &str = "settings.json";

/// The keys we own inside the `env` block. `DISABLE_NONESSENTIAL_TRAFFIC`
/// and `ATTRIBUTION_HEADER` ride along exactly as the Electron client wrote
/// them, so a gateway session leaks no side traffic.
const MANAGED_ENV_KEYS: [&str; 4] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_ATTRIBUTION_HEADER",
];

pub fn take_over(claude_dir: &Path, target: &RouteTarget, backups: &mut CliBackups) -> Result<()> {
    let path = claude_dir.join(SETTINGS_FILE);
    capture_backup(backups, SETTINGS_FILE, &path)?;

    let mut root = read_settings(&path)?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", path.display()))?;
    let env = object
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    let env = env
        .as_object_mut()
        .ok_or_else(|| anyhow!("`env` in {} is not an object", path.display()))?;
    env.insert(
        "ANTHROPIC_BASE_URL".to_owned(),
        json!(anthropic_base_url(&target.base_url)),
    );
    env.insert("ANTHROPIC_AUTH_TOKEN".to_owned(), json!(target.api_key));
    env.insert(
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
        json!("1"),
    );
    env.insert("CLAUDE_CODE_ATTRIBUTION_HEADER".to_owned(), json!("0"));

    write_settings(&path, &root)
}

pub fn restore(claude_dir: &Path, backups: &CliBackups) -> Result<()> {
    let Some(backup) = backups.get(SETTINGS_FILE) else {
        return Ok(());
    };
    let path = claude_dir.join(SETTINGS_FILE);
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

    // The original values of our keys, if the file existed before us.
    let backup_env: Map<String, Value> = backup
        .existed
        .then(|| serde_json::from_str::<Value>(&backup.content).ok())
        .flatten()
        .and_then(|value| {
            value
                .get("env")
                .and_then(Value::as_object)
                .cloned()
        })
        .unwrap_or_default();

    if let Some(env) = object.get_mut("env").and_then(Value::as_object_mut) {
        for key in MANAGED_ENV_KEYS {
            match backup_env.get(key) {
                Some(original) => {
                    env.insert(key.to_owned(), original.clone());
                }
                None => {
                    env.remove(key);
                }
            }
        }
        if env.is_empty() {
            object.remove("env");
        }
    }

    // A file that only ever existed because of us disappears again.
    if !backup.existed && object.is_empty() {
        return remove_if_exists(&path);
    }
    write_settings(&path, &root)
}

fn read_settings(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("{} is not valid JSON", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn write_settings(path: &Path, root: &Value) -> Result<()> {
    let mut encoded = serde_json::to_string_pretty(root).context("could not encode settings")?;
    encoded.push('\n');
    atomic_write_private(path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-claude-{tag}-{}-{}",
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
            base_url: "https://gw.example.org/v1/".to_owned(),
            api_key: "sk-live".to_owned(),
            models: Vec::new(),
        }
    }

    #[test]
    fn merges_env_keys_and_keeps_everything_else() {
        let dir = temp_dir("merge");
        std::fs::write(
            dir.join(SETTINGS_FILE),
            r#"{"permissions":{"allow":["Bash"]},"hooks":{"PostToolUse":[]},
                "env":{"ANTHROPIC_BASE_URL":"https://old.example.org","FOO":"bar"}}"#,
        )
        .unwrap();
        let mut backups = BTreeMap::new();
        take_over(&dir, &target(), &mut backups).expect("take over");

        let live: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap())
                .unwrap();
        // Endpoint normalized, token written, riders present.
        assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://gw.example.org");
        assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-live");
        assert_eq!(live["env"]["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
        // User content intact.
        assert_eq!(live["env"]["FOO"], "bar");
        assert_eq!(live["permissions"]["allow"][0], "Bash");
        assert!(live.get("hooks").is_some());

        // Restore: our keys revert to the ORIGINAL values, FOO survives, and
        // an edit made while managed (a new top-level key) survives too.
        let mut live_edit: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap())
                .unwrap();
        live_edit["theme"] = serde_json::json!("dark");
        std::fs::write(
            dir.join(SETTINGS_FILE),
            serde_json::to_string_pretty(&live_edit).unwrap(),
        )
        .unwrap();

        restore(&dir, &backups).expect("restore");
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap())
                .unwrap();
        assert_eq!(restored["env"]["ANTHROPIC_BASE_URL"], "https://old.example.org");
        assert!(restored["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        assert!(
            restored["env"]
                .get("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC")
                .is_none()
        );
        assert_eq!(restored["env"]["FOO"], "bar");
        assert_eq!(restored["theme"], "dark");
        assert_eq!(restored["permissions"]["allow"][0], "Bash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_we_created_disappears_on_restore() {
        let dir = temp_dir("fresh");
        let mut backups = BTreeMap::new();
        take_over(&dir, &target(), &mut backups).expect("take over");
        assert!(dir.join(SETTINGS_FILE).exists());
        restore(&dir, &backups).expect("restore");
        assert!(!dir.join(SETTINGS_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_takeover_keeps_the_first_backup() {
        let dir = temp_dir("switch");
        std::fs::write(
            dir.join(SETTINGS_FILE),
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"user-own-token"}}"#,
        )
        .unwrap();
        let mut backups = BTreeMap::new();
        take_over(&dir, &target(), &mut backups).expect("first");
        // A group switch writes a different key; the backup must still hold
        // the USER's token, not our first one.
        let second = RouteTarget {
            api_key: "sk-second".to_owned(),
            ..target()
        };
        take_over(&dir, &second, &mut backups).expect("second");
        restore(&dir, &backups).expect("restore");
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap())
                .unwrap();
        assert_eq!(restored["env"]["ANTHROPIC_AUTH_TOKEN"], "user-own-token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unparseable_file_is_never_touched() {
        let dir = temp_dir("corrupt");
        std::fs::write(dir.join(SETTINGS_FILE), "{ not json").unwrap();
        let mut backups = BTreeMap::new();
        assert!(take_over(&dir, &target(), &mut backups).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
