// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! AgentTeams settings: the reference plugin's `Config` (same camelCase keys
//! and defaults) plus an `enabled` switch, stored as `agent-teams.json`.
//!
//! Keys this build does not know (the reference's `memberProvider`,
//! `promptSectionOrder`, …) are kept in `extra` and written back unchanged.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::Path;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::profiles::{self, TeamProfileConfig};
use crate::types::TeamModelFallback;
use crate::validate::strip_bom;

/// File name of the settings file.
pub const CONFIG_FILE: &str = "agent-teams.json";
/// Team state lives at `<workspace>/<stateDir>/<teamId>/`.
pub use crate::store::DEFAULT_STATE_DIR;
/// Default team size cap in members.
pub const DEFAULT_MAX_MEMBERS: u32 = 8;
/// Hard ceiling on `maxMembers`.
pub const MAX_MEMBERS_LIMIT: u32 = 16;
/// Hard ceiling on `memberMaxDepth`: a member may delegate at most one level.
pub const MAX_MEMBER_DEPTH_LIMIT: u32 = 1;

/// The settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TeamsConfig {
    /// Whether AgentTeams is offered to the built-in agent at all.
    pub enabled: bool,
    /// State directory name under the captain's workspace.
    pub state_dir: String,
    /// Model override applied to every member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_model: Option<String>,
    /// Reasoning effort applied to every member that names none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_reasoning_effort: Option<String>,
    /// Prompt injected into member personas and task assignments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_prompt: Option<String>,
    /// Plugin-wide fallback route for unavailable member models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<TeamModelFallback>,
    /// Member delegation depth cap; `0` forbids delegation. Read it through
    /// [`TeamsConfig::member_max_depth`].
    pub member_max_depth: u32,
    /// Team size cap. Read it through [`TeamsConfig::max_members`].
    pub max_members: u32,
    /// Offer the `/agent-teams` command and its profile aliases.
    pub slash_command: bool,
    /// Named profiles. `None` means never configured: the caller seeds the
    /// built-ins ([`TeamsConfig::effective_profiles`]). `Some` of an empty
    /// map means the user removed them all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profiles: Option<BTreeMap<String, TeamProfileConfig>>,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for TeamsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            state_dir: DEFAULT_STATE_DIR.to_owned(),
            member_model: None,
            member_reasoning_effort: None,
            execution_prompt: None,
            fallback: None,
            member_max_depth: 0,
            max_members: DEFAULT_MAX_MEMBERS,
            slash_command: true,
            profiles: None,
            extra: Map::new(),
        }
    }
}

impl TeamsConfig {
    /// The state directory name, with a blank value read as the default.
    pub fn state_dir(&self) -> &str {
        match self.state_dir.trim() {
            "" => DEFAULT_STATE_DIR,
            dir => dir,
        }
    }

    /// `memberMaxDepth` clamped to `0..=1`.
    pub fn member_max_depth(&self) -> u32 {
        self.member_max_depth.min(MAX_MEMBER_DEPTH_LIMIT)
    }

    /// `maxMembers` clamped to `1..=16`.
    pub fn max_members(&self) -> u32 {
        self.max_members.clamp(1, MAX_MEMBERS_LIMIT)
    }

    /// The configured profiles, or the built-ins when none were ever
    /// configured.
    pub fn effective_profiles(&self) -> BTreeMap<String, TeamProfileConfig> {
        match &self.profiles {
            Some(profiles) => profiles.clone(),
            None => profiles::builtin_profiles(),
        }
    }

    /// Check every effective profile the way team creation will: at most
    /// [`profiles::MAX_TEAM_PROFILES`] distinct non-empty keys, each body
    /// valid under this config's `maxMembers`.
    pub fn validate(&self) -> Result<(), String> {
        let effective = self.effective_profiles();
        let max_members = self.max_members() as usize;
        for entry in profiles::list_configured_profiles(&effective)? {
            let value = serde_json::to_value(entry.config).map_err(|error| error.to_string())?;
            profiles::normalize_team_profile_value(&entry.name, &value, max_members)?;
        }
        Ok(())
    }
}

/// Read the settings. A missing or blank file is the default; a leading BOM
/// is tolerated; anything that does not parse is an error.
pub fn load(path: &Path) -> anyhow::Result<TeamsConfig> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TeamsConfig::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    parse(&text).with_context(|| format!("invalid AgentTeams settings in {}", path.display()))
}

/// Parse settings text (see [`load`]).
pub fn parse(text: &str) -> anyhow::Result<TeamsConfig> {
    let text = strip_bom(text);
    if text.trim().is_empty() {
        return Ok(TeamsConfig::default());
    }
    match serde_json::from_str::<TeamsConfig>(text) {
        Ok(config) => Ok(config),
        Err(error) => {
            // A profile body of the wrong shape: the reference's message
            // names the offending key, serde's only the position.
            if let Ok(value) = serde_json::from_str::<Value>(text)
                && let Some(profiles) = value.get("profiles")
                && let Err(message) = profiles::parse_profiles(profiles)
            {
                bail!("{message}");
            }
            Err(error.into())
        }
    }
}

/// Write the settings as pretty JSON, atomically: a temporary file in the
/// same directory is renamed over the target.
pub fn save(path: &Path, config: &TeamsConfig) -> anyhow::Result<()> {
    let mut text =
        serde_json::to_string_pretty(config).context("failed to encode AgentTeams settings")?;
    text.push('\n');
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(CONFIG_FILE);
    let temp = dir.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = write_temp(&temp, text.as_bytes())
        .and_then(|()| replace_file(&temp, path, text.as_bytes()));
    // After a successful rename the temporary file is gone already.
    let _ = fs::remove_file(&temp);
    result.with_context(|| format!("failed to write {}", path.display()))
}

fn write_temp(temp: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(temp)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn replace_file(temp: &Path, path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut result = fs::rename(temp, path);
    if cfg!(windows) {
        // Another process (an editor, a virus scanner) holding the target
        // open without delete sharing makes the rename fail for a moment.
        for _ in 0..3 {
            match &result {
                Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    result = fs::rename(temp, path);
                }
                _ => break,
            }
        }
        if let Err(error) = &result
            && error.kind() == ErrorKind::PermissionDenied
        {
            result = fs::write(path, contents);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_the_reference() {
        let config = TeamsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.state_dir, ".agent-teams");
        assert_eq!(config.member_max_depth, 0);
        assert_eq!(config.max_members, 8);
        assert!(config.slash_command);
        assert_eq!(config.profiles, None);
        assert_eq!(parse("{}").unwrap(), config);
        assert_eq!(parse("  \n").unwrap(), config);
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            json!({
                "enabled": true,
                "stateDir": ".agent-teams",
                "memberMaxDepth": 0,
                "maxMembers": 8,
                "slashCommand": true
            })
        );
    }

    #[test]
    fn partial_settings_keep_the_other_defaults() {
        let config = parse(r#"{"enabled": false, "memberModel": "m", "maxMembers": 3}"#).unwrap();
        assert!(!config.enabled);
        assert_eq!(config.member_model.as_deref(), Some("m"));
        assert_eq!(config.max_members(), 3);
        assert!(config.slash_command);
        assert_eq!(config.state_dir(), ".agent-teams");
    }

    #[test]
    fn unknown_keys_round_trip() {
        let text =
            r#"{"memberProvider": "spawn", "promptSectionOrder": 117, "future": {"x": [1]}}"#;
        let config = parse(text).unwrap();
        assert_eq!(config.extra.get("memberProvider"), Some(&json!("spawn")));
        let written = serde_json::to_value(&config).unwrap();
        assert_eq!(written["memberProvider"], json!("spawn"));
        assert_eq!(written["promptSectionOrder"], json!(117));
        assert_eq!(written["future"], json!({"x": [1]}));
    }

    #[test]
    fn accessors_clamp() {
        let mut config = TeamsConfig {
            member_max_depth: 5,
            max_members: 0,
            ..TeamsConfig::default()
        };
        assert_eq!(config.member_max_depth(), 1);
        assert_eq!(config.max_members(), 1);
        config.max_members = 99;
        config.member_max_depth = 0;
        assert_eq!(config.max_members(), 16);
        assert_eq!(config.member_max_depth(), 0);
        config.state_dir = "  ".to_owned();
        assert_eq!(config.state_dir(), DEFAULT_STATE_DIR);
        config.state_dir = " teams ".to_owned();
        assert_eq!(config.state_dir(), "teams");
    }

    #[test]
    fn effective_profiles_seed_builtins_only_when_unset() {
        let config = TeamsConfig::default();
        assert_eq!(config.effective_profiles(), profiles::builtin_profiles());
        assert!(config.validate().is_ok());
        let emptied = TeamsConfig {
            profiles: Some(BTreeMap::new()),
            ..TeamsConfig::default()
        };
        assert!(emptied.effective_profiles().is_empty());
        let written = serde_json::to_value(&emptied).unwrap();
        assert_eq!(written["profiles"], json!({}));
        assert_eq!(parse(&written.to_string()).unwrap(), emptied);
    }

    #[test]
    fn validate_reports_profile_errors() {
        let config = parse(
            r#"{"profiles": {"solo": {"members": [{"name": "w", "reasoningEffort": "x"}]}}}"#,
        )
        .unwrap();
        assert_eq!(
            config.validate().unwrap_err(),
            "profiles.solo.members[0].reasoningEffort is unknown; did you mean reasoning_effort?"
        );
        let mut config = TeamsConfig {
            max_members: 3,
            ..TeamsConfig::default()
        };
        // dual-review has four members.
        assert_eq!(
            config.validate().unwrap_err(),
            "profile \"dual-review\" has 4 members but maxMembers is 3"
        );
        let one = config.effective_profiles()["implement-test-fix"].clone();
        config.max_members = 8;
        config.profiles = Some((0..17).map(|i| (format!("p{i}"), one.clone())).collect());
        assert_eq!(
            config.validate().unwrap_err(),
            "too many AgentTeams profiles (17); the limit is 16"
        );
    }

    #[test]
    fn a_misshapen_profile_reports_the_reference_message() {
        let error = parse(r#"{"profiles": {"solo": {"members": [{"name": 5}]}}}"#).unwrap_err();
        assert_eq!(
            error.to_string(),
            "profiles.solo.members[0].name must be a string"
        );
        assert!(parse("{not json").is_err());
        assert!(parse(r#"{"maxMembers": -1}"#).is_err());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(CONFIG_FILE);
        assert_eq!(load(&path).unwrap(), TeamsConfig::default());

        let mut config = TeamsConfig {
            enabled: false,
            member_model: Some("gpt".to_owned()),
            member_reasoning_effort: Some("high".to_owned()),
            fallback: Some(TeamModelFallback {
                provider: "openai".to_owned(),
                model: "backup".to_owned(),
            }),
            profiles: Some(profiles::builtin_profiles()),
            ..TeamsConfig::default()
        };
        config
            .extra
            .insert("memberProvider".to_owned(), json!("spawn"));
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);

        // Overwriting keeps the file whole and leaves no temporary behind.
        config.enabled = true;
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        let names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [CONFIG_FILE]);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\n  \"enabled\": true"));
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn a_leading_bom_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);
        fs::write(&path, "\u{feff}{\"maxMembers\": 4}").unwrap();
        assert_eq!(load(&path).unwrap().max_members(), 4);
        fs::write(&path, "{").unwrap();
        assert!(load(&path).is_err());
    }
}
