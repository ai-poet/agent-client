//! Persist the built-in agent's model routes in its own settings file.
//!
//! External agent CLIs are not configured or launched by this build. The
//! native writer keeps the original backup/restore behavior for the built-in
//! agent's settings, and all filesystem writes are atomic.

pub mod native;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;
use crate::custom_api::{CustomApiConfig, NATIVE_SLOTS};
use crate::gateway::GatewayConfig;

/// Files used by the built-in agent's routing writer. Parameterized so tests
/// never touch the real home directory.
#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub native_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Option<Self> {
        Some(Self {
            data_dir: brand::data_dir()?,
            native_dir: native::config_dir()?,
        })
    }

    fn takeover_path(&self) -> PathBuf {
        self.data_dir.join("takeover.json")
    }
}

/// One API route used by the built-in agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteTarget {
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub managed_gateway: bool,
}

/// The built-in agent's routing: one line per API format it can speak.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeRoutes {
    pub messages: Option<RouteTarget>,
    pub responses: Option<RouteTarget>,
    pub chat: Option<RouteTarget>,
    /// Gateway keys by platform. Empty whenever a user's own endpoint is in
    /// use, so a gateway key can never be sent to that endpoint.
    pub platform_keys: BTreeMap<String, String>,
    pub model_keys: BTreeMap<String, String>,
    pub model_platforms: BTreeMap<String, String>,
    pub context_windows: BTreeMap<String, u64>,
    /// User-declared endpoints whose models are offered to the built-in agent.
    pub endpoints: BTreeMap<String, EndpointRoute>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointRoute {
    pub format: crate::providers::ApiFormat,
    pub base_url: String,
    pub api_key: String,
    pub context_windows: BTreeMap<String, u64>,
}

impl NativeRoutes {
    pub fn is_empty(&self) -> bool {
        self.messages.is_none()
            && self.responses.is_none()
            && self.chat.is_none()
            && self.endpoints.is_empty()
    }

    pub fn target(&self, engine_provider: &str) -> Option<&RouteTarget> {
        match engine_provider {
            "anthropic" => self.messages.as_ref(),
            "codex" => self.responses.as_ref(),
            "openai" => self.chat.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesiredRoutes {
    pub native: Option<NativeRoutes>,
}

/// Fold the managed gateway and the user's endpoints into native routes.
/// An explicitly bound custom format wins; unbound formats fall back to the
/// selected gateway origin.
pub fn desired_routes(cloud: Option<&GatewayConfig>, custom: &CustomApiConfig) -> DesiredRoutes {
    let cloud = cloud.filter(|config| config.is_usable());
    let cloud_target = |key: Option<&str>| {
        let config = cloud?;
        let key = key?.trim();
        (!key.is_empty()).then(|| RouteTarget {
            base_url: config.endpoint.clone(),
            api_key: key.to_owned(),
            models: Vec::new(),
            managed_gateway: true,
        })
    };
    let custom_target = |provider_id: &str| {
        custom
            .routed_endpoint(provider_id)
            .map(|endpoint| RouteTarget {
                base_url: endpoint.base_url.trim().to_owned(),
                api_key: endpoint.api_key.trim().to_owned(),
                models: endpoint.models.clone(),
                managed_gateway: false,
            })
    };
    let any_custom = NATIVE_SLOTS
        .into_iter()
        .any(|slot| custom.routed_endpoint(slot).is_some());
    let mut platform_keys = BTreeMap::new();
    let mut model_keys = BTreeMap::new();
    let mut model_platforms = BTreeMap::new();
    if let Some(config) = cloud.filter(|_| !any_custom) {
        model_keys = config
            .model_keys
            .iter()
            .filter(|(_, key)| !key.trim().is_empty())
            .map(|(model, key)| (model.clone(), key.trim().to_owned()))
            .collect();
        model_platforms = config
            .model_platforms
            .iter()
            .filter(|(model, _)| model_keys.contains_key(*model))
            .map(|(model, platform)| (model.clone(), platform.clone()))
            .collect();
        for (platform, key) in [
            ("anthropic", config.key_for("claude")),
            ("openai", config.key_for("codex")),
            ("default", config.api_key.as_deref()),
        ] {
            if let Some(key) = key.map(str::trim).filter(|key| !key.is_empty()) {
                platform_keys.insert(platform.to_owned(), key.to_owned());
            }
        }
    }
    let routes = NativeRoutes {
        messages: custom_target("native_messages")
            .or_else(|| cloud_target(cloud.and_then(|config| config.key_for("claude")))),
        responses: custom_target("native_responses")
            .or_else(|| cloud_target(cloud.and_then(|config| config.key_for("codex")))),
        chat: custom_target("native_chat")
            .or_else(|| cloud_target(cloud.and_then(|config| config.api_key.as_deref()))),
        platform_keys,
        model_keys,
        model_platforms,
        context_windows: native_context_windows(cloud, custom),
        endpoints: native_endpoints(custom),
    };
    DesiredRoutes {
        native: (!routes.is_empty()).then_some(routes),
    }
}

fn native_endpoints(custom: &CustomApiConfig) -> BTreeMap<String, EndpointRoute> {
    custom
        .registry
        .providers
        .iter()
        .filter(|entry| entry.offers_models_to_agent())
        .map(|entry| {
            let context_windows = entry
                .models
                .iter()
                .filter_map(|model| {
                    let id = model.id.trim();
                    let window = model.context_window.filter(|window| *window > 0)?;
                    (!id.is_empty()).then(|| (id.to_owned(), u64::from(window)))
                })
                .collect();
            (
                entry.id.clone(),
                EndpointRoute {
                    format: entry.format,
                    base_url: entry.base_url.trim().to_owned(),
                    api_key: entry.api_key.trim().to_owned(),
                    context_windows,
                },
            )
        })
        .collect()
}

fn native_context_windows(
    cloud: Option<&GatewayConfig>,
    custom: &CustomApiConfig,
) -> BTreeMap<String, u64> {
    let mut windows = cloud
        .map(|config| config.model_windows.clone())
        .unwrap_or_default();
    for slot in NATIVE_SLOTS {
        if custom.routed_endpoint(slot).is_none() {
            continue;
        }
        let Some(entry) = custom.bound_provider(slot) else {
            continue;
        };
        for model in &entry.models {
            let id = model.id.trim();
            if let Some(window) = model.context_window.filter(|window| *window > 0)
                && !id.is_empty()
            {
                windows.insert(id.to_owned(), u64::from(window));
            }
        }
    }
    windows
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteKind {
    Cloud,
    Custom,
    /// No managed or custom route is bound to this native API format.
    CliOwn,
}

pub fn active_route_kind(
    provider_id: &str,
    cloud: Option<&GatewayConfig>,
    custom: &CustomApiConfig,
) -> RouteKind {
    if !NATIVE_SLOTS.contains(&provider_id) {
        return RouteKind::CliOwn;
    }
    if custom.routed_endpoint(provider_id).is_some() {
        return RouteKind::Custom;
    }
    let cloud_only = desired_routes(cloud, &CustomApiConfig::default());
    let covered = cloud_only
        .native
        .as_ref()
        .is_some_and(|routes| match provider_id {
            "native_messages" => routes.messages.is_some(),
            "native_responses" => routes.responses.is_some(),
            _ => routes.chat.is_some(),
        });
    if covered {
        RouteKind::Cloud
    } else {
        RouteKind::CliOwn
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileBackup {
    pub existed: bool,
    #[serde(default)]
    pub content: String,
}

pub type CliBackups = BTreeMap<String, FileBackup>;

/// The native settings backup. Old CLI fields in this JSON are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverState {
    #[serde(default)]
    pub native: Option<CliBackups>,
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

pub fn reconcile(desired: &DesiredRoutes) -> Result<Vec<String>> {
    let paths = Paths::resolve().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let warnings = reconcile_at(&paths, desired)?;
    cleanup_legacy_generated_dirs(&paths.data_dir);
    Ok(warnings)
}

pub fn reconcile_at(paths: &Paths, desired: &DesiredRoutes) -> Result<Vec<String>> {
    let mut state = load_state(paths);
    let mut warnings = Vec::new();
    reconcile_native(
        desired.native.as_ref(),
        &mut state.native,
        &mut warnings,
        |routes, backups| native::take_over(&paths.native_dir, routes, backups),
        |backups| native::restore(&paths.native_dir, backups),
    );
    save_state(paths, &state)?;
    Ok(warnings)
}

fn reconcile_native<T>(
    target: Option<&T>,
    slot: &mut Option<CliBackups>,
    warnings: &mut Vec<String>,
    take_over: impl FnOnce(&T, &mut CliBackups) -> Result<()>,
    restore: impl FnOnce(&CliBackups) -> Result<()>,
) {
    match target {
        Some(target) => {
            let backups = slot.get_or_insert_with(BTreeMap::new);
            if let Err(error) = take_over(target, backups) {
                warnings.push(format!("native: {error:#}"));
            }
        }
        None => {
            if let Some(backups) = slot.take()
                && let Err(error) = restore(&backups)
            {
                warnings.push(format!("native: {error:#}"));
                *slot = Some(backups);
            }
        }
    }
}

pub fn config_file_for(provider_id: &str) -> Option<PathBuf> {
    let paths = Paths::resolve()?;
    NATIVE_SLOTS
        .contains(&provider_id)
        .then(|| native::settings_path(&paths.native_dir))
}

fn cleanup_legacy_generated_dirs(data_dir: &Path) {
    for legacy in ["gateway", "custom"] {
        let _ = std::fs::remove_dir_all(data_dir.join(legacy));
    }
}

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

/// Write atomically beside the target, with owner-only permissions on Unix.
pub fn atomic_write_private(path: &Path, contents: &[u8]) -> Result<()> {
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
        let fallback = std::fs::remove_file(path).and_then(|_| std::fs::rename(&temporary, path));
        if fallback.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        fallback.with_context(|| format!("could not replace {}", path.display()))
    })?;
    Ok(())
}

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

    fn cloud() -> GatewayConfig {
        GatewayConfig {
            enabled: true,
            endpoint: "https://gateway.example.com".into(),
            api_key: Some("sk-general".into()),
            ..GatewayConfig::default()
        }
    }

    #[test]
    fn the_gateway_feeds_every_unbound_native_format() {
        let desired = desired_routes(Some(&cloud()), &CustomApiConfig::default());
        let native = desired.native.expect("native routes");
        for target in [native.messages, native.responses, native.chat] {
            let target = target.expect("route");
            assert_eq!(target.base_url, "https://gateway.example.com");
            assert_eq!(target.api_key, "sk-general");
        }
        assert_eq!(
            native.platform_keys.get("default").map(String::as_str),
            Some("sk-general")
        );
    }

    #[test]
    fn an_empty_configuration_writes_no_native_route() {
        assert!(
            desired_routes(None, &CustomApiConfig::default())
                .native
                .is_none()
        );
        assert_eq!(
            active_route_kind("native_chat", None, &CustomApiConfig::default()),
            RouteKind::CliOwn
        );
    }

    #[test]
    fn the_legacy_takeover_file_keeps_only_native_state() {
        let state: TakeoverState = serde_json::from_str(
            r#"{"claude":{"settings.json":{"existed":true,"content":"old"}}}"#,
        )
        .expect("legacy state");
        assert_eq!(state, TakeoverState::default());
    }
}
