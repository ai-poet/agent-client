//! The `cua-driver` backend for `sky`.
//!
//! On macOS the native helper speaks the `sky` vocabulary itself, one MCP
//! tool per operation. On Windows the native work is done by `cua-driver`
//! (<https://github.com/trycua/cua>), whose tools address windows by
//! `(pid, window_id)`, hand out per-snapshot element tokens, and spell keys
//! and scroll amounts their own way. This module translates the ten `sky`
//! operations onto that surface so the model-facing contract — the `sky`
//! object, the skill, every provider wiring — stays identical across
//! platforms.
//!
//! What lives here besides translation:
//!
//! - **Target resolution.** A `sky` `app` is a display name, process name or
//!   executable path; the driver wants a pid and an HWND. Resolution is
//!   cached per session and re-done once when the driver reports the window
//!   gone.
//! - **The blocklist.** The macOS helper refuses password managers, security
//!   prompts and terminals by bundle id. The driver has no such gate, so the
//!   same policy is applied here by executable name (and by title for the
//!   packaged apps `ApplicationFrameHost.exe` fronts).
//! - **Elevated targets.** User Interface Privilege Isolation makes input to
//!   a higher-integrity window vanish without error. Those are refused up
//!   front instead of letting the model chase a UI that never reacts.
//! - **The accessibility diff and the preview.** Both were computed inside
//!   the Swift helper; the driver does neither, so they move here. The
//!   preview file is the one `ComputerUsePreviewMonitor` already watches.
//! - **Password redaction.** Windows UI Automation does not tell the driver
//!   which edit boxes are password fields (trycua/cua#3576), so values of
//!   password-looking fields are masked before the tree reaches the model.
//!
//! What deliberately does not live here: the peer verification the Swift
//! helper performs. That helper is a separately installed app with untrusted
//! callers; the driver is this process's own child over a private pipe.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use serde_json::{Value as JsonValue, json};
use sub2api::mcp_stdio::{McpStdioClient, is_tool_error, text_content};

/// `cua-driver mcp --direct`: the process owns its runtime and exits when
/// stdin closes, rather than proxying to a daemon.
const DRIVER_ARGS: &[&str] = &["mcp", "--direct"];

/// The driver phones home by default; this app does not. Both spellings the
/// binary understands are set so a rename upstream does not re-enable it.
const DRIVER_ENV: &[(&str, &str)] = &[
    ("CUA_DRIVER_RS_TELEMETRY_ENABLED", "0"),
    ("CUA_TELEMETRY_ENABLED", "0"),
];

/// Bounds on the UI Automation walk. Electron trees run to tens of
/// thousands of nodes; the model cannot use that many anyway.
const MAX_ELEMENTS: u64 = 2000;
const MAX_DEPTH: u64 = 25;

/// How long a freshly launched app is given to show a window.
const LAUNCH_WINDOW_WAIT: Duration = Duration::from_secs(5);

/// How long after an action a state capture waits for the UI to settle —
/// the same grace the macOS helper applies.
const SETTLE_AFTER_ACTION: Duration = Duration::from_secs(1);

/// Executables `sky` must never drive: credential stores, the security and
/// settings surfaces, terminals, and this app itself.
const BLOCKED_EXECUTABLES: &[&str] = &[
    // password managers
    "1password.exe",
    "1password-browserhelper.exe",
    "bitwarden.exe",
    "lastpass.exe",
    "keepass.exe",
    "keepassxc.exe",
    "dashlane.exe",
    "enpass.exe",
    "nordpass.exe",
    "proton pass.exe",
    // OS security and settings surfaces
    "systemsettings.exe",
    "control.exe",
    "mmc.exe",
    "lsass.exe",
    "winlogon.exe",
    "logonui.exe",
    "consent.exe",
    "credentialuibroker.exe",
    "securityhealthhost.exe",
    "securityhealthsystray.exe",
    "credwiz.exe",
    // terminals
    "windowsterminal.exe",
    "wt.exe",
    "openconsole.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "mintty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "wezterm.exe",
    "ghostty.exe",
    "putty.exe",
    "kitty.exe",
    // this app and its helpers
    "waku.exe",
    "waku-daemon.exe",
    "waku_js_repl.exe",
    "cua-driver.exe",
    "cua-driver-uia.exe",
];

/// Packaged apps are all fronted by `ApplicationFrameHost.exe`, so the
/// executable name cannot tell Settings from Calculator. These titles are
/// refused when that host owns the window.
const BLOCKED_HOSTED_TITLES: &[&str] = &[
    "settings",
    "windows security",
    "设置",
    "windows 安全中心",
    "設定",
    "windows セキュリティ",
];

/// Windows that are never worth listing or matching by title: the desktop
/// itself and the shell's input and search surfaces.
const IGNORED_EXECUTABLES: &[&str] = &[
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchui.exe",
    "dwm.exe",
];
const IGNORED_TITLES: &[&str] = &["program manager"];

/// Labels that mark an edit control as holding a secret. UI Automation's
/// `IsPassword` never reaches the driver, so the label is the only signal.
const SECRET_LABEL_MARKERS: &[&str] = &[
    "password",
    "passphrase",
    "passcode",
    "secret",
    "api key",
    "api-key",
    "api_key",
    "apikey",
    "token",
    "cvv",
    "cvc",
    "security code",
    "pin",
    "密码",
    "口令",
    "密钥",
    "验证码",
    "パスワード",
    "暗証",
];

pub(crate) struct CuaAdapter {
    connection: Option<McpStdioClient>,
    process_directory: Option<PathBuf>,
    /// Normalized `app` argument → resolved window.
    targets: HashMap<String, Target>,
    /// Window id → the last `get_window_state` snapshot of it.
    snapshots: HashMap<u64, Snapshot>,
    /// Target key → the last full tree text, for the diff.
    trees: HashMap<String, String>,
    /// Window id → when it was last acted on.
    last_action: HashMap<u64, Instant>,
    /// Remarks to prepend to the next `get_app_state` text.
    notes: Vec<String>,
}

#[derive(Clone, Debug)]
struct Target {
    pid: u64,
    window_id: u64,
    /// The executable's file name, lowercased — `notepad.exe`.
    app_name: String,
    /// The full executable path when the process could be queried.
    executable: Option<PathBuf>,
    /// What the Start menu calls it, else the executable name.
    display_name: String,
    title: String,
}

impl Target {
    /// The identity the preview and the allow-list key on: the lowercase
    /// executable path when known, else the executable name.
    fn key(&self) -> String {
        self.executable
            .as_ref()
            .map(|path| path.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| self.app_name.clone())
    }
}

struct Snapshot {
    tokens: HashMap<u64, String>,
}

/// One row of the driver's `list_windows`.
#[derive(Clone, Debug)]
struct WindowRow {
    pid: u64,
    window_id: u64,
    app_name: String,
    title: String,
    z_index: i64,
    is_on_screen: bool,
    minimized: bool,
    area: i64,
}

/// One row of the driver's `list_apps`.
#[derive(Clone, Debug)]
struct AppRow {
    name: String,
    pid: u64,
    running: bool,
    launch_path: Option<String>,
    last_used: Option<String>,
}

impl AppRow {
    /// The executable's file name, lowercased, when the launch path names one.
    fn executable_name(&self) -> Option<String> {
        let path = self.launch_path.as_deref()?;
        if path.starts_with("shell:") {
            return None;
        }
        let name = windows_file_name(path.trim_matches('"')).to_lowercase();
        name.ends_with(".exe").then_some(name)
    }
}

/// The last component of a path as the driver reports it. Those are Windows
/// paths, so they are split on `\` as well as `/` whatever the host: `Path`
/// on macOS or Linux reads the whole of `C:\x\y.exe` as one file name.
fn windows_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

impl CuaAdapter {
    pub(crate) fn new() -> Self {
        Self {
            connection: None,
            process_directory: std::env::var_os("WAKU_COMPUTER_USE_PROCESS_DIRECTORY")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            targets: HashMap::new(),
            snapshots: HashMap::new(),
            trees: HashMap::new(),
            last_action: HashMap::new(),
            notes: Vec::new(),
        }
    }

    /// Forget every resolved window, snapshot and diff baseline. The driver
    /// itself stays up; `js_reset` is about the model's state, not the
    /// desktop's.
    pub(crate) fn reset(&mut self) {
        self.targets.clear();
        self.snapshots.clear();
        self.trees.clear();
        self.last_action.clear();
        self.notes.clear();
    }

    /// Serve one `sky` operation. `name` has already passed the allow-list.
    pub(crate) fn call(
        &mut self,
        name: &str,
        arguments: JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<JsonValue> {
        match name {
            "list_apps" => self.list_apps(deadline),
            "get_app_state" => self.get_app_state(&arguments, deadline),
            "click" => self.click(&arguments, deadline),
            "drag" => self.drag(&arguments, deadline),
            "scroll" => self.scroll(&arguments, deadline),
            "press_key" => self.press_key(&arguments, deadline),
            "type_text" => self.type_text(&arguments, deadline),
            "set_value" => self.set_value(&arguments, deadline),
            "perform_secondary_action" => self.perform_secondary_action(&arguments, deadline),
            "select_text" => bail!(
                "select_text is not available on Windows. Use set_value to replace an \
                 element's whole value, or click the element and use press_key with \
                 shift+Left/Right/Home/End to select text."
            ),
            _ => bail!("unknown sky operation: {name}"),
        }
    }

    // --- driver session --------------------------------------------------

    fn connection(&mut self, deadline: Option<Instant>) -> anyhow::Result<&mut McpStdioClient> {
        if self.connection.is_none() {
            let driver = std::env::var_os("WAKU_COMPUTER_USE_SERVER")
                .map(PathBuf::from)
                .ok_or_else(|| {
                    anyhow!("WAKU_COMPUTER_USE_SERVER is required before the first sky operation")
                })?;
            let client = McpStdioClient::spawn(
                &driver,
                DRIVER_ARGS,
                DRIVER_ENV,
                "waku_js_repl",
                "Computer Use driver",
                deadline,
            )?;
            if let Some(directory) = &self.process_directory {
                // Registered so the daemon's reaper can find it if this
                // process dies without dropping the client.
                let _ = fs::write(directory.join(client.pid().to_string()), b"");
            }
            self.connection = Some(client);
        }
        Ok(self.connection.as_mut().expect("initialized above"))
    }

    /// Call one driver tool. `Ok` carries the result even when the tool
    /// flagged an error — the caller decides what a refusal means. `Err` is
    /// a transport failure, after which the driver (and every snapshot it
    /// held) is gone.
    fn tool(
        &mut self,
        name: &str,
        arguments: JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<JsonValue> {
        let result = self
            .connection(deadline)?
            .call_tool_raw(name, arguments, deadline);
        if result.is_err() {
            self.connection = None;
            self.snapshots.clear();
        }
        result
    }

    /// [`Self::tool`], turning a flagged result into an error.
    fn read(
        &mut self,
        name: &str,
        arguments: JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<JsonValue> {
        let result = self.tool(name, arguments, deadline)?;
        if is_tool_error(&result) {
            bail!("{}", refusal_text(&result));
        }
        Ok(result)
    }

    fn list_windows(&mut self, deadline: Option<Instant>) -> anyhow::Result<Vec<WindowRow>> {
        let result = self.read("list_windows", json!({"on_screen_only": false}), deadline)?;
        Ok(parse_windows(
            result
                .pointer("/structuredContent/windows")
                .unwrap_or(&JsonValue::Null),
        ))
    }

    fn list_installed_apps(&mut self, deadline: Option<Instant>) -> anyhow::Result<Vec<AppRow>> {
        let result = self.read("list_apps", json!({}), deadline)?;
        Ok(parse_apps(
            result
                .pointer("/structuredContent/apps")
                .unwrap_or(&JsonValue::Null),
        ))
    }

    // --- target resolution -----------------------------------------------

    fn resolve(&mut self, app: &str, deadline: Option<Instant>) -> anyhow::Result<Target> {
        let query = normalize_app(app)?;
        if let Some(target) = self.targets.get(&query) {
            return Ok(target.clone());
        }
        let target = self.resolve_uncached(&query, app, deadline)?;
        self.targets.insert(query, target.clone());
        Ok(target)
    }

    fn forget(&mut self, target: &Target) {
        self.targets
            .retain(|_, cached| cached.window_id != target.window_id);
        self.snapshots.remove(&target.window_id);
    }

    fn resolve_uncached(
        &mut self,
        query: &str,
        app: &str,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Target> {
        let windows = self.list_windows(deadline)?;
        let mut apps: Option<Vec<AppRow>> = None;

        let mut candidates = match_windows_by_executable(query, &windows);
        if candidates.is_empty() {
            let rows = self.list_installed_apps(deadline)?;
            candidates = match_windows_by_app_name(query, &rows, &windows, true);
            apps = Some(rows);
        }
        if candidates.is_empty() {
            candidates = match_windows_by_title(query, &windows);
        }
        if candidates.is_empty() {
            let rows = apps.as_deref().expect("listed above");
            candidates = match_windows_by_app_name(query, rows, &windows, false);
        }
        let launched: Vec<WindowRow>;
        if candidates.is_empty() {
            // Nothing running answers to that name: launch it, the way the
            // macOS helper launches an installed app on first use. Only an
            // installed app or an explicit executable path qualifies — a bare
            // name is never handed to a PATH search.
            let rows = apps.as_deref().expect("listed above");
            let launch = launch_argument(query, app, rows);
            let Some(launch) = launch else {
                bail!(
                    "no window matches \"{app}\"; open the app first, or call \
                     sky.list_apps() to see what is running and installed"
                );
            };
            launched = self.launch(launch, deadline)?;
            candidates = launched.iter().collect();
        }

        let Some(window) = pick_window(&candidates) else {
            bail!("no window matches \"{app}\"");
        };
        let display_name = apps
            .as_deref()
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.running && row.pid == window.pid)
                    .map(|row| row.name.clone())
            })
            .unwrap_or_else(|| window.app_name.clone());
        self.admit(window, display_name)
    }

    /// Turn a chosen window into a target, refusing what policy forbids.
    fn admit(&self, window: &WindowRow, display_name: String) -> anyhow::Result<Target> {
        if is_blocked(&window.app_name, &window.title) {
            bail!(
                "Computer Use cannot control {} (\"{}\"): password managers, security \
                 and settings surfaces, terminals and this app itself are off limits",
                window.app_name,
                window.title
            );
        }
        let pid = u32::try_from(window.pid).context("pid does not fit in u32")?;
        if process_is_protected(pid) {
            bail!(
                "\"{}\" ({}) runs elevated or protected, so Windows would silently \
                 discard any input sent to it; it cannot be controlled",
                window.title,
                window.app_name
            );
        }
        Ok(Target {
            pid: window.pid,
            window_id: window.window_id,
            app_name: window.app_name.clone(),
            executable: process_executable(pid),
            display_name,
            title: window.title.clone(),
        })
    }

    /// `launch_app`, then wait for a window to appear.
    fn launch(
        &mut self,
        arguments: JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Vec<WindowRow>> {
        let result = self.read("launch_app", arguments, deadline)?;
        let pid = result
            .pointer("/structuredContent/pid")
            .and_then(JsonValue::as_u64)
            .filter(|pid| *pid > 0)
            .ok_or_else(|| anyhow!("the app was launched but reported no process id"))?;
        let started = Instant::now();
        loop {
            let windows = self
                .list_windows(deadline)?
                .into_iter()
                .filter(|window| window.pid == pid)
                .collect::<Vec<_>>();
            if !windows.is_empty() {
                return Ok(windows);
            }
            if started.elapsed() >= LAUNCH_WINDOW_WAIT
                || deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                bail!("the app was launched (pid {pid}) but showed no window");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Run `operation` against the resolved `app`, once more with a fresh
    /// resolution if the driver reports the cached window gone.
    fn with_target<T>(
        &mut self,
        app: &str,
        deadline: Option<Instant>,
        mut operation: impl FnMut(&mut Self, &Target) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let target = self.resolve(app, deadline)?;
        match operation(self, &target) {
            Err(error) if is_window_gone(&error) => {
                self.forget(&target);
                let target = self.resolve(app, deadline)?;
                operation(self, &target)
            }
            other => other,
        }
    }

    // --- reads -----------------------------------------------------------

    fn list_apps(&mut self, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let windows = self.list_windows(deadline)?;
        let apps = self.list_installed_apps(deadline)?;
        Ok(JsonValue::Array(app_list(&windows, &apps)))
    }

    fn get_app_state(
        &mut self,
        arguments: &JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let disable_diff = arguments
            .get("disableDiff")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        self.with_target(&app, deadline, |this, target| {
            this.settle(target.window_id);
            let result = this.read(
                "get_window_state",
                json!({
                    "pid": target.pid,
                    "window_id": target.window_id,
                    "include_accessibility_tree": true,
                    "include_screenshot": true,
                    "max_elements": MAX_ELEMENTS,
                    "max_depth": MAX_DEPTH,
                }),
                deadline,
            )?;
            let structured = result
                .get("structuredContent")
                .cloned()
                .unwrap_or(JsonValue::Null);
            let elements = structured
                .get("elements")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            this.snapshots
                .insert(target.window_id, Snapshot { tokens: element_tokens(&elements) });

            let tree = structured
                .get("tree_markdown")
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| text_content(&result));
            let tree = redact_secret_values(&tree, &elements);
            let title = window_title(&tree).unwrap_or_else(|| target.title.clone());
            let mut full = format!(
                "Window: \"{title}\", App: {} ({}, pid {})\n{}",
                target.display_name,
                target.app_name,
                target.pid,
                tree.trim_end()
            );
            if structured.get("elements_complete").and_then(JsonValue::as_bool) == Some(false) {
                let returned = structured
                    .get("returned_element_count")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0);
                let total = structured
                    .get("total_element_count")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0);
                full.push_str(&format!(
                    "\n(The accessibility tree was truncated to {returned} of {total} elements.)"
                ));
            }
            let mut text = this.render_tree(&target.key(), full, disable_diff);
            if !this.notes.is_empty() {
                let notes = this
                    .notes
                    .drain(..)
                    .map(|note| format!("<computer_use_note>{note}</computer_use_note>"))
                    .collect::<Vec<_>>()
                    .join("\n");
                text = format!("{notes}\n{text}");
            }

            let screenshot = screenshot_data_url(&result, &structured);
            if let Some(url) = &screenshot {
                let width = structured
                    .get("screenshot_width")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0);
                let height = structured
                    .get("screenshot_height")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0);
                this.write_preview(target, &title, width, height, url);
            }
            Ok(json!({
                "app": target.app_name,
                "text": text,
                "screenshot": screenshot.map(|url| json!({"url": url})),
            }))
        })
    }

    /// Give the UI the same grace the macOS helper does after an action.
    fn settle(&self, window_id: u64) {
        if let Some(acted) = self.last_action.get(&window_id) {
            if let Some(remaining) = SETTLE_AFTER_ACTION.checked_sub(acted.elapsed()) {
                std::thread::sleep(remaining);
            }
        }
    }

    /// The macOS helper's diff: the full tree the first time or on request,
    /// a one-line notice when nothing changed, otherwise the lines that
    /// left and arrived.
    fn render_tree(&mut self, key: &str, tree: String, disable_diff: bool) -> String {
        let previous = self.trees.insert(key.to_owned(), tree.clone());
        if disable_diff {
            return tree;
        }
        let Some(previous) = previous else {
            return tree;
        };
        render_tree_diff(&previous, &tree)
    }

    fn write_preview(&self, target: &Target, title: &str, width: u64, height: u64, url: &str) {
        let Some(directory) = &self.process_directory else {
            return;
        };
        let Ok(window_id) = u32::try_from(target.window_id) else {
            return;
        };
        let payload = json!({
            "target": {
                "windowId": window_id,
                "bundleId": target.key(),
                "appName": target.display_name,
                "windowTitle": title,
                "width": width,
                "height": height,
            },
            "imageUrl": url,
        });
        let path = directory.join(format!("preview-{window_id}.json"));
        let staging = directory.join(format!("preview-{window_id}.json.tmp"));
        if serde_json::to_vec(&payload)
            .ok()
            .and_then(|bytes| fs::write(&staging, bytes).ok())
            .is_some()
        {
            let _ = fs::rename(&staging, &path);
        }
    }

    // --- actions ---------------------------------------------------------

    /// Perform one driver action. A `background_unavailable` refusal is
    /// retried in the foreground — the driver has established that no
    /// focus-preserving route exists — and the model is told so with the
    /// next state.
    fn act(
        &mut self,
        target: &Target,
        tool: &str,
        mut arguments: JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<JsonValue> {
        let mut result = self.tool(tool, arguments.clone(), deadline)?;
        if is_tool_error(&result)
            && refusal_code(&result) == Some("background_unavailable")
            && arguments.get("delivery_mode").and_then(JsonValue::as_str) == Some("background")
        {
            arguments["delivery_mode"] = json!("foreground");
            result = self.tool(tool, arguments, deadline)?;
            if !is_tool_error(&result) {
                self.notes.push(format!(
                    "the last {tool} could not be delivered in the background and was \
                     delivered in the foreground instead; \"{}\" was briefly focused",
                    target.title
                ));
            }
        }
        if is_tool_error(&result) {
            bail!("{}", refusal_text(&result));
        }
        self.last_action.insert(target.window_id, Instant::now());
        if result.pointer("/structuredContent/effect").and_then(JsonValue::as_str)
            == Some("suspected_noop")
        {
            self.notes.push(format!(
                "the last {tool} may have had no effect; check the new state before repeating it"
            ));
        }
        Ok(JsonValue::Null)
    }

    /// The token for `element_index` in the window's current snapshot.
    fn element_token(&self, target: &Target, arguments: &JsonValue) -> anyhow::Result<String> {
        let index = arguments
            .get("element_index")
            .and_then(element_index)
            .ok_or_else(|| anyhow!("element_index is required"))?;
        let token = self
            .snapshots
            .get(&target.window_id)
            .and_then(|snapshot| snapshot.tokens.get(&index));
        match token {
            Some(token) => Ok(token.clone()),
            None => bail!(
                "element {index} is not in the current snapshot of \"{}\"; call \
                 sky.get_app_state first and use an index from that tree",
                target.title
            ),
        }
    }

    fn click(&mut self, arguments: &JsonValue, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let button = mouse_button(arguments.get("mouse_button"))?;
        let count = click_count(arguments.get("click_count"))?;
        self.with_target(&app, deadline, |this, target| {
            let mut call = json!({
                "pid": target.pid,
                "window_id": target.window_id,
                "button": button,
                "count": count,
                "delivery_mode": "background",
            });
            if arguments.get("element_index").is_some() {
                call["element_token"] = json!(this.element_token(target, arguments)?);
            } else {
                let (x, y) = coordinates(arguments, "x", "y")
                    .ok_or_else(|| anyhow!("click requires element_index, or x and y"))?;
                call["x"] = json!(x);
                call["y"] = json!(y);
                call["scope"] = json!("window");
            }
            this.act(target, "click", call, deadline)
        })
    }

    fn drag(&mut self, arguments: &JsonValue, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let (from_x, from_y) = coordinates(arguments, "from_x", "from_y")
            .ok_or_else(|| anyhow!("drag requires from_x and from_y"))?;
        let (to_x, to_y) = coordinates(arguments, "to_x", "to_y")
            .ok_or_else(|| anyhow!("drag requires to_x and to_y"))?;
        self.with_target(&app, deadline, |this, target| {
            this.act(
                target,
                "drag",
                json!({
                    "pid": target.pid,
                    "window_id": target.window_id,
                    "from_x": from_x,
                    "from_y": from_y,
                    "to_x": to_x,
                    "to_y": to_y,
                    "button": "left",
                    "delivery_mode": "background",
                    "scope": "window",
                }),
                deadline,
            )
        })
    }

    fn scroll(&mut self, arguments: &JsonValue, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let direction = scroll_direction(arguments.get("direction"))?;
        let (by, amount) = scroll_amount(arguments.get("pages"))?;
        self.with_target(&app, deadline, |this, target| {
            let mut call = json!({
                "pid": target.pid,
                "window_id": target.window_id,
                "direction": direction,
                "by": by,
                "amount": amount,
                "delivery_mode": "background",
            });
            if arguments.get("element_index").is_some() {
                call["element_token"] = json!(this.element_token(target, arguments)?);
            }
            this.act(target, "scroll", call, deadline)
        })
    }

    fn press_key(&mut self, arguments: &JsonValue, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let spec = required_string(arguments, "key")?;
        let (key, modifiers) = translate_key(&spec)?;
        self.with_target(&app, deadline, |this, target| {
            if modifiers.is_empty() {
                this.act(
                    target,
                    "press_key",
                    json!({
                        "pid": target.pid,
                        "window_id": target.window_id,
                        "key": key,
                        "delivery_mode": "background",
                    }),
                    deadline,
                )
            } else {
                let mut keys = modifiers.clone();
                keys.push(key.clone());
                this.act(
                    target,
                    "hotkey",
                    json!({
                        "pid": target.pid,
                        "window_id": target.window_id,
                        "keys": keys,
                        "delivery_mode": "background",
                    }),
                    deadline,
                )
            }
        })
    }

    fn type_text(&mut self, arguments: &JsonValue, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let text = arguments
            .get("text")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| anyhow!("text is required"))?
            .to_owned();
        self.with_target(&app, deadline, |this, target| {
            // The driver types characters only; a line break is the Return
            // key, which is what `sky.type_text` promises for "\n".
            let normalized = text.replace("\r\n", "\n");
            let mut first = true;
            for segment in normalized.split(['\n', '\r']) {
                if !first {
                    this.act(
                        target,
                        "press_key",
                        json!({
                            "pid": target.pid,
                            "window_id": target.window_id,
                            "key": "return",
                            "delivery_mode": "background",
                        }),
                        deadline,
                    )?;
                }
                first = false;
                if segment.is_empty() {
                    continue;
                }
                this.act(
                    target,
                    "type_text",
                    json!({
                        "pid": target.pid,
                        "window_id": target.window_id,
                        "text": segment,
                        "delivery_mode": "background",
                    }),
                    deadline,
                )?;
            }
            Ok(JsonValue::Null)
        })
    }

    fn set_value(&mut self, arguments: &JsonValue, deadline: Option<Instant>) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let value = arguments
            .get("value")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| anyhow!("value is required"))?
            .to_owned();
        self.with_target(&app, deadline, |this, target| {
            let token = this.element_token(target, arguments)?;
            this.act(
                target,
                "set_value",
                json!({
                    "pid": target.pid,
                    "window_id": target.window_id,
                    "element_token": token,
                    "value": value,
                }),
                deadline,
            )
        })
    }

    fn perform_secondary_action(
        &mut self,
        arguments: &JsonValue,
        deadline: Option<Instant>,
    ) -> anyhow::Result<JsonValue> {
        let app = required_string(arguments, "app")?;
        let action = required_string(arguments, "action")?;
        let plan = secondary_action(&action)?;
        self.with_target(&app, deadline, |this, target| match &plan {
            SecondaryAction::Menu(path) => this.act(
                target,
                "invoke_menu",
                json!({"pid": target.pid, "window_id": target.window_id, "path": path}),
                deadline,
            ),
            SecondaryAction::Click(button) => {
                let token = this.element_token(target, arguments)?;
                this.act(
                    target,
                    "click",
                    json!({
                        "pid": target.pid,
                        "window_id": target.window_id,
                        "element_token": token,
                        "button": button,
                        "count": 1,
                        "delivery_mode": "background",
                    }),
                    deadline,
                )
            }
        })
    }
}

// --- pure helpers ---------------------------------------------------------

fn required_string(arguments: &JsonValue, name: &str) -> anyhow::Result<String> {
    arguments
        .get(name)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{name} is required"))
}

fn normalize_app(app: &str) -> anyhow::Result<String> {
    let normalized = app.trim().to_lowercase();
    if normalized.is_empty() {
        bail!("app is required");
    }
    Ok(normalized)
}

fn element_index(value: &JsonValue) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn coordinates(arguments: &JsonValue, x: &str, y: &str) -> Option<(f64, f64)> {
    Some((
        arguments.get(x)?.as_f64()?,
        arguments.get(y)?.as_f64()?,
    ))
}

fn mouse_button(value: Option<&JsonValue>) -> anyhow::Result<&'static str> {
    Ok(match value.and_then(JsonValue::as_str).map(str::to_lowercase).as_deref() {
        None | Some("left" | "l") => "left",
        Some("right" | "r") => "right",
        Some("middle" | "m") => "middle",
        Some(other) => bail!("mouse_button must be left, right or middle (got \"{other}\")"),
    })
}

fn click_count(value: Option<&JsonValue>) -> anyhow::Result<u64> {
    let Some(value) = value else {
        return Ok(1);
    };
    let count = value.as_f64().ok_or_else(|| anyhow!("click_count must be a number"))?;
    if !count.is_finite() || count.fract() != 0.0 || !(1.0..=3.0).contains(&count) {
        bail!("click_count must be 1, 2 or 3 on Windows");
    }
    Ok(count as u64)
}

fn scroll_direction(value: Option<&JsonValue>) -> anyhow::Result<&'static str> {
    Ok(match value.and_then(JsonValue::as_str).map(str::to_lowercase).as_deref() {
        Some("up" | "u") => "up",
        Some("down" | "d") => "down",
        Some("left" | "l") => "left",
        Some("right" | "r") => "right",
        _ => bail!("direction must be up, down, left or right"),
    })
}

/// `pages` → the driver's `(by, amount)`: whole pages scroll by page, a
/// fraction of a page scrolls by lines at three lines per notch.
fn scroll_amount(value: Option<&JsonValue>) -> anyhow::Result<(&'static str, u64)> {
    let Some(value) = value else {
        return Ok(("page", 1));
    };
    let pages = value.as_f64().ok_or_else(|| anyhow!("pages must be a number"))?;
    if !pages.is_finite() || pages <= 0.0 {
        bail!("pages must be a positive number");
    }
    if pages.fract() == 0.0 {
        Ok(("page", (pages as u64).clamp(1, 50)))
    } else {
        Ok(("line", ((pages * 3.0).round() as u64).clamp(1, 50)))
    }
}

/// xdotool-style `"ctrl+shift+Return"` → the driver's key name and
/// modifier list. `super` is the Windows key, never rewritten to `ctrl`:
/// the skill tells the model which modifier means what on this platform.
fn translate_key(spec: &str) -> anyhow::Result<(String, Vec<String>)> {
    let parts = spec.split('+').map(str::trim).collect::<Vec<_>>();
    let Some((key, modifiers)) = parts.split_last() else {
        bail!("key is required");
    };
    if key.is_empty() {
        // "ctrl++" — the key itself is a plus sign.
        if spec.trim_end().ends_with('+') && parts.len() >= 2 {
            let modifiers = modifiers[..modifiers.len() - 1]
                .iter()
                .map(|m| translate_modifier(m))
                .collect::<anyhow::Result<Vec<_>>>()?;
            return Ok(("+".to_owned(), modifiers));
        }
        bail!("key is required");
    }
    let modifiers = modifiers
        .iter()
        .map(|m| translate_modifier(m))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok((translate_keysym(key), modifiers))
}

fn translate_modifier(modifier: &str) -> anyhow::Result<String> {
    Ok(match modifier.to_lowercase().as_str() {
        "ctrl" | "control" => "ctrl",
        "shift" => "shift",
        "alt" | "option" | "opt" => "alt",
        "super" | "win" | "windows" | "cmd" | "command" | "meta" => "win",
        other => bail!("unknown modifier \"{other}\"; use ctrl, shift, alt or super"),
    }
    .to_owned())
}

/// X keysym names the model may use → the driver's vocabulary. Unknown
/// names pass through lowercased; the driver reports what it cannot press.
fn translate_keysym(key: &str) -> String {
    let lower = key.to_lowercase();
    let mapped = match lower.as_str() {
        "return" | "enter" | "kp_enter" => "return",
        "escape" | "esc" => "escape",
        "backspace" => "backspace",
        "delete" | "del" => "delete",
        "tab" => "tab",
        "space" => "space",
        "prior" | "page_up" | "pageup" => "pageup",
        "next" | "page_down" | "pagedown" => "pagedown",
        "home" => "home",
        "end" => "end",
        "insert" => "insert",
        "up" | "down" | "left" | "right" => lower.as_str(),
        "kp_0" | "kp_1" | "kp_2" | "kp_3" | "kp_4" | "kp_5" | "kp_6" | "kp_7" | "kp_8"
        | "kp_9" => return format!("numpad{}", &lower[3..]),
        "kp_add" | "kp_plus" => "numpadadd",
        "kp_subtract" | "kp_minus" => "numpadsubtract",
        "kp_multiply" => "numpadmultiply",
        "kp_divide" => "numpaddivide",
        "kp_decimal" => "numpaddecimal",
        "print" => "printscreen",
        _ => lower.as_str(),
    };
    mapped.to_owned()
}

enum SecondaryAction {
    Menu(Vec<String>),
    Click(&'static str),
}

/// The driver exposes no per-element action list beyond click, so the
/// accessibility actions the tree advertises collapse onto a click, a
/// right-click, or a menu path.
fn secondary_action(action: &str) -> anyhow::Result<SecondaryAction> {
    let trimmed = action.trim();
    if let Some(path) = trimmed
        .strip_prefix("menu:")
        .or_else(|| trimmed.strip_prefix("Menu:"))
    {
        let segments = path
            .split('>')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if segments.is_empty() {
            bail!("a menu action needs a path, e.g. \"menu: File > Save As\"");
        }
        return Ok(SecondaryAction::Menu(segments));
    }
    let normalized = trimmed.to_lowercase().replace([' ', '-'], "_");
    Ok(match normalized.as_str() {
        "press" | "invoke" | "click" | "expand" | "collapse" | "toggle" | "select" | "open"
        | "confirm" | "pick" | "axpress" | "axexpand" | "axcollapse" | "axconfirm" | "axpick" => {
            SecondaryAction::Click("left")
        }
        "show_menu" | "context_menu" | "showmenu" | "axshowmenu" | "right_click" => {
            SecondaryAction::Click("right")
        }
        _ => bail!(
            "unsupported secondary action \"{action}\" on Windows; use press, expand, \
             show_menu, or a menu path such as \"menu: File > Save As\""
        ),
    })
}

fn parse_windows(value: &JsonValue) -> Vec<WindowRow> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let bounds = row.get("bounds");
            let size = |name: &str| {
                bounds
                    .and_then(|bounds| bounds.get(name))
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0)
            };
            Some(WindowRow {
                pid: row.get("pid")?.as_u64()?,
                window_id: row.get("window_id")?.as_u64()?,
                app_name: row
                    .get("app_name")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_lowercase(),
                title: row
                    .get("title")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                z_index: row.get("z_index").and_then(JsonValue::as_i64).unwrap_or(-1),
                is_on_screen: row
                    .get("is_on_screen")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                minimized: row
                    .get("minimized")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                area: size("width") * size("height"),
            })
        })
        .collect()
}

fn parse_apps(value: &JsonValue) -> Vec<AppRow> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let string = |name: &str| {
                row.get(name)
                    .and_then(JsonValue::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            Some(AppRow {
                name: string("name")?,
                pid: row.get("pid").and_then(JsonValue::as_u64).unwrap_or(0),
                running: row
                    .get("running")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                launch_path: string("launch_path"),
                last_used: string("last_used"),
            })
        })
        .collect()
}

fn executable_stem(app_name: &str) -> &str {
    app_name.strip_suffix(".exe").unwrap_or(app_name)
}

fn is_ignored(window: &WindowRow) -> bool {
    IGNORED_EXECUTABLES.contains(&window.app_name.as_str())
        || IGNORED_TITLES.contains(&window.title.to_lowercase().as_str())
        || window.title.is_empty()
}

/// Whether the policy forbids driving this window.
fn is_blocked(app_name: &str, title: &str) -> bool {
    let app_name = app_name.to_lowercase();
    if BLOCKED_EXECUTABLES.contains(&app_name.as_str()) {
        return true;
    }
    if app_name.starts_with("1password") {
        return true;
    }
    app_name == "applicationframehost.exe"
        && BLOCKED_HOSTED_TITLES.contains(&title.trim().to_lowercase().as_str())
}

/// Windows whose executable is `query` (`chrome`, `chrome.exe`) or whose
/// full executable path is.
fn match_windows_by_executable<'a>(query: &str, windows: &'a [WindowRow]) -> Vec<&'a WindowRow> {
    let query = query.trim_matches('"');
    let by_path = query.contains(['\\', '/']);
    let wanted_name = windows_file_name(query).to_lowercase();
    windows
        .iter()
        .filter(|window| !is_ignored(window))
        .filter(|window| {
            if by_path {
                let Ok(pid) = u32::try_from(window.pid) else {
                    return false;
                };
                process_executable(pid)
                    .is_some_and(|path| path.to_string_lossy().to_lowercase() == query)
            } else {
                window.app_name == wanted_name
                    || executable_stem(&window.app_name) == executable_stem(&wanted_name)
            }
        })
        .collect()
}

/// Windows of running apps whose Start-menu name matches `query`, exactly
/// or (when `exact` is false) as a substring.
fn match_windows_by_app_name<'a>(
    query: &str,
    apps: &[AppRow],
    windows: &'a [WindowRow],
    exact: bool,
) -> Vec<&'a WindowRow> {
    let pids = apps
        .iter()
        .filter(|app| app.running && app.pid > 0)
        .filter(|app| {
            let name = app.name.to_lowercase();
            if exact {
                name == query || app.executable_name().is_some_and(|exe| exe == query)
            } else {
                name.contains(query)
            }
        })
        .map(|app| app.pid)
        .collect::<Vec<_>>();
    windows
        .iter()
        .filter(|window| !is_ignored(window) && pids.contains(&window.pid))
        .collect()
}

fn match_windows_by_title<'a>(query: &str, windows: &'a [WindowRow]) -> Vec<&'a WindowRow> {
    windows
        .iter()
        .filter(|window| !is_ignored(window) && window.title.to_lowercase().contains(query))
        .collect()
}

/// The `launch_app` arguments for an app that is installed but not running:
/// its Start-menu launch path, or an explicit path to an executable.
fn launch_argument(query: &str, app: &str, apps: &[AppRow]) -> Option<JsonValue> {
    if let Some(installed) = apps.iter().filter(|app| !app.running).find(|installed| {
        installed.name.to_lowercase() == query
            || installed.executable_name().is_some_and(|exe| {
                exe == query || executable_stem(&exe) == executable_stem(query)
            })
    }) {
        if installed
            .executable_name()
            .is_some_and(|exe| is_blocked(&exe, ""))
        {
            return None;
        }
        let path = installed.launch_path.clone()?;
        return Some(json!({"launch_path": path}));
    }
    let candidate = Path::new(app.trim().trim_matches('"'));
    if candidate.is_absolute()
        && candidate.is_file()
        && candidate
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        let name = candidate.file_name()?.to_string_lossy().to_lowercase();
        if is_blocked(&name, "") {
            return None;
        }
        return Some(json!({"path": candidate.to_string_lossy()}));
    }
    None
}

/// The window the model most likely means: visible over minimized, then
/// frontmost, then the biggest.
fn pick_window<'a>(candidates: &[&'a WindowRow]) -> Option<&'a WindowRow> {
    candidates
        .iter()
        .copied()
        .max_by_key(|window| {
            (
                window.is_on_screen && !window.minimized,
                window.z_index,
                window.area,
            )
        })
}

/// The `sky.list_apps()` payload: every driveable window's app, then the
/// installed apps that are not running.
fn app_list(windows: &[WindowRow], apps: &[AppRow]) -> Vec<JsonValue> {
    let mut seen = std::collections::HashSet::new();
    let mut list = Vec::new();
    let mut running = windows
        .iter()
        .filter(|window| !is_ignored(window) && !is_blocked(&window.app_name, &window.title))
        .collect::<Vec<_>>();
    running.sort_by_key(|window| std::cmp::Reverse(window.z_index));
    for window in running {
        if !seen.insert(window.app_name.clone()) {
            continue;
        }
        let display_name = apps
            .iter()
            .find(|app| app.running && app.pid == window.pid)
            .map(|app| app.name.clone())
            .unwrap_or_else(|| window.app_name.clone());
        list.push(json!({
            "id": window.app_name,
            "displayName": display_name,
            "isRunning": true,
        }));
    }
    for app in apps.iter().filter(|app| !app.running) {
        let Some(executable) = app.executable_name() else {
            continue;
        };
        if is_blocked(&executable, "") || !seen.insert(executable.clone()) {
            continue;
        }
        let mut entry = json!({
            "id": executable,
            "displayName": app.name,
            "isRunning": false,
        });
        if let Some(last_used) = &app.last_used {
            entry["lastUsedDate"] = json!(last_used);
        }
        list.push(entry);
    }
    list
}

fn element_tokens(elements: &[JsonValue]) -> HashMap<u64, String> {
    elements
        .iter()
        .filter_map(|element| {
            Some((
                element.get("element_index")?.as_u64()?,
                element.get("element_token")?.as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// The title from the tree's `- Window "…"` root line.
fn window_title(tree: &str) -> Option<String> {
    let line = tree.lines().map(str::trim).find(|line| line.starts_with("- Window "))?;
    let rest = line.strip_prefix("- Window ")?.trim();
    let quoted = rest.strip_prefix('"')?;
    let end = quoted.find('"')?;
    Some(quoted[..end].to_owned())
}

/// Mask the values of edit controls that look like secret fields, and any
/// value that is nothing but mask glyphs. Only the `value="…"` segment is
/// rewritten, so indices and labels stay put.
fn redact_secret_values(tree: &str, elements: &[JsonValue]) -> String {
    let mut redacted = tree.to_owned();
    for element in elements {
        let Some(value) = element.get("value").and_then(JsonValue::as_str) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let role = element
            .get("role")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let label = element
            .get("label")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let editable = matches!(
            role.as_str(),
            "edit" | "text" | "document" | "combobox" | "passwordbox" | "custom"
        );
        let secret = editable && looks_like_secret_label(&label);
        if !secret && !is_masked(value) {
            continue;
        }
        let needle = format!("value=\"{value}\"");
        redacted = redacted.replace(&needle, "value=\"<redacted>\"");
    }
    redacted
}

fn looks_like_secret_label(label: &str) -> bool {
    SECRET_LABEL_MARKERS.iter().any(|marker| {
        if *marker == "pin" || *marker == "token" {
            // Short markers must stand on their own: "pin" must not match
            // "spinner", "token" must not match "tokenizer".
            label
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word == *marker)
        } else {
            label.contains(marker)
        }
    })
}

fn is_masked(value: &str) -> bool {
    let mut glyphs = 0;
    for character in value.chars() {
        if character.is_whitespace() {
            continue;
        }
        if !matches!(character, '•' | '●' | '*' | '·' | '○') {
            return false;
        }
        glyphs += 1;
    }
    glyphs > 0
}

/// The macOS helper's three-way render: unchanged, or the lines that left
/// and arrived. Lines are compared as a multiset so that static text (which
/// the driver leaves unindexed) shows up too.
fn render_tree_diff(previous: &str, current: &str) -> String {
    if previous == current {
        let window = current
            .lines()
            .find(|line| line.starts_with("Window: "))
            .unwrap_or("the current window.");
        return format!("There has been no change in the accessibility tree for {window}");
    }
    let mut old_counts: HashMap<&str, usize> = HashMap::new();
    for line in previous.lines() {
        *old_counts.entry(line).or_default() += 1;
    }
    let mut new_counts: HashMap<&str, usize> = HashMap::new();
    for line in current.lines() {
        *new_counts.entry(line).or_default() += 1;
    }
    let mut changes = Vec::new();
    let mut remaining = new_counts.clone();
    for line in previous.lines() {
        match remaining.get_mut(line) {
            Some(count) if *count > 0 => *count -= 1,
            _ => changes.push(format!("- {line}")),
        }
    }
    let mut remaining = old_counts;
    for line in current.lines() {
        match remaining.get_mut(line) {
            Some(count) if *count > 0 => *count -= 1,
            _ => changes.push(format!("+ {line}")),
        }
    }
    format!("<accessibility_diff>\n{}\n</accessibility_diff>", changes.join("\n"))
}

/// The screenshot as a PNG data URL: the inline image block when the driver
/// embedded one, else the file it was told to write.
fn screenshot_data_url(result: &JsonValue, structured: &JsonValue) -> Option<String> {
    let inline = result
        .get("content")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .find(|block| block.get("type").and_then(JsonValue::as_str) == Some("image"))
        .and_then(|block| {
            let data = block.get("data")?.as_str()?;
            let mime = block
                .get("mimeType")
                .and_then(JsonValue::as_str)
                .unwrap_or("image/png");
            Some(format!("data:{mime};base64,{data}"))
        });
    if inline.is_some() {
        return inline;
    }
    let path = structured
        .get("screenshot_file_path")
        .and_then(JsonValue::as_str)?;
    let bytes = fs::read(path).ok()?;
    let _ = fs::remove_file(path);
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn refusal_code(result: &JsonValue) -> Option<&str> {
    result
        .pointer("/structuredContent/refusal/code")
        .or_else(|| result.pointer("/structuredContent/code"))
        .and_then(JsonValue::as_str)
}

fn refusal_text(result: &JsonValue) -> String {
    let text = text_content(result);
    if !text.trim().is_empty() {
        return text;
    }
    result
        .pointer("/structuredContent/refusal/message")
        .and_then(JsonValue::as_str)
        .unwrap_or("the Computer Use driver refused the request")
        .to_owned()
}

fn is_window_gone(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("No window with window_id") || text.contains("has no on-screen window")
}

#[cfg(windows)]
fn process_executable(pid: u32) -> Option<PathBuf> {
    sub2api::win_process::executable_path(pid)
}

#[cfg(windows)]
fn process_is_protected(pid: u32) -> bool {
    sub2api::win_process::is_protected(pid)
}

#[cfg(not(windows))]
fn process_executable(_: u32) -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn process_is_protected(_: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(pid: u64, id: u64, app: &str, title: &str, z: i64) -> WindowRow {
        WindowRow {
            pid,
            window_id: id,
            app_name: app.to_owned(),
            title: title.to_owned(),
            z_index: z,
            is_on_screen: true,
            minimized: false,
            area: 1000,
        }
    }

    fn app(name: &str, pid: u64, running: bool, launch_path: Option<&str>) -> AppRow {
        AppRow {
            name: name.to_owned(),
            pid,
            running,
            launch_path: launch_path.map(str::to_owned),
            last_used: None,
        }
    }

    #[test]
    fn keys_translate_xdotool_syntax_without_rewriting_super() {
        assert_eq!(translate_key("Return").unwrap(), ("return".into(), vec![]));
        assert_eq!(
            translate_key("ctrl+shift+t").unwrap(),
            ("t".into(), vec!["ctrl".into(), "shift".into()])
        );
        assert_eq!(translate_key("super+d").unwrap(), ("d".into(), vec!["win".into()]));
        assert_eq!(translate_key("KP_0").unwrap().0, "numpad0");
        assert_eq!(translate_key("Prior").unwrap().0, "pageup");
        assert_eq!(translate_key("BackSpace").unwrap().0, "backspace");
        assert_eq!(translate_key("ctrl++").unwrap(), ("+".into(), vec!["ctrl".into()]));
        assert!(translate_key("hyper+x").is_err());
        assert!(translate_key("").is_err());
    }

    #[test]
    fn scroll_pages_map_to_driver_granularity() {
        assert_eq!(scroll_amount(None).unwrap(), ("page", 1));
        assert_eq!(scroll_amount(Some(&json!(2))).unwrap(), ("page", 2));
        assert_eq!(scroll_amount(Some(&json!(0.5))).unwrap(), ("line", 2));
        assert_eq!(scroll_amount(Some(&json!(0.1))).unwrap(), ("line", 1));
        assert!(scroll_amount(Some(&json!(0))).is_err());
        assert_eq!(scroll_direction(Some(&json!("d"))).unwrap(), "down");
        assert!(scroll_direction(Some(&json!("sideways"))).is_err());
    }

    #[test]
    fn click_arguments_are_normalized_and_bounded() {
        assert_eq!(mouse_button(Some(&json!("r"))).unwrap(), "right");
        assert_eq!(mouse_button(None).unwrap(), "left");
        assert_eq!(click_count(Some(&json!(2))).unwrap(), 2);
        assert!(click_count(Some(&json!(4))).is_err());
        assert!(click_count(Some(&json!(1.5))).is_err());
    }

    #[test]
    fn secondary_actions_collapse_onto_clicks_and_menus() {
        assert!(matches!(
            secondary_action("Show Menu").unwrap(),
            SecondaryAction::Click("right")
        ));
        assert!(matches!(
            secondary_action("expand").unwrap(),
            SecondaryAction::Click("left")
        ));
        match secondary_action("menu: File > Save As…").unwrap() {
            SecondaryAction::Menu(path) => assert_eq!(path, vec!["File", "Save As…"]),
            _ => panic!("expected a menu path"),
        }
        assert!(secondary_action("increment").is_err());
        assert!(secondary_action("menu:").is_err());
    }

    #[test]
    fn the_blocklist_covers_credential_stores_terminals_and_hosted_settings() {
        assert!(is_blocked("1Password.exe", "1Password"));
        assert!(is_blocked("WindowsTerminal.exe", "PowerShell"));
        assert!(is_blocked("waku.exe", "CheapRouter"));
        assert!(is_blocked("ApplicationFrameHost.exe", "Settings"));
        assert!(is_blocked("ApplicationFrameHost.exe", "设置"));
        assert!(!is_blocked("ApplicationFrameHost.exe", "Calculator"));
        assert!(!is_blocked("notepad.exe", "Untitled - Notepad"));
    }

    #[test]
    fn windows_resolve_by_executable_then_start_menu_name_then_title() {
        let windows = vec![
            window(1, 10, "chrome.exe", "GitHub - Google Chrome", 3),
            window(1, 11, "chrome.exe", "Settings - Google Chrome", 1),
            window(2, 20, "notepad.exe", "Untitled - Notepad", 2),
            window(3, 30, "explorer.exe", "Program Manager", 0),
        ];
        let apps = vec![
            app("Google Chrome", 1, true, Some("C:\\Chrome\\chrome.exe")),
            app("Notepad", 2, true, Some("%windir%\\system32\\notepad.exe")),
            app("Microsoft Edge", 0, false, Some("C:\\Edge\\msedge.exe")),
        ];

        let by_exe = match_windows_by_executable("Chrome", &windows);
        assert_eq!(by_exe.len(), 2);
        assert_eq!(pick_window(&by_exe).unwrap().window_id, 10);
        assert_eq!(
            match_windows_by_executable("notepad.exe", &windows)[0].window_id,
            20
        );
        assert!(match_windows_by_executable("explorer", &windows).is_empty());

        let by_name = match_windows_by_app_name("google chrome", &apps, &windows, true);
        assert_eq!(by_name.len(), 2);
        assert!(match_windows_by_app_name("chrom", &apps, &windows, true).is_empty());
        assert_eq!(
            match_windows_by_app_name("chrom", &apps, &windows, false).len(),
            2
        );

        let by_title = match_windows_by_title("github", &windows);
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].window_id, 10);

        let launch = launch_argument("microsoft edge", "Microsoft Edge", &apps).unwrap();
        assert_eq!(launch["launch_path"], "C:\\Edge\\msedge.exe");
        assert!(launch_argument("msedge", "msedge", &apps).is_some());
        assert!(launch_argument("word", "Word", &apps).is_none());
    }

    #[test]
    fn the_app_list_names_running_windows_then_installed_apps() {
        let windows = vec![
            window(1, 10, "chrome.exe", "GitHub - Google Chrome", 3),
            window(1, 11, "chrome.exe", "Settings - Google Chrome", 1),
            window(4, 40, "WindowsTerminal.exe", "PowerShell", 5),
            window(3, 30, "explorer.exe", "Program Manager", 0),
        ];
        let apps = vec![
            app("Google Chrome", 1, true, Some("C:\\Chrome\\chrome.exe")),
            app("Microsoft Edge", 0, false, Some("C:\\Edge\\msedge.exe")),
            app("1Password", 0, false, Some("C:\\1P\\1Password.exe")),
            app("Some Store App", 0, false, Some("shell:appsFolder\\Pkg!App")),
        ];
        let list = app_list(&windows, &apps);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "chrome.exe");
        assert_eq!(list[0]["displayName"], "Google Chrome");
        assert_eq!(list[0]["isRunning"], true);
        assert_eq!(list[1]["id"], "msedge.exe");
        assert_eq!(list[1]["isRunning"], false);
    }

    #[test]
    fn secret_values_are_masked_without_moving_indices() {
        let tree = "- Window \"Sign in\"\n  - [0] Edit \"Email\" [value=\"me@example.com\"]\n  - [1] Edit \"Password\" [value=\"hunter2\" actions=[set_value]]\n  - [2] Edit \"Card PIN\" [value=\"1234\"]\n  - [3] Edit \"Spinner\" [value=\"1234\"]\n  - [4] Edit \"Other\" [value=\"••••\"]";
        let elements = vec![
            json!({"element_index": 0, "role": "Edit", "label": "Email", "value": "me@example.com"}),
            json!({"element_index": 1, "role": "Edit", "label": "Password", "value": "hunter2"}),
            json!({"element_index": 2, "role": "Edit", "label": "Card PIN", "value": "1234"}),
            json!({"element_index": 3, "role": "Edit", "label": "Spinner", "value": "1234"}),
            json!({"element_index": 4, "role": "Edit", "label": "Other", "value": "••••"}),
        ];
        let redacted = redact_secret_values(tree, &elements);
        assert!(redacted.contains("[0] Edit \"Email\" [value=\"me@example.com\"]"));
        assert!(redacted.contains("[1] Edit \"Password\" [value=\"<redacted>\""));
        assert!(!redacted.contains("hunter2"));
        // The PIN's value is masked wherever that exact segment appears, which
        // takes the identical Spinner value with it; indices are untouched.
        assert!(redacted.contains("[2] Edit \"Card PIN\" [value=\"<redacted>\"]"));
        assert!(redacted.contains("[3] Edit \"Spinner\""));
        assert!(redacted.contains("[4] Edit \"Other\" [value=\"<redacted>\"]"));
        assert_eq!(window_title(tree).as_deref(), Some("Sign in"));
    }

    #[test]
    fn the_diff_reports_no_change_or_the_lines_that_moved() {
        let first = "Window: \"A\"\n- Window \"A\"\n  - [0] Edit \"Text\" [value=\"\"]\n  - Text \"Ln 1\"";
        let same = render_tree_diff(first, first);
        assert!(same.starts_with("There has been no change in the accessibility tree for Window: \"A\""));

        let second = "Window: \"A\"\n- Window \"A\"\n  - [0] Edit \"Text\" [value=\"hi\"]\n  - Text \"Ln 2\"";
        let diff = render_tree_diff(first, second);
        assert!(diff.starts_with("<accessibility_diff>\n"));
        assert!(diff.contains("- \n  - [0] Edit \"Text\" [value=\"\"]".trim_start_matches("- \n")));
        assert!(diff.contains("-   - [0] Edit \"Text\" [value=\"\"]"));
        assert!(diff.contains("+   - [0] Edit \"Text\" [value=\"hi\"]"));
        assert!(diff.contains("-   - Text \"Ln 1\""));
        assert!(diff.contains("+   - Text \"Ln 2\""));
        assert!(!diff.contains("Window \"A\"\n+"));
    }

    #[test]
    fn screenshots_come_from_the_inline_image_block() {
        let result = json!({
            "content": [
                {"type": "text", "text": "tree"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png"}
            ]
        });
        assert_eq!(
            screenshot_data_url(&result, &JsonValue::Null).as_deref(),
            Some("data:image/png;base64,AAAA")
        );
        assert_eq!(screenshot_data_url(&json!({}), &JsonValue::Null), None);
    }

    /// The driver's paths are Windows paths on every host; these tests run
    /// on macOS too, where `Path` would not split them.
    #[test]
    fn driver_paths_split_on_either_separator_on_any_host() {
        assert_eq!(windows_file_name("%windir%\\system32\\notepad.exe"), "notepad.exe");
        assert_eq!(windows_file_name("C:/Tools/app.exe"), "app.exe");
        assert_eq!(windows_file_name("chrome"), "chrome");
        assert_eq!(windows_file_name("C:\\Tools\\"), "");
    }

    #[test]
    fn app_rows_expose_their_executable_name() {
        assert_eq!(
            app("Notepad", 0, false, Some("%windir%\\system32\\notepad.exe")).executable_name(),
            Some("notepad.exe".into())
        );
        assert_eq!(
            app("Edge", 0, false, Some("\"C:\\Program Files\\Edge\\msedge.exe\" --flag"))
                .executable_name(),
            None
        );
        assert_eq!(
            app("Store", 0, false, Some("shell:appsFolder\\Pkg!App")).executable_name(),
            None
        );
    }
}
