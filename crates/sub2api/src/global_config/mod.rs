//! Routing by writing each CLI's own global configuration — the cc-switch
//! model.
//!
//! The previous mechanism injected environment at spawn and kept generated
//! config directories. It was invisible (users verify routing by reading
//! their CLI's config files), fragile against config layers that outrank the
//! process environment, and split session storage. This module does what
//! cc-switch and the Electron client do instead: **edit the live files**.
//!
//! Two modes, matching cc-switch's split:
//!
//! * **Switching mode** — Claude Code (`~/.claude/settings.json`), Codex
//!   (`~/.codex/auth.json` + `config.toml`), Grok (`~/.grok/config.toml`).
//!   Taking over captures a backup of each file first, then surgically edits
//!   only our keys (JSON deep-merge; TOML via `toml_edit`, preserving the
//!   user's comments and key order). Releasing restores our keys from the
//!   backup and leaves everything the user changed meanwhile intact.
//! * **Additive mode** — OpenCode (`opencode.json`) and Pi (`models.json`).
//!   We own exactly one provider entry named [`PROVIDER_ID`]; adding and
//!   removing it never touches the user's other entries. Pi's `auth.json`
//!   and `settings.json` are never read or written.
//!
//! The takeover ledger lives at `~/.cheaprouter/takeover.json`. Every write is
//! atomic (temp file + rename) and credential-bearing files are 0600 on
//! Unix. All paths are parameterized so tests never touch the real home.

pub mod claude;
pub mod codex;
pub mod grok;
pub mod opencode;
pub mod pi;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;
use crate::custom_api::CustomApiConfig;
use crate::gateway::GatewayConfig;

/// The provider entry name we own in additive configs and the Codex provider
/// table. One stable id, so upgrades and removals always find their entry.
pub const PROVIDER_ID: &str = "cheaprouter";

/// Where every file this module touches lives. Parameterized so tests build
/// the whole tree under a temp directory.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Our own state directory (`~/.cheaprouter`), holding the takeover ledger.
    pub data_dir: PathBuf,
    pub claude_dir: PathBuf,
    pub codex_dir: PathBuf,
    pub grok_dir: PathBuf,
    pub opencode_dir: PathBuf,
    pub pi_agent_dir: PathBuf,
}

impl Paths {
    /// The real locations, matching each CLI's own resolution rules.
    pub fn resolve() -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self {
            data_dir: brand::data_dir()?,
            claude_dir: home.join(".claude"),
            codex_dir: home.join(".codex"),
            grok_dir: home.join(".grok"),
            // OpenCode uses `~/.config/opencode` on every platform.
            opencode_dir: home.join(".config").join("opencode"),
            // Pi honours PI_CODING_AGENT_DIR, same as the CLI itself.
            pi_agent_dir: std::env::var_os("PI_CODING_AGENT_DIR")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| home.join(".pi").join("agent")),
        })
    }

    fn takeover_path(&self) -> PathBuf {
        self.data_dir.join("takeover.json")
    }
}

/// What one CLI should route through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteTarget {
    /// Service origin as entered; writers normalize (`/v1` etc.) per CLI.
    pub base_url: String,
    pub api_key: String,
    /// Model ids to advertise, for the CLIs whose config carries a model
    /// list (OpenCode, Pi). Empty means "none declared".
    pub models: Vec<String>,
}

/// The routing every CLI should end up with. `None` = leave alone / restore.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesiredRoutes {
    pub claude: Option<RouteTarget>,
    pub codex: Option<RouteTarget>,
    pub grok: Option<RouteTarget>,
    pub opencode: Option<RouteTarget>,
    pub pi: Option<RouteTarget>,
}

/// Fold the cloud gateway and the custom endpoints into one desired state.
///
/// Per CLI the cloud wins while it covers it (signed in, routing on, key
/// present); the custom endpoint applies otherwise. Grok rides the general
/// gateway key — the Electron client's managed login set Grok up the same
/// way. OpenCode and Pi are custom-only for now; their writers are generic,
/// so extending cloud coverage later is a matter of adding two lines here.
pub fn desired_routes(cloud: Option<&GatewayConfig>, custom: &CustomApiConfig) -> DesiredRoutes {
    let cloud = cloud.filter(|config| config.is_usable());
    let cloud_target = |key: Option<&str>| {
        let config = cloud?;
        let key = key?.trim();
        (!key.is_empty()).then(|| RouteTarget {
            base_url: config.endpoint.clone(),
            api_key: key.to_owned(),
            models: Vec::new(),
        })
    };
    let custom_target = |provider_id: &str| {
        custom.endpoint_for(provider_id).map(|endpoint| RouteTarget {
            base_url: endpoint.base_url.trim().to_owned(),
            api_key: endpoint.api_key.trim().to_owned(),
            models: endpoint.models.clone(),
        })
    };
    DesiredRoutes {
        claude: cloud_target(cloud.and_then(|config| config.key_for("claude")))
            .or_else(|| custom_target("claude")),
        codex: cloud_target(cloud.and_then(|config| config.key_for("codex")))
            .or_else(|| custom_target("codex")),
        grok: cloud_target(cloud.and_then(|config| config.api_key.as_deref()))
            .or_else(|| custom_target("grok")),
        opencode: custom_target("opencode"),
        pi: custom_target("pi"),
    }
}

/// Which configuration a CLI runs with right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteKind {
    /// The managed gateway covers it: signed in, routing on, key present.
    Cloud,
    /// A usable custom endpoint is set and nothing outranks it.
    Custom,
    /// Neither — the CLI's own configuration, untouched (or restored).
    CliOwn,
}

/// The route a CLI is on, for the Providers page. Same precedence as
/// [`desired_routes`]: the cloud wins while it covers the CLI, the custom
/// endpoint applies otherwise. A custom entry the cloud outranks is still
/// stored, which is why the page says so next to it.
pub fn active_route_kind(
    provider_id: &str,
    cloud: Option<&GatewayConfig>,
    custom: &CustomApiConfig,
) -> RouteKind {
    let cloud_only = desired_routes(cloud, &CustomApiConfig::default());
    let cloud_covers = match provider_id {
        "claude" => cloud_only.claude.is_some(),
        "codex" => cloud_only.codex.is_some(),
        "grok" => cloud_only.grok.is_some(),
        _ => false,
    };
    if cloud_covers {
        RouteKind::Cloud
    } else if custom.endpoint_for(provider_id).is_some() {
        RouteKind::Custom
    } else {
        RouteKind::CliOwn
    }
}

/// A file as it was the moment we first took a CLI over.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileBackup {
    pub existed: bool,
    #[serde(default)]
    pub content: String,
}

/// Backups for one switching-mode CLI, keyed by file name.
pub type CliBackups = BTreeMap<String, FileBackup>;

/// The takeover ledger: which CLIs we currently manage, and what their
/// files looked like before we did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverState {
    #[serde(default)]
    pub claude: Option<CliBackups>,
    #[serde(default)]
    pub codex: Option<CliBackups>,
    #[serde(default)]
    pub grok: Option<CliBackups>,
    #[serde(default)]
    pub opencode: bool,
    #[serde(default)]
    pub pi: bool,
}

pub fn load_state(paths: &Paths) -> TakeoverState {
    std::fs::read_to_string(paths.takeover_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(paths: &Paths, state: &TakeoverState) -> Result<()> {
    let encoded = serde_json::to_string_pretty(state).context("could not encode takeover state")?;
    atomic_write_private(&paths.takeover_path(), encoded.as_bytes())
}

/// Drive every CLI's live configuration to `desired`, at the real paths.
///
/// Returns per-CLI warnings; a CLI that cannot be written (unparseable file,
/// permissions) is skipped with a warning rather than failing the rest.
pub fn reconcile(desired: &DesiredRoutes) -> Result<Vec<String>> {
    let paths = Paths::resolve().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let warnings = reconcile_at(&paths, desired)?;
    cleanup_legacy_generated_dirs(&paths.data_dir);
    Ok(warnings)
}

/// [`reconcile`] against explicit paths.
pub fn reconcile_at(paths: &Paths, desired: &DesiredRoutes) -> Result<Vec<String>> {
    let mut state = load_state(paths);
    let mut warnings = Vec::new();

    // Switching-mode CLIs: take over (capturing backups once) or restore.
    reconcile_switching(
        "claude",
        desired.claude.as_ref(),
        &mut state.claude,
        &mut warnings,
        |target, backups| claude::take_over(&paths.claude_dir, target, backups),
        |backups| claude::restore(&paths.claude_dir, backups),
    );
    reconcile_switching(
        "codex",
        desired.codex.as_ref(),
        &mut state.codex,
        &mut warnings,
        |target, backups| codex::take_over(&paths.codex_dir, target, backups),
        |backups| codex::restore(&paths.codex_dir, backups),
    );
    reconcile_switching(
        "grok",
        desired.grok.as_ref(),
        &mut state.grok,
        &mut warnings,
        |target, backups| grok::take_over(&paths.grok_dir, target, backups),
        |backups| grok::restore(&paths.grok_dir, backups),
    );

    // Additive-mode CLIs: insert or remove our single provider entry.
    match desired.opencode.as_ref() {
        Some(target) => match opencode::take_over(&paths.opencode_dir, target) {
            Ok(()) => state.opencode = true,
            Err(error) => warnings.push(format!("opencode: {error:#}")),
        },
        None if state.opencode => match opencode::remove(&paths.opencode_dir) {
            Ok(()) => state.opencode = false,
            Err(error) => warnings.push(format!("opencode: {error:#}")),
        },
        None => {}
    }
    match desired.pi.as_ref() {
        Some(target) => match pi::take_over(&paths.pi_agent_dir, target) {
            Ok(()) => state.pi = true,
            Err(error) => warnings.push(format!("pi: {error:#}")),
        },
        None if state.pi => match pi::remove(&paths.pi_agent_dir) {
            Ok(()) => state.pi = false,
            Err(error) => warnings.push(format!("pi: {error:#}")),
        },
        None => {}
    }

    save_state(paths, &state)?;
    Ok(warnings)
}

/// One switching-mode CLI's reconcile step. On a failed restore the backups
/// are kept so a later attempt can still put the user's file back.
fn reconcile_switching(
    cli: &str,
    target: Option<&RouteTarget>,
    slot: &mut Option<CliBackups>,
    warnings: &mut Vec<String>,
    take_over: impl FnOnce(&RouteTarget, &mut CliBackups) -> Result<()>,
    restore: impl FnOnce(&CliBackups) -> Result<()>,
) {
    match target {
        Some(target) => {
            let backups = slot.get_or_insert_with(BTreeMap::new);
            if let Err(error) = take_over(target, backups) {
                warnings.push(format!("{cli}: {error:#}"));
            }
        }
        None => {
            if let Some(backups) = slot.take() {
                if let Err(error) = restore(&backups) {
                    warnings.push(format!("{cli}: {error:#}"));
                    *slot = Some(backups);
                }
            }
        }
    }
}

/// The live configuration file routing writes for `provider_id` — what a
/// user opens to verify a save took.
pub fn config_file_for(provider_id: &str) -> Option<PathBuf> {
    let paths = Paths::resolve()?;
    Some(match provider_id {
        "claude" => paths.claude_dir.join("settings.json"),
        "codex" => paths.codex_dir.join("config.toml"),
        "grok" => paths.grok_dir.join("config.toml"),
        "opencode" => paths.opencode_dir.join("opencode.json"),
        "pi" => paths.pi_agent_dir.join("models.json"),
        _ => return None,
    })
}

/// The injection era generated config directories under our data dir; they
/// are dead weight now and their stale credentials should not linger.
fn cleanup_legacy_generated_dirs(data_dir: &Path) {
    for legacy in ["gateway", "custom"] {
        let _ = std::fs::remove_dir_all(data_dir.join(legacy));
    }
}

/// Record `path`'s current content into `backups` under `name`, once. Later
/// takeovers (group switches) must not overwrite the pre-takeover original.
pub(crate) fn capture_backup(backups: &mut CliBackups, name: &str, path: &Path) -> Result<()> {
    if backups.contains_key(name) {
        return Ok(());
    }
    let backup = match std::fs::read_to_string(path) {
        Ok(content) => FileBackup {
            existed: true,
            content,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileBackup::default(),
        Err(error) => {
            return Err(error).with_context(|| format!("could not back up {}", path.display()));
        }
    };
    backups.insert(name.to_owned(), backup);
    Ok(())
}

/// Write atomically: temp file in the same directory, then rename over the
/// target. Credential-bearing files get 0600 on Unix.
pub(crate) fn atomic_write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", parent.display()))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, contents)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&temporary, path).or_else(|_| {
        // Windows rename over an existing read-locked file can fail; fall
        // back to remove + rename, and clean the temp file up on failure.
        let fallback = std::fs::remove_file(path).and_then(|_| std::fs::rename(&temporary, path));
        if fallback.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        fallback.with_context(|| format!("could not replace {}", path.display()))
    })?;
    Ok(())
}

/// Set a TOML key, replacing an existing value **in place** so the key's
/// decor — the user's comment sitting right above it — survives.
/// `Table::insert` would mint a fresh key and drop that comment.
pub(crate) fn set_toml_value(table: &mut toml_edit::Table, key: &str, item: toml_edit::Item) {
    match table.get_mut(key) {
        Some(existing) => *existing = item,
        None => {
            table.insert(key, item);
        }
    }
}

/// Remove `path` if it exists; absent is success.
pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("could not remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_route_kind_prefers_cloud_over_custom() {
        let cloud = GatewayConfig {
            enabled: true,
            endpoint: "https://cloud.example.org".into(),
            api_key: Some("sk-general".into()),
            claude_api_key: Some("sk-claude".into()),
            codex_api_key: None,
            codex_model: None,
        };
        let mut custom = CustomApiConfig::default();
        custom.set(
            "claude",
            Some(crate::custom_api::CustomEndpoint {
                base_url: "https://mine.example.org".into(),
                api_key: "sk-mine".into(),
                models: Vec::new(),
            }),
        );
        custom.set(
            "codex",
            Some(crate::custom_api::CustomEndpoint {
                base_url: "https://mine.example.org".into(),
                api_key: "sk-mine".into(),
                models: Vec::new(),
            }),
        );

        // Claude: the cloud key outranks the stored custom endpoint.
        assert_eq!(
            active_route_kind("claude", Some(&cloud), &custom),
            RouteKind::Cloud
        );
        // Codex and Grok ride the general key when they have none of their own.
        assert_eq!(
            active_route_kind("codex", Some(&cloud), &custom),
            RouteKind::Cloud
        );
        assert_eq!(
            active_route_kind("grok", Some(&cloud), &custom),
            RouteKind::Cloud
        );
        // Without a general key, Codex falls back to its custom endpoint and
        // Grok has nothing.
        let claude_only = GatewayConfig {
            api_key: None,
            ..cloud.clone()
        };
        assert_eq!(
            active_route_kind("codex", Some(&claude_only), &custom),
            RouteKind::Custom
        );
        assert_eq!(
            active_route_kind("grok", Some(&claude_only), &custom),
            RouteKind::CliOwn
        );
        // Nothing configured at all.
        assert_eq!(
            active_route_kind("pi", Some(&cloud), &custom),
            RouteKind::CliOwn
        );
        // Signed out: the custom endpoint takes over for Claude.
        assert_eq!(
            active_route_kind("claude", None, &custom),
            RouteKind::Custom
        );
        let disabled = GatewayConfig {
            enabled: false,
            ..cloud
        };
        assert_eq!(
            active_route_kind("grok", Some(&disabled), &custom),
            RouteKind::CliOwn
        );
    }

    fn target(url: &str, key: &str) -> RouteTarget {
        RouteTarget {
            base_url: url.to_owned(),
            api_key: key.to_owned(),
            models: Vec::new(),
        }
    }

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "sub2api-gc-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Paths {
            data_dir: root.join("waku"),
            claude_dir: root.join("claude"),
            codex_dir: root.join("codex"),
            grok_dir: root.join("grok"),
            opencode_dir: root.join("opencode"),
            pi_agent_dir: root.join("pi-agent"),
        }
    }

    fn cleanup(paths: &Paths) {
        if let Some(root) = paths.data_dir.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn cloud_wins_over_custom_and_grok_rides_the_general_key() {
        let cloud = GatewayConfig {
            enabled: true,
            endpoint: "https://cloud.example.org".into(),
            api_key: Some("sk-general".into()),
            claude_api_key: Some("sk-claude".into()),
            codex_api_key: None,
            codex_model: None,
        };
        let mut custom = CustomApiConfig::default();
        custom.set(
            "claude",
            Some(crate::custom_api::CustomEndpoint {
                base_url: "https://relay.example.org".into(),
                api_key: "sk-custom".into(),
                models: Vec::new(),
            }),
        );
        custom.set(
            "pi",
            Some(crate::custom_api::CustomEndpoint {
                base_url: "https://relay.example.org".into(),
                api_key: "sk-pi".into(),
                models: vec!["m1".into()],
            }),
        );

        let desired = desired_routes(Some(&cloud), &custom);
        // Cloud claude key beats the custom claude endpoint.
        assert_eq!(desired.claude.as_ref().unwrap().api_key, "sk-claude");
        // Codex falls back to the general key via the cloud.
        assert_eq!(desired.codex.as_ref().unwrap().api_key, "sk-general");
        // Grok rides the general key too.
        assert_eq!(desired.grok.as_ref().unwrap().api_key, "sk-general");
        // Pi is custom-only, with its model list carried through.
        assert_eq!(desired.pi.as_ref().unwrap().models, vec!["m1".to_owned()]);
        assert!(desired.opencode.is_none());

        // Signed out: customs apply where set.
        let desired = desired_routes(None, &custom);
        assert_eq!(desired.claude.as_ref().unwrap().api_key, "sk-custom");
        assert!(desired.codex.is_none());
        assert!(desired.grok.is_none());
    }

    #[test]
    fn reconcile_round_trips_every_cli_and_is_idempotent() {
        let paths = temp_paths("roundtrip");
        // Pre-existing user files with content that must survive.
        std::fs::create_dir_all(&paths.claude_dir).unwrap();
        std::fs::write(
            paths.claude_dir.join("settings.json"),
            r#"{"permissions":{"allow":["Bash"]},"env":{"FOO":"bar"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(&paths.codex_dir).unwrap();
        std::fs::write(
            paths.codex_dir.join("config.toml"),
            "# my note\nmodel = \"my-model\"\n\n[mcp_servers.files]\ncommand = \"fs\"\n",
        )
        .unwrap();
        std::fs::write(
            paths.codex_dir.join("auth.json"),
            r#"{"tokens":{"access_token":"chatgpt-oauth"}}"#,
        )
        .unwrap();

        let desired = DesiredRoutes {
            claude: Some(target("https://gw.example.org", "sk-c")),
            codex: Some(target("https://gw.example.org", "sk-x")),
            grok: Some(target("https://gw.example.org", "sk-g")),
            opencode: Some(target("https://gw.example.org", "sk-o")),
            pi: Some(target("https://gw.example.org", "sk-p")),
        };
        let warnings = reconcile_at(&paths, &desired).expect("reconcile");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        // Idempotent: a second pass changes nothing and warns nothing.
        let warnings = reconcile_at(&paths, &desired).expect("reconcile again");
        assert!(warnings.is_empty());

        // Spot-check each file is managed.
        let claude_raw =
            std::fs::read_to_string(paths.claude_dir.join("settings.json")).unwrap();
        assert!(claude_raw.contains("https://gw.example.org"));
        assert!(claude_raw.contains(r#""FOO": "bar""#));
        let codex_raw = std::fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(codex_raw.contains("# my note"));
        assert!(codex_raw.contains(&format!("[model_providers.{PROVIDER_ID}]")));
        assert!(codex_raw.contains("[mcp_servers.files]"));
        assert!(
            std::fs::read_to_string(paths.grok_dir.join("config.toml"))
                .unwrap()
                .contains("sk-g")
        );
        assert!(
            std::fs::read_to_string(paths.opencode_dir.join("opencode.json"))
                .unwrap()
                .contains(PROVIDER_ID)
        );
        assert!(
            std::fs::read_to_string(paths.pi_agent_dir.join("models.json"))
                .unwrap()
                .contains(PROVIDER_ID)
        );

        // Release everything: files return to their pre-takeover state.
        let warnings = reconcile_at(&paths, &DesiredRoutes::default()).expect("restore");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let claude_restored: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(paths.claude_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(claude_restored["env"]["FOO"], "bar");
        assert!(claude_restored["env"].get("ANTHROPIC_BASE_URL").is_none());
        let codex_restored =
            std::fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(codex_restored.contains("# my note"));
        assert!(codex_restored.contains("model = \"my-model\""));
        assert!(!codex_restored.contains(PROVIDER_ID));
        assert_eq!(
            std::fs::read_to_string(paths.codex_dir.join("auth.json")).unwrap(),
            r#"{"tokens":{"access_token":"chatgpt-oauth"}}"#
        );
        // Files we created from nothing are gone again.
        assert!(!paths.grok_dir.join("config.toml").exists());
        assert!(
            !std::fs::read_to_string(paths.opencode_dir.join("opencode.json"))
                .unwrap()
                .contains(PROVIDER_ID)
        );
        assert!(
            !std::fs::read_to_string(paths.pi_agent_dir.join("models.json"))
                .unwrap()
                .contains(PROVIDER_ID)
        );
        // Restore with nothing managed is a no-op.
        let warnings = reconcile_at(&paths, &DesiredRoutes::default()).expect("noop");
        assert!(warnings.is_empty());
        cleanup(&paths);
    }
}
