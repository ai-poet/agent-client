//! Pi: `~/.pi/agent/models.json`, additive.
//!
//! One entry under `providers.cheaprouter`. The invariant this module
//! inherits from cc-switch (which ships a byte-equality test for it): Pi's
//! `auth.json` and `settings.json` are **never** read or written — `/login`
//! credentials and the default-model choice stay entirely the user's.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

use super::{PROVIDER_ID, RouteTarget, atomic_write_private};
use crate::brand;
use crate::gateway::openai_base_url;

const MODELS_FILE: &str = "models.json";

pub fn take_over(pi_agent_dir: &Path, target: &RouteTarget) -> Result<()> {
    let path = pi_agent_dir.join(MODELS_FILE);
    let mut root = read_models(&path)?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", path.display()))?;
    let providers = object
        .entry("providers")
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`providers` in {} is not an object", path.display()))?;

    let mut entry = Map::new();
    entry.insert("name".to_owned(), json!(brand::DISPLAY_NAME));
    entry.insert("baseUrl".to_owned(), json!(openai_base_url(&target.base_url)));
    entry.insert("api".to_owned(), json!("openai-completions"));
    entry.insert("apiKey".to_owned(), json!(target.api_key));
    if !target.models.is_empty() {
        let models: Vec<Value> = target.models.iter().map(|id| json!({ "id": id })).collect();
        entry.insert("models".to_owned(), Value::Array(models));
    }
    providers.insert(PROVIDER_ID.to_owned(), Value::Object(entry));

    write_models(&path, &root)
}

pub fn remove(pi_agent_dir: &Path) -> Result<()> {
    let path = pi_agent_dir.join(MODELS_FILE);
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_models(&path)?;
    let Some(object) = root.as_object_mut() else {
        return Err(anyhow!("{} is not a JSON object", path.display()));
    };
    let Some(providers) = object.get_mut("providers").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if providers.remove(PROVIDER_ID).is_none() {
        return Ok(());
    }
    write_models(&path, &root)
}

fn read_models(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("{} is not valid JSON", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({ "providers": {} }))
        }
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn write_models(path: &Path, root: &Value) -> Result<()> {
    let mut encoded = serde_json::to_string_pretty(root).context("could not encode models.json")?;
    encoded.push('\n');
    atomic_write_private(path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sub2api-pi-{tag}-{}-{}",
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
            api_key: "sk-pi".to_owned(),
            models: models.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn coexists_and_never_touches_auth_or_settings() {
        let dir = temp_dir("invariant");
        std::fs::write(
            dir.join(MODELS_FILE),
            r#"{"providers":{"anthropic":{"apiKey":"user-key"}}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("auth.json"), r#"{"pi":"oauth-material"}"#).unwrap();
        std::fs::write(dir.join("settings.json"), r#"{"defaultProvider":"anthropic"}"#).unwrap();

        take_over(&dir, &target(&["gpt-a", "gpt-b"])).expect("take over");
        let live: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MODELS_FILE)).unwrap())
                .unwrap();
        assert_eq!(live["providers"]["anthropic"]["apiKey"], "user-key");
        let ours = &live["providers"][PROVIDER_ID];
        assert_eq!(ours["baseUrl"], "https://gw.example.org/v1");
        assert_eq!(ours["api"], "openai-completions");
        assert_eq!(ours["models"][1]["id"], "gpt-b");

        remove(&dir).expect("remove");
        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MODELS_FILE)).unwrap())
                .unwrap();
        assert!(after["providers"].get(PROVIDER_ID).is_none());
        assert_eq!(after["providers"]["anthropic"]["apiKey"], "user-key");

        // The cc-switch invariant, byte for byte.
        assert_eq!(
            std::fs::read_to_string(dir.join("auth.json")).unwrap(),
            r#"{"pi":"oauth-material"}"#
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).unwrap(),
            r#"{"defaultProvider":"anthropic"}"#
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_the_document_when_absent_and_removal_is_idempotent() {
        let dir = temp_dir("fresh");
        remove(&dir).expect("remove before create is fine");
        take_over(&dir, &target(&[])).expect("take over");
        let live: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MODELS_FILE)).unwrap())
                .unwrap();
        assert!(live["providers"][PROVIDER_ID].get("models").is_none());
        remove(&dir).expect("remove");
        remove(&dir).expect("remove again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
