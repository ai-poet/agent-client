//! Headless Computer Use state and helper lifecycle.
//!
//! The native helper differs per platform — the Swift bundle shipped inside
//! the app on macOS, `cua-driver` on Windows — but everything above it
//! speaks one protocol (MCP over stdio) and lives at one set of paths
//! resolved here.

#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::io::Write as _;
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use uuid::Uuid;

#[cfg(target_os = "macos")]
const MAX_HELPER_OUTPUT_BYTES: usize = 24 * 1024 * 1024;

pub use waku_protocol::computer_use::{
    ComputerAppGrant, ComputerPermissions, ComputerTarget, ComputerUsePhase, ComputerUseState,
};

#[derive(Clone, Debug)]
pub struct ComputerToolRequest {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
}

impl ComputerToolRequest {
    pub fn summary(&self) -> String {
        if self.tool != "use" {
            return match self.tool.as_str() {
                "status" => "Check computer-use access".into(),
                _ => self.tool.clone(),
            };
        }
        let actions = self
            .arguments
            .get("actions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if actions.is_empty() {
            return "Inspect the window".into();
        }
        let mut labels = actions
            .iter()
            .filter_map(|action| action.get("type").and_then(Value::as_str))
            .map(action_label)
            .collect::<Vec<_>>();
        labels.dedup();
        format!("{} {}", labels.join(", "), plural(actions.len(), "action"))
    }
}

fn action_label(action: &str) -> &'static str {
    match action {
        "click" | "double_click" => "Click",
        "move" => "Move the pointer",
        "drag" => "Drag",
        "scroll" => "Scroll",
        "type" => "Type text",
        "keypress" => "Press keys",
        "wait" => "Wait",
        _ => "Interact",
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComputerUsePreviewUpdate {
    target: ComputerTarget,
    image_url: String,
}

pub fn decode_preview_update(data: &[u8]) -> anyhow::Result<ComputerUseState> {
    let update: ComputerUsePreviewUpdate =
        serde_json::from_slice(data).context("Computer Use preview is invalid JSON")?;
    validate_preview_image_url(&update.image_url)?;
    Ok(ComputerUseState {
        target: Some(update.target),
        phase: ComputerUsePhase::Running,
        visible: true,
        image_url: Some(update.image_url),
    })
}

fn validate_preview_image_url(image_url: &str) -> anyhow::Result<()> {
    const PNG_PREFIX: &str = "data:image/png;base64,";
    let encoded = image_url
        .strip_prefix(PNG_PREFIX)
        .ok_or_else(|| anyhow!("Computer Use preview is not a PNG data URL"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("Computer Use preview contains invalid base64")?;
    if bytes.is_empty() {
        bail!("Computer Use preview is empty");
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct PendingComputerApproval {
    pub request: ComputerToolRequest,
    pub target: ComputerTarget,
    pub sensitive: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperResponse {
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    permissions: Option<ComputerPermissions>,
}

/// macOS: ask the helper for its Screen Recording and Accessibility grants,
/// prompting the user for them when `prompt` is set.
#[cfg(target_os = "macos")]
pub fn probe_permissions(prompt: bool) -> anyhow::Result<ComputerPermissions> {
    let operation = if prompt {
        json!({"operation": "requestPermissions"})
    } else {
        json!({"operation": "status"})
    };
    let helper = mcp_server_command()?;
    let active_helper_pid = AtomicU32::new(0);
    let response = invoke_helper_direct(&helper, &operation, &active_helper_pid)?;
    if !response.success {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| tr!("computer_use.permission_check_failed"))
        );
    }
    Ok(response.permissions.unwrap_or_default())
}

/// Windows: ask `cua-driver` itself, over the same MCP session the REPL
/// would open. There is no TCC here, so the two grants are structurally
/// present whenever the driver answers; what the probe really establishes
/// is that a driver is installed, runs, and can reach the interactive
/// desktop. A field the driver does not report counts as granted.
#[cfg(windows)]
pub fn probe_permissions(prompt: bool) -> anyhow::Result<ComputerPermissions> {
    use std::time::{Duration, Instant};

    use sub2api::mcp_stdio::{McpStdioClient, text_content};

    let driver = mcp_server_command()?;
    let deadline = Some(Instant::now() + Duration::from_secs(30));
    let mut client = McpStdioClient::spawn(
        &driver,
        &["mcp", "--direct"],
        &[],
        "waku-daemon",
        "Computer Use driver",
        deadline,
    )?;
    let result = client.call_tool(
        "check_permissions",
        json!({"prompt": prompt, "probe_direct_capture": true}),
        deadline,
    )?;
    let report = result
        .get("structuredContent")
        .cloned()
        .or_else(|| serde_json::from_str::<Value>(&text_content(&result)).ok())
        .unwrap_or(Value::Null);
    let granted = |name: &str| report.get(name).and_then(Value::as_bool).unwrap_or(true);
    Ok(ComputerPermissions {
        screen_recording: granted("screen_recording"),
        accessibility: granted("accessibility"),
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn probe_permissions(_: bool) -> anyhow::Result<ComputerPermissions> {
    bail!("Computer Use is not available on this platform")
}

#[cfg(target_os = "macos")]
fn invoke_helper_direct(
    helper: &Path,
    operation: &Value,
    active_helper_pid: &AtomicU32,
) -> anyhow::Result<HelperResponse> {
    let mode = match operation.get("operation").and_then(Value::as_str) {
        Some("status") => Some("status"),
        Some("requestPermissions") => Some("request-permissions"),
        _ => None,
    };
    let mut command = Command::new(helper);
    if let Some(mode) = mode {
        command.arg(mode);
    }
    let command = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::command_env::spawn(command)
        .with_context(|| format!("failed to start {}", helper.display()))?;
    let pid = child.id();
    active_helper_pid.store(pid, Ordering::SeqCst);
    let payload = serde_json::to_vec(operation)?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("computer-use helper stdin unavailable"))?
        .write_all(&payload)?;
    let output = child.wait_with_output()?;
    let _ = active_helper_pid.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    if output.stdout.len() > MAX_HELPER_OUTPUT_BYTES {
        bail!("computer-use helper returned too much data");
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("computer-use helper failed: {}", stderr.trim());
    }
    serde_json::from_slice(&output.stdout).context("computer-use helper returned invalid JSON")
}

#[cfg(target_os = "macos")]
fn helper_app_path() -> anyhow::Result<PathBuf> {
    let executable = host_executable_path()?;
    let macos = executable
        .parent()
        .ok_or_else(|| anyhow!("Waku executable has no parent directory"))?;
    let contents = macos
        .parent()
        .ok_or_else(|| anyhow!("Waku app bundle is malformed"))?;
    let app_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("Waku executable name is invalid"))?;
    let helper_name = format!("{app_name} Computer Use");
    let path = contents.join("Helpers").join(format!("{helper_name}.app"));
    if !path.is_dir() {
        bail!("Computer Use helper is missing from this Waku build")
    }
    Ok(path)
}

/// The user-facing name of whatever does the native work.
#[cfg(not(windows))]
pub fn helper_display_name() -> String {
    host_executable_path()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .map(|app_name| format!("{app_name} Computer Use"))
        .unwrap_or_else(|| "Waku Computer Use".into())
}

#[cfg(windows)]
pub fn helper_display_name() -> String {
    "cua-driver".into()
}

/// The executable the REPL spawns in MCP mode to reach the desktop.
///
/// macOS: the bundled Swift helper, re-installed under Application Support
/// so it carries its own TCC identity.
#[cfg(target_os = "macos")]
pub fn mcp_server_command() -> anyhow::Result<PathBuf> {
    let bundled_helper = helper_app_path()?;
    let helper = install_helper_app(&bundled_helper)?;
    let executable = helper
        .file_stem()
        .ok_or_else(|| anyhow!("Computer Use helper name is invalid"))?;
    Ok(helper.join("Contents").join("MacOS").join(executable))
}

/// Windows: `cua-driver`, wherever [`sub2api::cua_install::resolve_driver`]
/// finds it. Nothing is installed here; Settings does that on request.
#[cfg(windows)]
pub fn mcp_server_command() -> anyhow::Result<PathBuf> {
    sub2api::cua_install::resolve_driver()
        .ok_or_else(|| anyhow!("Computer Use driver (cua-driver) is not installed"))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn mcp_server_command() -> anyhow::Result<PathBuf> {
    bail!("Computer Use is not available on this platform")
}

/// The directory holding the resources shipped beside Waku — the JavaScript
/// REPL, the Pi extension and the skill. Inside the bundle's `Resources` on
/// macOS; flat beside the executable everywhere else, which is the layout
/// `scripts/bundle-windows.ts` produces.
fn resource_root() -> anyhow::Result<PathBuf> {
    let executable = host_executable_path()?;
    let directory = executable
        .parent()
        .ok_or_else(|| anyhow!("Waku executable has no parent directory"))?;
    if cfg!(target_os = "macos") {
        let contents = directory
            .parent()
            .ok_or_else(|| anyhow!("Waku app bundle is malformed"))?;
        Ok(contents.join("Resources"))
    } else {
        Ok(directory.to_path_buf())
    }
}

pub fn js_repl_server_path() -> anyhow::Result<PathBuf> {
    let name = if cfg!(windows) {
        "waku_js_repl.exe"
    } else {
        "waku_js_repl"
    };
    let path = resource_root()?.join(name);
    if !path.is_file() {
        bail!("Waku JavaScript REPL is missing from this Waku build")
    }
    Ok(path)
}

pub fn pi_extension_path() -> anyhow::Result<PathBuf> {
    let path = resource_root()?
        .join("computer-use")
        .join("pi-extension.ts");
    if !path.is_file() {
        bail!("Waku Pi Computer Use extension is missing from this Waku build")
    }
    Ok(path)
}

/// Install the bundled helper as an independent, stable runtime service.
///
/// Screen Recording differs from Accessibility on macOS: it follows the
/// responsible application. A helper launched from inside Waku's bundle is
/// therefore attributed to Waku even though the capture API runs in the
/// helper. Launching this standalone copy through Launch Services gives the
/// helper its own TCC identity while the signed app bundle remains the source
/// shipped with Waku.
#[cfg(target_os = "macos")]
fn install_helper_app(source: &Path) -> anyhow::Result<PathBuf> {
    let application_support =
        dirs::data_dir().ok_or_else(|| anyhow!("Application Support directory is unavailable"))?;
    let install_root = application_support
        .join(crate::identity::DATA_DIRECTORY_NAME)
        .join("Computer Use");
    crate::fs_ext::create_private_dir_all(&install_root)
        .with_context(|| format!("could not create {}", install_root.display()))?;
    let bundle_name = source
        .file_name()
        .ok_or_else(|| anyhow!("Computer Use helper bundle name is invalid"))?;
    let destination = install_root.join(bundle_name);
    if helper_install_matches(source, &destination)? {
        return Ok(destination);
    }

    let staging = install_root.join(format!(".install-{}.app", Uuid::new_v4().simple()));
    copy_directory(source, &staging)?;
    let previous = install_root.join(format!(".previous-{}.app", Uuid::new_v4().simple()));
    let had_previous = destination.exists();
    if had_previous {
        fs::rename(&destination, &previous)
            .with_context(|| format!("could not replace {}", destination.display()))?;
    }
    if let Err(error) = fs::rename(&staging, &destination) {
        if had_previous {
            let _ = fs::rename(&previous, &destination);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(error).context("could not install Computer Use helper");
    }
    if had_previous {
        let _ = fs::remove_dir_all(previous);
    }
    Ok(destination)
}

#[cfg(target_os = "macos")]
fn helper_install_matches(source: &Path, destination: &Path) -> anyhow::Result<bool> {
    if !destination.is_dir() {
        return Ok(false);
    }
    let fingerprint = Path::new("Contents/Resources/.waku-helper-fingerprint");
    let source_fingerprint = fs::read(source.join(fingerprint))?;
    let Ok(installed_fingerprint) = fs::read(destination.join(fingerprint)) else {
        return Ok(false);
    };
    Ok(source_fingerprint == installed_fingerprint)
}

#[cfg(target_os = "macos")]
fn copy_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    fs::create_dir(destination)?;
    fs::set_permissions(destination, metadata.permissions())?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_symlink() {
            crate::fs_ext::symlink(&fs::read_link(&source_path)?, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)?;
            fs::set_permissions(
                &destination_path,
                fs::symlink_metadata(&source_path)?.permissions(),
            )?;
        }
    }
    Ok(())
}

pub fn skill_root_path() -> anyhow::Result<PathBuf> {
    let path = resource_root()?.join("skills");
    if !path.join("waku-computer-use").join("SKILL.md").is_file() {
        bail!("Waku Computer Use skill is missing from this Waku build")
    }
    Ok(path)
}

fn host_executable_path() -> anyhow::Result<PathBuf> {
    std::env::var_os(crate::APP_EXECUTABLE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| std::env::current_exe().context("Waku executable path is unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_grants_preserve_bundle_identity() {
        let target = ComputerTarget {
            window_id: 42,
            bundle_id: "net.imput.helium".into(),
            team_id: Some("S4Q33XPHB4".into()),
            app_name: "Helium".into(),
            window_title: "Window".into(),
            width: 1440,
            height: 823,
        };
        let grant = ComputerAppGrant {
            bundle_id: "net.imput.helium".into(),
            app_name: "Helium".into(),
        };
        assert_eq!(target.grant_key(), grant.key());
        assert!(target.persistable());
    }

    #[test]
    fn preview_updates_restore_the_pip_state() {
        let state = decode_preview_update(
            br#"{
                "target": {
                    "windowId": 42,
                    "bundleId": "net.imput.helium",
                    "teamId": "S4Q33XPHB4",
                    "appName": "Helium",
                    "windowTitle": "Window",
                    "width": 1440,
                    "height": 823
                },
                "imageUrl": "data:image/png;base64,aGVsbG8="
            }"#,
        )
        .unwrap();

        assert_eq!(state.target.unwrap().window_id, 42);
        assert_eq!(state.phase, ComputerUsePhase::Running);
        assert!(state.visible);
        assert!(state.image_url.is_some());
    }
}
