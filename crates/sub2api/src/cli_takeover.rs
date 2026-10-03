//! Which CLIs the signed-in cloud account may configure.
//!
//! Signing in writes the gateway into each CLI's own global configuration
//! ([`crate::global_config`]). Some users already run Claude Code on their
//! own Claude subscription, or Codex on their ChatGPT sign-in, and want to
//! keep that — while still using the account for the built-in agent and
//! everything else. This module holds that choice per CLI and finds the
//! setups worth asking about at sign-in.
//!
//! The choice only ever filters the **cloud** source: an endpoint the user
//! explicitly bound to a CLI on the Model providers page is their own
//! instruction and still applies. The built-in agent is not a CLI here and
//! always follows the account.
//!
//! The file lives beside the other preferences (`~/.cheaprouter`), not in
//! the credential file, because signing out clears that one and the choice
//! has to outlive a session.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brand;
use crate::global_config::{self, CliBackups, Paths, atomic_write_private};

/// The CLIs the cloud account would take over, in display order.
pub const TAKEOVER_CLIS: [&str; 3] = ["claude", "codex", "grok"];

/// What the cloud account does with one CLI's configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliMode {
    /// Write the gateway into its configuration while signed in.
    Managed,
    /// Leave it alone: it keeps the user's own sign-in.
    Own,
}

/// The choice for every CLI. A CLI missing here has not been asked and is
/// managed, which is how every build before this one behaved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliTakeoverPrefs {
    #[serde(default)]
    pub clis: BTreeMap<String, CliMode>,
}

impl CliTakeoverPrefs {
    /// The recorded choice, or `None` when the user was never asked.
    pub fn mode(&self, cli: &str) -> Option<CliMode> {
        self.clis.get(cli).copied()
    }

    /// Only an explicit "keep my own" counts.
    pub fn keeps_own(&self, cli: &str) -> bool {
        self.mode(cli) == Some(CliMode::Own)
    }

    /// The CLIs the account must not configure.
    pub fn own_set(&self) -> BTreeSet<String> {
        self.clis
            .iter()
            .filter(|(_, mode)| **mode == CliMode::Own)
            .map(|(cli, _)| cli.clone())
            .collect()
    }

    pub fn set(&mut self, cli: &str, mode: CliMode) {
        self.clis.insert(cli.to_owned(), mode);
    }
}

/// Where the choice lives.
pub fn config_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("cli-takeover.json"))
}

/// Load the choice; absent or unreadable means "every CLI managed".
pub fn load() -> CliTakeoverPrefs {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(prefs: &CliTakeoverPrefs) -> Result<()> {
    let path = config_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut encoded =
        serde_json::to_string_pretty(prefs).context("could not encode CLI takeover settings")?;
    encoded.push('\n');
    atomic_write_private(&path, encoded.as_bytes())
}

/// Why a CLI looks like it already runs on the user's own account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnSetupReason {
    /// Signed in with the CLI's own account: Claude's OAuth login, Codex's
    /// ChatGPT sign-in.
    SignedIn,
    /// An API key of the user's own (`ANTHROPIC_API_KEY`, `apiKeyHelper`,
    /// Codex's `OPENAI_API_KEY`).
    ApiKey,
    /// Pointed at another provider or relay in its configuration.
    CustomProvider,
}

/// A CLI that already has a setup of the user's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnSetup {
    pub cli: &'static str,
    pub reason: OwnSetupReason,
    /// The signed-in account, when the CLI records one.
    pub account: Option<String>,
}

/// Find the CLIs that already run on the user's own account or provider.
///
/// Read-only, and an unreadable file counts as nothing found. The files the
/// takeover rewrites (`settings.json`, `config.toml`) are read from the
/// takeover backup when one exists, so our own keys are never mistaken for
/// the user's.
pub fn detect_own_setup(paths: &Paths) -> Vec<OwnSetup> {
    let state = global_config::load_state(paths);
    [
        detect_claude(paths, state.claude.as_ref()),
        detect_codex(paths, state.codex.as_ref()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn detect_claude(paths: &Paths, backups: Option<&CliBackups>) -> Option<OwnSetup> {
    let setup = |reason, account| {
        Some(OwnSetup {
            cli: "claude",
            reason,
            account,
        })
    };
    // `~/.claude.json` sits beside the config directory, not inside it. Its
    // `oauthAccount` is the one sign-in record every platform keeps — macOS
    // holds the tokens themselves in the keychain.
    let account = paths
        .claude_dir
        .parent()
        .and_then(|home| read_json(&home.join(".claude.json")))
        .and_then(|root| root.get("oauthAccount").cloned())
        .filter(Value::is_object);
    if let Some(account) = account {
        let email = account
            .get("emailAddress")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|email| !email.is_empty())
            .map(str::to_owned);
        return setup(OwnSetupReason::SignedIn, email);
    }
    if paths.claude_dir.join(".credentials.json").is_file() {
        return setup(OwnSetupReason::SignedIn, None);
    }

    let settings = effective_content(backups, "settings.json", &paths.claude_dir.join("settings.json"))
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())?;
    if non_empty_str(settings.get("apiKeyHelper")) {
        return setup(OwnSetupReason::ApiKey, None);
    }
    let env = settings.get("env").and_then(Value::as_object)?;
    let set = |name: &str| non_empty_str(env.get(name));
    if set("ANTHROPIC_BASE_URL")
        || set("CLAUDE_CODE_USE_BEDROCK")
        || set("CLAUDE_CODE_USE_VERTEX")
    {
        return setup(OwnSetupReason::CustomProvider, None);
    }
    if set("ANTHROPIC_API_KEY") || set("ANTHROPIC_AUTH_TOKEN") {
        return setup(OwnSetupReason::ApiKey, None);
    }
    None
}

fn detect_codex(paths: &Paths, backups: Option<&CliBackups>) -> Option<OwnSetup> {
    let setup = |reason, account| {
        Some(OwnSetup {
            cli: "codex",
            reason,
            account,
        })
    };
    // Earlier builds replaced `auth.json` and kept the user's original in
    // the ledger; that original is the one that says who they are.
    let auth = effective_content(backups, "auth.json", &paths.codex_dir.join("auth.json"))
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    if let Some(auth) = auth {
        if let Some(tokens) = auth.get("tokens").filter(|tokens| tokens.is_object()) {
            let email = tokens
                .get("id_token")
                .and_then(Value::as_str)
                .and_then(jwt_email);
            return setup(OwnSetupReason::SignedIn, email);
        }
        if non_empty_str(auth.get("OPENAI_API_KEY")) {
            return setup(OwnSetupReason::ApiKey, None);
        }
    }

    let config = effective_content(backups, "config.toml", &paths.codex_dir.join("config.toml"))
        .and_then(|raw| raw.parse::<toml_edit::DocumentMut>().ok())?;
    let provider = config
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|provider| !provider.is_empty())?;
    let declared = config
        .get("model_providers")
        .and_then(|item| item.as_table_like())
        .and_then(|providers| providers.get(provider))
        .and_then(|item| item.as_table_like())
        .is_some_and(|table| table.get("base_url").is_some());
    declared.then(|| OwnSetup {
        cli: "codex",
        reason: OwnSetupReason::CustomProvider,
        account: None,
    })
}

/// The file as the user left it: the takeover backup while one is held,
/// the live file otherwise. `None` when it does not exist (or did not
/// before we took it over).
fn effective_content(backups: Option<&CliBackups>, name: &str, path: &Path) -> Option<String> {
    match backups.and_then(|backups| backups.get(name)) {
        Some(backup) => backup.existed.then(|| backup.content.clone()),
        None => std::fs::read_to_string(path).ok(),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

fn non_empty_str(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

/// The `email` claim of a JWT, for display only — nothing is verified.
fn jwt_email(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    claims
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_config::FileBackup;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "sub2api-takeover-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Paths {
            data_dir: root.join("data"),
            native_dir: root.join("claurst"),
            claude_dir: root.join(".claude"),
            codex_dir: root.join(".codex"),
            grok_dir: root.join(".grok"),
            opencode_dir: root.join("opencode"),
            pi_agent_dir: root.join("pi-agent"),
        }
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn cleanup(paths: &Paths) {
        if let Some(root) = paths.data_dir.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    fn record_backup(paths: &Paths, cli: &str, name: &str, content: Option<&str>) {
        let backups = CliBackups::from([(
            name.to_owned(),
            FileBackup {
                existed: content.is_some(),
                content: content.unwrap_or_default().to_owned(),
            },
        )]);
        let mut state = global_config::TakeoverState::default();
        match cli {
            "claude" => state.claude = Some(backups),
            "codex" => state.codex = Some(backups),
            _ => unreachable!(),
        }
        write(
            &paths.data_dir.join("takeover.json"),
            &serde_json::to_string(&state).unwrap(),
        );
    }

    fn reasons(paths: &Paths) -> Vec<(&'static str, OwnSetupReason, Option<String>)> {
        detect_own_setup(paths)
            .into_iter()
            .map(|setup| (setup.cli, setup.reason, setup.account))
            .collect()
    }

    #[test]
    fn absent_prefs_mean_managed_and_round_trip() {
        let mut prefs = CliTakeoverPrefs::default();
        assert_eq!(prefs.mode("claude"), None);
        assert!(!prefs.keeps_own("claude"));
        prefs.set("claude", CliMode::Own);
        prefs.set("codex", CliMode::Managed);
        assert!(prefs.keeps_own("claude"));
        assert!(!prefs.keeps_own("codex"));
        assert_eq!(prefs.own_set(), BTreeSet::from(["claude".to_owned()]));

        let encoded = serde_json::to_string(&prefs).unwrap();
        assert!(encoded.contains(r#""claude":"own""#), "{encoded}");
        assert_eq!(serde_json::from_str::<CliTakeoverPrefs>(&encoded).unwrap(), prefs);
        assert_eq!(
            serde_json::from_str::<CliTakeoverPrefs>("{}").unwrap(),
            CliTakeoverPrefs::default()
        );
    }

    #[test]
    fn nothing_found_on_a_clean_machine() {
        let paths = temp_paths("clean");
        assert!(detect_own_setup(&paths).is_empty());
        // Files without anything of the user's own count as nothing too.
        write(&paths.claude_dir.join("settings.json"), r#"{"env":{"FOO":"1"}}"#);
        write(&paths.codex_dir.join("config.toml"), "model = \"gpt-5\"\n");
        write(&paths.codex_dir.join("auth.json"), r#"{"OPENAI_API_KEY":null}"#);
        assert!(detect_own_setup(&paths).is_empty());
        cleanup(&paths);
    }

    #[test]
    fn claude_oauth_login_is_found_with_its_email() {
        let paths = temp_paths("claude-oauth");
        write(
            &paths.claude_dir.parent().unwrap().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"me@example.org"}}"#,
        );
        assert_eq!(
            reasons(&paths),
            [("claude", OwnSetupReason::SignedIn, Some("me@example.org".to_owned()))]
        );
        cleanup(&paths);

        let paths = temp_paths("claude-credentials");
        write(&paths.claude_dir.join(".credentials.json"), "{}");
        assert_eq!(reasons(&paths), [("claude", OwnSetupReason::SignedIn, None)]);
        cleanup(&paths);
    }

    #[test]
    fn claude_own_keys_and_relays_are_found() {
        let paths = temp_paths("claude-relay");
        write(
            &paths.claude_dir.join("settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.org"}}"#,
        );
        assert_eq!(
            reasons(&paths),
            [("claude", OwnSetupReason::CustomProvider, None)]
        );
        write(&paths.claude_dir.join("settings.json"), r#"{"apiKeyHelper":"~/key.sh"}"#);
        assert_eq!(reasons(&paths), [("claude", OwnSetupReason::ApiKey, None)]);
        write(
            &paths.claude_dir.join("settings.json"),
            r#"{"env":{"ANTHROPIC_API_KEY":"sk-ant-mine"}}"#,
        );
        assert_eq!(reasons(&paths), [("claude", OwnSetupReason::ApiKey, None)]);
        cleanup(&paths);
    }

    /// While taken over, `settings.json` holds our gateway keys; what the
    /// user had is the backup, and only that may be judged.
    #[test]
    fn a_taken_over_file_is_judged_by_its_backup() {
        let paths = temp_paths("claude-backup");
        write(
            &paths.claude_dir.join("settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway","ANTHROPIC_AUTH_TOKEN":"sk-ours"}}"#,
        );
        record_backup(&paths, "claude", "settings.json", None);
        assert!(detect_own_setup(&paths).is_empty());

        record_backup(
            &paths,
            "claude",
            "settings.json",
            Some(r#"{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.org"}}"#),
        );
        assert_eq!(
            reasons(&paths),
            [("claude", OwnSetupReason::CustomProvider, None)]
        );
        cleanup(&paths);
    }

    #[test]
    fn codex_chatgpt_login_is_found_with_its_email() {
        let paths = temp_paths("codex-chatgpt");
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"email":"me@example.org"}"#);
        write(
            &paths.codex_dir.join("auth.json"),
            &format!(
                r#"{{"OPENAI_API_KEY":null,"tokens":{{"id_token":"h.{claims}.s","access_token":"a"}}}}"#
            ),
        );
        assert_eq!(
            reasons(&paths),
            [("codex", OwnSetupReason::SignedIn, Some("me@example.org".to_owned()))]
        );
        write(&paths.codex_dir.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-mine"}"#);
        assert_eq!(reasons(&paths), [("codex", OwnSetupReason::ApiKey, None)]);
        cleanup(&paths);
    }

    #[test]
    fn codex_custom_provider_is_found() {
        let paths = temp_paths("codex-provider");
        write(
            &paths.codex_dir.join("config.toml"),
            "model_provider = \"relay\"\n\n[model_providers.relay]\nbase_url = \"https://relay.example.org/v1\"\n",
        );
        assert_eq!(
            reasons(&paths),
            [("codex", OwnSetupReason::CustomProvider, None)]
        );
        // A provider name with no table behind it is Codex's built-in one.
        write(&paths.codex_dir.join("config.toml"), "model_provider = \"openai\"\n");
        assert!(detect_own_setup(&paths).is_empty());

        // Taken over: the live file is ours, the backup had nothing.
        write(
            &paths.codex_dir.join("config.toml"),
            "model_provider = \"OpenAI\"\n\n[model_providers.OpenAI]\nbase_url = \"https://gateway\"\n",
        );
        record_backup(&paths, "codex", "config.toml", None);
        assert!(detect_own_setup(&paths).is_empty());
        cleanup(&paths);
    }
}
