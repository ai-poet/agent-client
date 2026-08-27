//! OpenCode: `~/.config/opencode/opencode.json`, additive.
//!
//! We own exactly one entry, `provider.cheaprouter`; the user's other
//! providers, theme, MCP servers, and plugins are values in the same
//! document and are carried through the read-modify-write untouched. A file
//! that cannot be parsed, or whose root is not an object, is never written —
//! reporting beats silently rebuilding a user's config (cc-switch's rule).

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

use super::{PROVIDER_ID, RouteTarget, atomic_write_private};
use crate::brand;
use crate::gateway::openai_base_url;

const CONFIG_FILE: &str = "opencode.json";
const SCHEMA_URL: &str = "https://opencode.ai/config.json";

pub fn take_over(opencode_dir: &Path, target: &RouteTarget) -> Result<()> {
    let path = opencode_dir.join(CONFIG_FILE);
    let mut root = read_config(&path)?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", path.display()))?;

    let providers = object
        .entry("provider")
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`provider` in {} is not an object", path.display()))?;

    let mut models = Map::new();
    for id in &target.models {
        models.insert(id.clone(), json!({ "name": id }));
    }
    let mut entry = Map::new();
    entry.insert("npm".to_owned(), json!("@ai-sdk/openai-compatible"));
    entry.insert("name".to_owned(), json!(brand::DISPLAY_NAME));
    entry.insert(
        "options".to_owned(),
        json!({
            "baseURL": openai_base_url(&target.base_url),
            "apiKey": target.api_key,
        }),
    );
    if !models.is_empty() {
        entry.insert("models".to_owned(), Value::Object(models));
    }
    providers.insert(PROVIDER_ID.to_owned(), Value::Object(entry));

    write_config(&path, &root)
}

pub fn remove(opencode_dir: &Path) -> Result<()> {
    let path = opencode_dir.join(CONFIG_FILE);
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_config(&path)?;
    let Some(object) = root.as_object_mut() else {
        return Err(anyhow!("{} is not a JSON object", path.display()));
    };
    let Some(providers) = object.get_mut("provider").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if providers.remove(PROVIDER_ID).is_none() {
        return Ok(());
    }
    if providers.is_empty() {
        object.remove("provider");
    }
    write_config(&path, &root)
}

fn read_config(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("{} is not valid JSON", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({ "$schema": SCHEMA_URL }))
        }
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn write_config(path: &Path, root: &Value) -> Result<()> {
    let mut encoded = serde_json::to_string_pretty(root).context("could not encode opencode.json")?;
    encoded.push('\n');
    atomic_write_private(path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-opencode-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn target(models: &[&str]) -> RouteTarget {
        RouteTarget {
            base_url: "https://gw.example.org".to_owned(),
            api_key: "sk-oc".to_owned(),
            models: models.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn coexists_with_user_providers_and_removes_cleanly() {
        let dir = temp_dir("coexist");
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"$schema":"https://opencode.ai/config.json","theme":"dark",
                "provider":{"mine":{"npm":"@ai-sdk/openai-compatible",
                    "options":{"baseURL":"https://mine.example.org/v1","apiKey":"user-key"}}}}"#,
        )
        .unwrap();
        take_over(&dir, &target(&["gpt-x"])).expect("take over");

        let live: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap())
                .unwrap();
        assert_eq!(live["theme"], "dark");
        assert_eq!(live["provider"]["mine"]["options"]["apiKey"], "user-key");
        let ours = &live["provider"][PROVIDER_ID];
        assert_eq!(ours["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(ours["options"]["baseURL"], "https://gw.example.org/v1");
        assert_eq!(ours["options"]["apiKey"], "sk-oc");
        assert_eq!(ours["models"]["gpt-x"]["name"], "gpt-x");

        remove(&dir).expect("remove");
        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap())
                .unwrap();
        assert!(after["provider"].get(PROVIDER_ID).is_none());
        assert_eq!(after["provider"]["mine"]["options"]["apiKey"], "user-key");
        assert_eq!(after["theme"], "dark");
        // Removing again is a no-op.
        remove(&dir).expect("idempotent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_a_skeleton_with_schema_when_absent() {
        let dir = temp_dir("fresh");
        take_over(&dir, &target(&[])).expect("take over");
        let live: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap())
                .unwrap();
        assert_eq!(live["$schema"], SCHEMA_URL);
        // No models key when none were given — an empty map would render as
        // a provider with zero models rather than "unspecified".
        assert!(live["provider"][PROVIDER_ID].get("models").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_rebuild_an_unparseable_or_non_object_file() {
        let dir = temp_dir("corrupt");
        std::fs::write(dir.join(CONFIG_FILE), "[1,2,3]").unwrap();
        assert!(take_over(&dir, &target(&[])).is_err());
        assert_eq!(std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap(), "[1,2,3]");
        std::fs::write(dir.join(CONFIG_FILE), "{ nope").unwrap();
        assert!(take_over(&dir, &target(&[])).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
