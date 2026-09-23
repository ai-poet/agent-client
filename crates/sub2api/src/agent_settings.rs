//! The built-in agent's own settings, as the Agent settings page edits them.
//!
//! The engine reads one file — `settings.json` in its config directory — and
//! that file is also where routing is written (`global_config::native`) and
//! where "always allow" answers land (through the engine's own permission
//! manager). This module owns the remaining keys a user would want a page
//! for: behaviour, tools, MCP servers, permission rules.
//!
//! It edits the live file rather than a copy, the way every other writer in
//! this crate does, and it touches only the keys listed in [`save`]. The
//! engine's model pin, hooks, agent definitions, plugin lists and anything a
//! future engine version adds pass through untouched — which is what lets the
//! vendored engine be upgraded without this page silently discarding settings
//! it never heard of.
//!
//! GPUI-free, like the rest of the crate, so the page's logic compiles and
//! tests in seconds.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::global_config::{atomic_write_private, native};

/// One MCP server as the engine's `McpServerConfig` serializes it.
///
/// `transport` is the engine's `type` field: `stdio` runs `command` with
/// `args`, anything else (`http`, `sse`) connects to `url`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    #[serde(default = "stdio", rename = "type")]
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

fn stdio() -> String {
    "stdio".to_owned()
}

impl McpServer {
    pub fn is_stdio(&self) -> bool {
        self.transport == "stdio"
    }

    /// A one-line description for a list row: the command line or the URL.
    pub fn summary(&self) -> String {
        if self.is_stdio() {
            let mut parts = vec![self.command.clone().unwrap_or_default()];
            parts.extend(self.args.iter().cloned());
            parts.join(" ").trim().to_owned()
        } else {
            self.url.clone().unwrap_or_default()
        }
    }

    /// Whether the entry could be launched at all. The page refuses to save
    /// an unusable one rather than letting the engine fail on it at startup.
    pub fn is_usable(&self) -> bool {
        if self.name.trim().is_empty() {
            return false;
        }
        if self.is_stdio() {
            self.command.as_deref().is_some_and(|command| !command.trim().is_empty())
        } else {
            self.url.as_deref().is_some_and(|url| !url.trim().is_empty())
        }
    }
}

/// A persisted permission rule, as the engine's `SerializedPermissionRule`
/// serializes it. `action` is the enum's variant name: `Allow` or `Deny`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PermissionRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_pattern: Option<String>,
    pub action: String,
}

impl PermissionRule {
    pub fn is_allow(&self) -> bool {
        self.action == "Allow"
    }

    /// What the rule applies to, for a list row.
    pub fn subject(&self) -> String {
        match (&self.tool_name, &self.path_pattern) {
            (Some(tool), Some(path)) => format!("{tool} · {path}"),
            (Some(tool), None) => tool.clone(),
            (None, Some(path)) => path.clone(),
            (None, None) => "*".to_owned(),
        }
    }
}

/// The keys this page owns. Everything else in the file is left alone.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct AgentSettings {
    /// Appended after the engine's own system prompt. The place for house
    /// rules that should apply to every session.
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    #[serde(default)]
    pub auto_compact: Option<bool>,
    /// Fraction of the context window (0–1) at which compaction runs.
    #[serde(default)]
    pub compact_threshold: Option<f32>,
    /// Built-in tools the model must not be offered.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Whether MCP servers declared inside a repository may launch without
    /// being approved first. Off by default — the engine treats a
    /// project-defined server as untrusted code, and so should the page.
    #[serde(default)]
    pub trust_project_mcp_servers: bool,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(default)]
    pub permission_rules: Vec<PermissionRule>,
}

/// Every built-in tool the engine offers, for the Tools section.
///
/// Mirrors `claurst_tools::all_tools()` plus the sub-agent tool from
/// `claurst_query`. `waku-agent-bridge` asserts in its tests that this list
/// and the engine's agree, so the page cannot drift from what the model is
/// actually offered — and this crate stays free of the engine crates.
pub const BUILTIN_TOOLS: [&str; 45] = [
    "Agent", "ApplyPatch", "AskUserQuestion", "Bash", "BatchEdit", "Brief", "Config",
    "CronCreate", "CronDelete", "CronList", "Edit", "EnterPlanMode", "EnterWorktree",
    "ExitPlanMode", "ExitWorktree", "Glob", "GoalComplete", "Grep", "LSP",
    "ListMcpResources", "NotebookEdit", "PowerShell", "REPL", "Read",
    "ReadMcpResource", "RemoteTrigger", "SendMessage", "Skill", "Sleep",
    "StructuredOutput", "TaskCreate", "TaskGet", "TaskList", "TaskOutput", "TaskStop",
    "TaskUpdate", "TeamCreate", "TeamDelete", "TodoWrite", "ToolSearch", "WebFetch",
    "WebSearch", "Write", "mcp__auth", "monitor",
];

/// Tools the page does not offer to disable: without them the agent cannot
/// read, edit, run, or search, and a session with them off would look broken
/// rather than restricted.
pub const ESSENTIAL_TOOLS: [&str; 7] =
    ["Read", "Edit", "Write", "Bash", "Glob", "Grep", "PowerShell"];

/// Where the engine's settings live.
pub fn settings_path() -> Option<PathBuf> {
    native::config_dir().map(|dir| native::settings_path(&dir))
}

/// Read the owned keys. An absent or unreadable file yields defaults; the
/// page shows an empty form rather than refusing to open.
pub fn load() -> AgentSettings {
    settings_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .map(|root| from_document(&root))
        .unwrap_or_default()
}

/// Write the owned keys back, preserving every other key in the file.
pub fn save(settings: &AgentSettings) -> Result<()> {
    let path = settings_path().ok_or_else(|| anyhow!("no home directory to store settings in"))?;
    let mut root = match std::fs::read_to_string(&path) {
        Ok(raw) if !raw.trim().is_empty() => serde_json::from_str::<Value>(&raw)
            .with_context(|| format!("{} is not valid JSON", path.display()))?,
        Ok(_) => json!({}),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error).with_context(|| format!("could not read {}", path.display())),
    };
    apply(&mut root, settings)?;
    let mut rendered = serde_json::to_string_pretty(&root)?;
    rendered.push('\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    atomic_write_private(&path, rendered.as_bytes())
}

fn from_document(root: &Value) -> AgentSettings {
    let config = root.get("config").and_then(Value::as_object);
    let field = |key: &str| config.and_then(|config| config.get(key));
    AgentSettings {
        append_system_prompt: field("append_system_prompt")
            .and_then(Value::as_str)
            .map(str::to_owned),
        auto_compact: field("auto_compact").and_then(Value::as_bool),
        compact_threshold: field("compact_threshold")
            .and_then(Value::as_f64)
            .map(|value| value as f32),
        disallowed_tools: field("disallowed_tools")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        trust_project_mcp_servers: root
            .get("trustProjectMcpServers")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        mcp_servers: field("mcp_servers")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
        permission_rules: root
            .get("permissionRules")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
    }
}

/// Set the owned keys on the document. A key whose value is `None` is
/// removed, not written as `null`: the engine's own default applies then,
/// which is what "unset" should mean.
fn apply(root: &mut Value, settings: &AgentSettings) -> Result<()> {
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("the settings file is not a JSON object"))?;
    let config = object
        .entry("config")
        .or_insert_with(|| Value::Object(Map::new()));
    let config = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("`config` in the settings file is not an object"))?;

    set_optional(config, "append_system_prompt", settings.append_system_prompt.as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
        .map(|prompt| json!(prompt)));
    // Earlier builds offered a per-turn step cap here and wrote it under this
    // key. The engine never read it, and a message is no longer capped at
    // all, so the key goes on the next save.
    config.remove("max_turns");
    set_optional(config, "auto_compact", settings.auto_compact.map(|value| json!(value)));
    set_optional(
        config,
        "compact_threshold",
        settings.compact_threshold.map(|value| json!(value)),
    );
    config.insert(
        "disallowed_tools".to_owned(),
        json!(settings.disallowed_tools),
    );
    config.insert(
        "mcp_servers".to_owned(),
        serde_json::to_value(&settings.mcp_servers)?,
    );

    object.insert(
        "trustProjectMcpServers".to_owned(),
        json!(settings.trust_project_mcp_servers),
    );
    object.insert(
        "permissionRules".to_owned(),
        serde_json::to_value(&settings.permission_rules)?,
    );
    Ok(())
}

fn set_optional(object: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(value) => {
            object.insert(key.to_owned(), value);
        }
        None => {
            object.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_reads_as_defaults() {
        assert_eq!(from_document(&json!({})), AgentSettings::default());
    }

    #[test]
    fn owned_keys_round_trip_and_the_rest_of_the_file_survives() {
        let mut root = json!({
            "config": {
                "model": "claude-opus-5",
                "hooks": {"PreToolUse": [{"command": "echo"}]},
                "max_turns": 3
            },
            "enabledPlugins": ["thing"],
            "permissionRules": [{"tool_name": "Bash", "action": "Allow"}]
        });
        let mut settings = from_document(&root);
        assert_eq!(settings.permission_rules.len(), 1);

        settings.append_system_prompt = Some("Answer in Chinese.".into());
        settings.disallowed_tools = vec!["WebSearch".into()];
        settings.mcp_servers.push(McpServer {
            name: "fs".into(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "server-filesystem".into()],
            env: BTreeMap::new(),
            url: None,
        });
        settings.permission_rules.clear();
        apply(&mut root, &settings).unwrap();

        // Untouched.
        assert_eq!(root.pointer("/config/model").unwrap(), "claude-opus-5");
        assert!(root.pointer("/config/hooks/PreToolUse").is_some());
        assert_eq!(root.pointer("/enabledPlugins/0").unwrap(), "thing");
        // The step cap earlier builds wrote is dropped.
        assert!(root.pointer("/config/max_turns").is_none());
        // Written in the engine's shape.
        assert_eq!(
            root.pointer("/config/append_system_prompt").unwrap(),
            "Answer in Chinese."
        );
        assert_eq!(root.pointer("/config/mcp_servers/0/type").unwrap(), "stdio");
        assert_eq!(root.pointer("/config/mcp_servers/0/command").unwrap(), "npx");
        assert_eq!(root.pointer("/permissionRules").unwrap(), &json!([]));

        let reread = from_document(&root);
        assert_eq!(reread, settings);
    }

    #[test]
    fn a_blank_system_prompt_is_unset_rather_than_stored() {
        let mut root = json!({"config": {"append_system_prompt": "old"}});
        let settings = AgentSettings {
            append_system_prompt: Some("   ".into()),
            ..AgentSettings::default()
        };
        apply(&mut root, &settings).unwrap();
        assert!(root.pointer("/config/append_system_prompt").is_none());
    }

    #[test]
    fn an_mcp_server_needs_a_command_or_a_url() {
        let mut server = McpServer {
            name: "x".into(),
            transport: "stdio".into(),
            ..McpServer::default()
        };
        assert!(!server.is_usable());
        server.command = Some("npx".into());
        assert!(server.is_usable());
        server.transport = "http".into();
        assert!(!server.is_usable());
        server.url = Some("https://mcp.example".into());
        assert!(server.is_usable());
        assert_eq!(server.summary(), "https://mcp.example");
    }

    #[test]
    fn the_essential_tools_are_all_built_in() {
        for tool in ESSENTIAL_TOOLS {
            assert!(BUILTIN_TOOLS.contains(&tool), "{tool}");
        }
    }

    #[test]
    fn a_rule_row_names_what_it_applies_to() {
        let rule = PermissionRule {
            tool_name: Some("Bash".into()),
            path_pattern: Some("src/**".into()),
            action: "Deny".into(),
        };
        assert_eq!(rule.subject(), "Bash · src/**");
        assert!(!rule.is_allow());
    }
}
