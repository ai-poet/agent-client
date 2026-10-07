//! Auto-memory settings, stored as `auto-memory.json` beside the engine's
//! `settings.json` — never inside it: the engine rewrites `settings.json`
//! from its typed struct and would drop keys it does not know.
//!
//! Keys this build does not know are kept in `extra` and written back.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fsutil::atomic_write_text;

/// File name of the settings file.
pub const CONFIG_FILE: &str = "auto-memory.json";

/// The budget presets the settings page offers.
pub const MAX_BYTES_PRESETS: [u32; 3] = [2048, 4096, 8192];
/// The stale-after presets (days; `0` is off).
pub const STALE_PRESETS: [u32; 4] = [0, 30, 90, 180];

/// The settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryConfig {
    /// Whether the built-in agent has memory at all.
    pub enabled: bool,
    /// The user scope (memories shared by every session).
    pub enable_user_scope: bool,
    /// Extract new memories from the conversation in the background.
    pub auto_summarize: bool,
    /// Most memories one consolidation pass files.
    pub auto_summarize_max_memories: u32,
    /// Output token cap of a consolidation call.
    pub auto_summarize_max_tokens: u32,
    /// A pass runs after this many turns the person started.
    pub auto_summarize_every_turns: u32,
    /// Mirror project memories into Claude Code's project memory.
    pub mirror_to_claude_code: bool,
    /// Byte budget of the prompt section (index and policy together).
    pub max_bytes: u32,
    /// Leave memories nobody read out of the index after this many days
    /// without an update; `0` is off.
    pub stale_after_days: u32,
    /// Keys this build does not know.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            enable_user_scope: true,
            auto_summarize: true,
            auto_summarize_max_memories: 5,
            auto_summarize_max_tokens: 4096,
            auto_summarize_every_turns: 12,
            mirror_to_claude_code: true,
            max_bytes: 4096,
            stale_after_days: 0,
            extra: Map::new(),
        }
    }
}

impl MemoryConfig {
    pub fn max_bytes(&self) -> usize {
        self.max_bytes.clamp(1024, 32_768) as usize
    }

    pub fn stale_after_days(&self) -> u32 {
        self.stale_after_days.min(3650)
    }

    pub fn auto_summarize_max_memories(&self) -> usize {
        self.auto_summarize_max_memories.clamp(1, 20) as usize
    }

    pub fn auto_summarize_max_tokens(&self) -> u32 {
        self.auto_summarize_max_tokens.clamp(512, 32_000)
    }

    pub fn auto_summarize_every_turns(&self) -> u32 {
        self.auto_summarize_every_turns.clamp(2, 200)
    }
}

/// Read the settings: a missing or blank file is the default, a leading BOM
/// is fine, anything that does not parse is an error.
pub fn load(path: &Path) -> anyhow::Result<MemoryConfig> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(MemoryConfig::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    parse(&text).with_context(|| format!("invalid auto-memory settings in {}", path.display()))
}

/// Parse settings text (see [`load`]).
pub fn parse(text: &str) -> anyhow::Result<MemoryConfig> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim().is_empty() {
        return Ok(MemoryConfig::default());
    }
    Ok(serde_json::from_str(text)?)
}

/// Write the settings as pretty JSON, atomically.
pub fn save(path: &Path, config: &MemoryConfig) -> anyhow::Result<()> {
    let mut text =
        serde_json::to_string_pretty(config).context("failed to encode auto-memory settings")?;
    text.push('\n');
    atomic_write_text(path, &text).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_turn_everything_on_but_eviction() {
        let config = MemoryConfig::default();
        assert!(config.enabled && config.enable_user_scope);
        assert!(config.auto_summarize && config.mirror_to_claude_code);
        assert_eq!(config.max_bytes(), 4096);
        assert_eq!(config.stale_after_days(), 0);
    }

    #[test]
    fn missing_blank_and_bom_files_load() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load(&tmp.path().join("none.json")).unwrap(), MemoryConfig::default());
        assert_eq!(parse("\u{feff}  ").unwrap(), MemoryConfig::default());
        assert!(!parse("{\"autoSummarize\": false}").unwrap().auto_summarize);
        assert!(parse("{").is_err());
    }

    #[test]
    fn unknown_keys_round_trip_and_values_are_clamped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(CONFIG_FILE);
        let mut config = parse("{\"futureKey\": 1, \"maxBytes\": 10, \"autoSummarizeEveryTurns\": 0}").unwrap();
        assert_eq!(config.max_bytes(), 1024);
        assert_eq!(config.auto_summarize_every_turns(), 2);
        config.enabled = false;
        save(&path, &config).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.extra.get("futureKey"), Some(&Value::from(1)));
        assert!(!back.enabled);
    }
}
