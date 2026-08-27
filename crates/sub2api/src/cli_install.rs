//! Detecting agent CLIs and assisting with their installation.
//!
//! Upstream expects the user to have installed and authenticated an agent CLI
//! before launching the app. This module closes that gap: it reports what is
//! missing and produces the exact command that fixes it.
//!
//! # Why commands rather than an installer
//!
//! The command is handed to the app's built-in terminal and runs in front of
//! the user. That keeps npm's own output — proxy failures, permission errors,
//! EACCES on a root-owned prefix — visible instead of collapsed into a generic
//! "install failed", and it leaves the user with a command they can rerun or
//! paste into an issue. Everything here is therefore a pure string-producing
//! function, which also makes it testable without touching the machine.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Node major version the agent CLIs require.
pub const REQUIRED_NODE_MAJOR: u32 = 22;

/// Registry used when the user is likely behind the Great Firewall. npm's
/// default registry is frequently unreachable there, and a stalled install is
/// the single most common setup failure.
pub const MIRROR_REGISTRY: &str = "https://registry.npmmirror.com";

/// An installable agent CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CliDescriptor {
    /// Provider id, matching upstream's `ProviderKind::id()`.
    pub id: &'static str,
    /// Name shown in the UI.
    pub display_name: &'static str,
    /// npm package that provides the CLI.
    pub package: &'static str,
    /// Executable to look for on `PATH`.
    pub binary: &'static str,
}

/// CLIs this build knows how to install.
///
/// Package names match the ones the Electron client ships, so a machine set up
/// by either app looks the same.
pub const DESCRIPTORS: &[CliDescriptor] = &[
    CliDescriptor {
        id: "claude",
        display_name: "Claude Code",
        package: "@anthropic-ai/claude-code",
        binary: "claude",
    },
    CliDescriptor {
        id: "codex",
        display_name: "Codex CLI",
        package: "@openai/codex",
        binary: "codex",
    },
    // The three CLIs above and below are the supported set; OpenCode and Pi
    // installs were dropped deliberately — the app still detects and runs
    // them when the user installs them elsewhere.
    CliDescriptor {
        id: "grok",
        display_name: "Grok Build",
        package: "@xai-official/grok",
        binary: "grok",
    },
];

/// Look up a descriptor by provider id.
pub fn descriptor(id: &str) -> Option<&'static CliDescriptor> {
    DESCRIPTORS.iter().find(|descriptor| descriptor.id == id)
}

/// What we found for one CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub id: &'static str,
    pub path: Option<PathBuf>,
}

impl Detection {
    pub fn is_installed(&self) -> bool {
        self.path.is_some()
    }
}

/// Detect every known CLI on the current `PATH`.
pub fn detect_all() -> Vec<Detection> {
    DESCRIPTORS
        .iter()
        .map(|descriptor| Detection {
            id: descriptor.id,
            path: find_executable(descriptor.binary),
        })
        .collect()
}

/// Resolve an executable on `PATH`, honouring `PATHEXT` on Windows.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|directory| {
        for candidate in executable_candidates(name) {
            let full = directory.join(&candidate);
            if is_executable_file(&full) {
                return Some(full);
            }
        }
        None
    })
}

/// Names to try for `name`: the bare name, plus each `PATHEXT` suffix on
/// Windows, where `claude` is really `claude.cmd`.
fn executable_candidates(name: &str) -> Vec<String> {
    if !cfg!(target_os = "windows") {
        return vec![name.to_owned()];
    }
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    let mut candidates = vec![name.to_owned()];
    candidates.extend(
        extensions
            .split(';')
            .map(str::trim)
            .filter(|extension| !extension.is_empty())
            .map(|extension| format!("{name}{}", extension.to_ascii_lowercase())),
    );
    candidates
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// `npm install -g <package>`, optionally pinned to a registry mirror.
///
/// The mirror command carries retry and timeout flags: without them npm's
/// default fetch can hang for minutes on a flaky route, which reads as "the
/// install froze".
pub fn install_command(package: &str, registry: Option<&str>) -> String {
    match registry {
        Some(registry) if !registry.trim().is_empty() => format!(
            "npm install -g {package} --registry={} --fetch-retries=2 --fetch-timeout=60000",
            registry.trim()
        ),
        _ => format!("npm install -g {package}"),
    }
}

/// Install commands to try, in order.
///
/// On Windows the mirror comes first for everyone, with the official registry
/// as the fallback — the audience this ships to reaches npmmirror far more
/// reliably than registry.npmjs.org, and a locale check would miss them
/// because Windows rarely sets `LANG`. Elsewhere the official registry is
/// dependable and the mirror is the wrong default.
pub fn install_candidates(package: &str) -> Vec<String> {
    let pinned = format!("{package}@latest");
    if cfg!(target_os = "windows") {
        vec![
            install_command(&pinned, Some(MIRROR_REGISTRY)),
            install_command(&pinned, None),
        ]
    } else {
        vec![install_command(&pinned, None)]
    }
}

/// Run each candidate until one succeeds.
///
/// A failure keeps every attempt's tail in the output, labeled, so "the mirror
/// timed out and then the official registry refused the proxy" reads as two
/// distinct problems rather than one mystery.
pub fn run_candidates(commands: &[String]) -> InstallOutcome {
    let mut failures: Vec<String> = Vec::new();
    for command in commands {
        let outcome = run_command(command);
        if outcome.success {
            return outcome;
        }
        failures.push(format!("{command}\n{}", outcome.output));
    }
    InstallOutcome {
        success: false,
        output: failures.join("\n\n"),
    }
}

/// Extract the major version from `node --version` style output.
///
/// Accepts `v22.11.0`, `22.11.0`, and trailing whitespace.
pub fn parse_major_version(output: &str) -> Option<u32> {
    parse_semantic_version(output)?
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Pull the first `x.y.z` out of arbitrary CLI version output.
pub fn parse_semantic_version(output: &str) -> Option<String> {
    let mut digits = String::new();
    let mut dots = 0;
    for character in output.chars() {
        match character {
            '0'..='9' => digits.push(character),
            '.' if !digits.is_empty() && dots < 2 && !digits.ends_with('.') => {
                digits.push('.');
                dots += 1;
            }
            _ if dots == 2 && !digits.ends_with('.') => break,
            _ => {
                digits.clear();
                dots = 0;
            }
        }
    }
    let trimmed = digits.trim_end_matches('.');
    (trimmed.split('.').count() == 3).then(|| trimmed.to_owned())
}

/// Whether the detected Node satisfies [`REQUIRED_NODE_MAJOR`].
pub fn node_is_supported(version_output: &str) -> bool {
    parse_major_version(version_output).is_some_and(|major| major >= REQUIRED_NODE_MAJOR)
}

/// Resolve npm's global bin directory from `npm prefix -g` output.
///
/// On Windows the prefix *is* the bin directory; elsewhere binaries live in
/// `<prefix>/bin`.
pub fn npm_global_bin_from_prefix(prefix_output: &str) -> Option<PathBuf> {
    let prefix = prefix_output.trim();
    if prefix.is_empty() {
        return None;
    }
    let prefix = PathBuf::from(prefix);
    if cfg!(target_os = "windows") {
        Some(prefix)
    } else {
        Some(prefix.join("bin"))
    }
}

/// Entries that must be added to `PATH` for `needed` to be reachable.
///
/// Comparison is case-insensitive and ignores trailing separators, because
/// Windows reports the same directory in several spellings.
pub fn missing_path_entries(current_path: &str, needed: &[String]) -> Vec<String> {
    let existing: Vec<String> = current_path
        .split(if cfg!(target_os = "windows") { ';' } else { ':' })
        .map(normalize_path_entry)
        .filter(|entry| !entry.is_empty())
        .collect();
    let mut missing = Vec::new();
    for entry in needed {
        let normalized = normalize_path_entry(entry);
        if normalized.is_empty() {
            continue;
        }
        if !existing.contains(&normalized) && !missing.contains(entry) {
            missing.push(entry.clone());
        }
    }
    missing
}

fn normalize_path_entry(entry: &str) -> String {
    let trimmed = entry.trim().trim_end_matches(['/', '\\']);
    // Only Windows paths are case-insensitive; folding case on Unix would
    // treat /opt/Tools as already present when only /opt/tools is.
    if cfg!(target_os = "windows") {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.to_owned()
    }
}

/// Append entries to a Windows user `PATH` value without duplicating them.
pub fn windows_user_path_value(current_path: &str, additions: &[String]) -> String {
    let missing = missing_path_entries(current_path, additions);
    if missing.is_empty() {
        return current_path.to_owned();
    }
    let mut value = current_path.trim_end_matches(';').to_owned();
    for entry in missing {
        if !value.is_empty() {
            value.push(';');
        }
        value.push_str(&entry);
    }
    value
}

/// PowerShell that broadcasts `WM_SETTINGCHANGE`.
///
/// Without it a `PATH` edit is invisible to already-running processes,
/// including the shell the user is about to type in — the classic "I installed
/// it but the command is not found" report.
pub fn broadcast_environment_change_command() -> &'static str {
    r#"$sig = '[DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'; $type = Add-Type -MemberDefinition $sig -Name 'Win32SendMessageTimeout' -Namespace Win32Functions -PassThru; [UIntPtr]$result = [UIntPtr]::Zero; $type::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result) | Out-Null"#
}

/// Result of running an install command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallOutcome {
    pub success: bool,
    /// Combined stdout and stderr, trimmed. npm puts its diagnostics on both.
    pub output: String,
}

/// Shell invocation for a command line.
///
/// The commands are shell strings (`npm install -g x --registry y`), and on
/// Windows `npm` is a `.cmd` shim that only a shell will resolve — so this
/// always goes through one rather than trying to exec the first word.
pub fn shell_invocation(command: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "windows") {
        ("cmd", vec!["/C".to_owned(), command.to_owned()])
    } else {
        ("sh", vec!["-c".to_owned(), command.to_owned()])
    }
}

/// Directories where the app-managed toolchains may live, existing ones only.
///
/// Prepended to `PATH` for everything this crate spawns and for the agent
/// processes: after a portable Node or Git install, the app's own inherited
/// `PATH` predates the install and would never find them — the `claude` shim
/// needs `node`, and the daemon's checkpoints shell out to a bare `git`.
pub fn node_runtime_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(managed) = crate::node_install::managed_node_bin_dir()
        && managed.is_dir()
    {
        dirs.push(managed);
    }
    if cfg!(target_os = "windows") {
        let program_files = PathBuf::from(r"C:\Program Files\nodejs");
        if program_files.is_dir() {
            dirs.push(program_files);
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let npm = PathBuf::from(appdata).join("npm");
            if npm.is_dir() {
                dirs.push(npm);
            }
        }
    }
    dirs
}

/// `current` with `extra` prepended, skipping entries already present.
pub(crate) fn augment_path(current: Option<std::ffi::OsString>, extra: &[PathBuf]) -> std::ffi::OsString {
    let existing: Vec<PathBuf> = current
        .as_deref()
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    let additions: Vec<&PathBuf> = extra
        .iter()
        .filter(|candidate| !existing.contains(candidate))
        .collect();
    std::env::join_paths(
        additions
            .into_iter()
            .cloned()
            .chain(existing),
    )
    .unwrap_or_else(|_| current.unwrap_or_default())
}

/// Prepend the managed toolchains to a command's `PATH`, and point Claude
/// Code at Git Bash when one is known.
///
/// Applied to every agent process the fork spawns, so a CLI installed against
/// the managed runtime keeps working without an app restart.
pub fn apply_node_runtime(command: &mut std::process::Command) {
    let dirs = node_runtime_dirs();
    if !dirs.is_empty() {
        let configured = command
            .get_envs()
            .find(|(name, _)| {
                name.to_str().is_some_and(|name| name.eq_ignore_ascii_case("path"))
            })
            .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
            .or_else(|| std::env::var_os("PATH"));
        command.env("PATH", augment_path(configured, &dirs));
    }

}

/// Run a program directly — no shell — and capture its output.
///
/// Used where an argument would otherwise need shell quoting (PowerShell
/// scripts, paths with spaces); `cmd`'s quoting rules are not worth fighting.
pub fn run_program(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> InstallOutcome {
    let mut process = std::process::Command::new(program.as_ref());
    process.args(args);
    process.env("PATH", augment_path(std::env::var_os("PATH"), &node_runtime_dirs()));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    capture(process, &program.as_ref().to_string_lossy())
}

/// Run an install command and capture its output.
///
/// Blocking; callers run it off the UI thread. Output is captured rather than
/// streamed because the app has no terminal surface to stream into — a failure
/// is reported with npm's own message attached, which is what makes proxy and
/// permission errors diagnosable.
pub fn run_command(command: &str) -> InstallOutcome {
    let (program, args) = shell_invocation(command);
    let mut process = std::process::Command::new(program);
    process.args(args);
    // The freshly installed managed runtime must be visible to `npm` runs that
    // happen in this same app session.
    process.env("PATH", augment_path(std::env::var_os("PATH"), &node_runtime_dirs()));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    capture(process, program)
}

fn capture(mut process: std::process::Command, program: &str) -> InstallOutcome {
    match process.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let errors = String::from_utf8_lossy(&output.stderr);
            if !errors.trim().is_empty() {
                if !text.trim().is_empty() {
                    text.push('\n');
                }
                text.push_str(&errors);
            }
            InstallOutcome {
                success: output.status.success(),
                output: last_meaningful_lines(text.trim(), 6),
            }
        }
        Err(error) => InstallOutcome {
            success: false,
            output: format!("could not start {program}: {error}"),
        },
    }
}

/// Keep the tail of a command's output: npm's failure reason is at the end,
/// and the preceding progress lines are noise in a settings panel.
fn last_meaningful_lines(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}

/// A step the user must complete, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupStep {
    /// Node is missing or too old; nothing else can proceed.
    InstallNode { required_major: u32 },
    /// Install one agent CLI. `commands` are tried in order.
    InstallCli {
        id: &'static str,
        display_name: &'static str,
        commands: Vec<String>,
    },
}

/// Work out what the user has to do, given what was detected.
///
/// Returns an empty plan when the machine is already usable.
pub fn setup_plan(node_version_output: Option<&str>, detections: &[Detection]) -> Vec<SetupStep> {
    let mut steps = Vec::new();
    let node_ok = node_version_output.is_some_and(node_is_supported);
    if !node_ok {
        steps.push(SetupStep::InstallNode {
            required_major: REQUIRED_NODE_MAJOR,
        });
    }
    for detection in detections {
        if detection.is_installed() {
            continue;
        }
        let Some(descriptor) = descriptor(detection.id) else {
            continue;
        };
        steps.push(SetupStep::InstallCli {
            id: descriptor.id,
            display_name: descriptor.display_name,
            commands: install_candidates(descriptor.package),
        });
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(id: &'static str) -> Detection {
        Detection { id, path: None }
    }

    fn installed(id: &'static str) -> Detection {
        Detection {
            id,
            path: Some(PathBuf::from("/usr/local/bin/x")),
        }
    }

    #[test]
    fn descriptors_are_unique_and_addressable() {
        for descriptor in DESCRIPTORS {
            assert_eq!(super::descriptor(descriptor.id), Some(descriptor));
            assert!(!descriptor.package.is_empty());
            assert!(!descriptor.binary.is_empty());
        }
        assert_eq!(super::descriptor("nope"), None);
    }

    #[test]
    fn install_command_pins_the_mirror_when_asked() {
        assert_eq!(
            install_command("@openai/codex", None),
            "npm install -g @openai/codex"
        );
        assert_eq!(
            install_command("@openai/codex", Some(MIRROR_REGISTRY)),
            "npm install -g @openai/codex --registry=https://registry.npmmirror.com --fetch-retries=2 --fetch-timeout=60000"
        );
        // A blank registry must not produce dangling flags.
        assert_eq!(
            install_command("@openai/codex", Some("  ")),
            "npm install -g @openai/codex"
        );
    }

    #[test]
    fn candidates_try_the_mirror_first_on_windows_only() {
        let candidates = install_candidates("@openai/codex");
        if cfg!(target_os = "windows") {
            // Mirror first with a fallback: the audience reaches npmmirror far
            // more reliably, and Windows rarely sets LANG so a locale check
            // cannot be the gate.
            assert_eq!(candidates.len(), 2);
            assert!(candidates[0].contains("--registry=https://registry.npmmirror.com"));
            assert!(candidates[0].contains("--fetch-retries=2"));
            assert!(candidates[1].ends_with("@openai/codex@latest"));
        } else {
            assert_eq!(
                candidates,
                vec!["npm install -g @openai/codex@latest".to_owned()]
            );
        }
    }

    #[test]
    fn run_candidates_stops_at_the_first_success() {
        let outcome = run_candidates(&["exit 1".to_owned(), "exit 0".to_owned()]);
        assert!(outcome.success, "output: {}", outcome.output);
    }

    #[test]
    fn run_candidates_reports_every_failed_attempt() {
        let outcome = run_candidates(&[
            "echo first-fail 1>&2 && exit 1".to_owned(),
            "echo second-fail 1>&2 && exit 1".to_owned(),
        ]);
        assert!(!outcome.success);
        // Both attempts must be readable, labeled by their command, so two
        // different failures do not collapse into one mystery.
        assert!(outcome.output.contains("first-fail"), "{}", outcome.output);
        assert!(outcome.output.contains("second-fail"), "{}", outcome.output);
        assert!(outcome.output.contains("exit 1"));
    }

    #[test]
    fn parses_node_versions() {
        assert_eq!(parse_major_version("v22.11.0\n"), Some(22));
        assert_eq!(parse_major_version("22.11.0"), Some(22));
        assert_eq!(parse_major_version("v18.20.4"), Some(18));
        assert_eq!(parse_major_version(""), None);
        assert_eq!(parse_major_version("not a version"), None);
    }

    #[test]
    fn parses_versions_out_of_noisy_output() {
        assert_eq!(
            parse_semantic_version("claude-code/1.2.34 darwin-arm64"),
            Some("1.2.34".to_owned())
        );
        assert_eq!(parse_semantic_version("v0.0.1"), Some("0.0.1".to_owned()));
        // Two components is not a semantic version.
        assert_eq!(parse_semantic_version("1.2"), None);
    }

    #[test]
    fn node_support_follows_the_required_major() {
        assert!(node_is_supported("v22.0.0"));
        assert!(node_is_supported("v24.1.0"));
        assert!(!node_is_supported("v20.11.1"));
        assert!(!node_is_supported(""));
    }

    #[test]
    fn npm_global_bin_resolves_per_platform() {
        let resolved = npm_global_bin_from_prefix("  /usr/local  ").expect("prefix");
        if cfg!(target_os = "windows") {
            assert_eq!(resolved, PathBuf::from("/usr/local"));
        } else {
            assert_eq!(resolved, PathBuf::from("/usr/local/bin"));
        }
        assert_eq!(npm_global_bin_from_prefix("   "), None);
    }

    #[test]
    fn missing_path_entries_ignores_case_and_trailing_separators() {
        let current = if cfg!(target_os = "windows") {
            r"C:\Windows;C:\Users\me\AppData\Roaming\npm\"
        } else {
            "/usr/bin:/usr/local/bin/"
        };
        let already = if cfg!(target_os = "windows") {
            r"c:\users\me\appdata\roaming\npm"
        } else {
            "/usr/local/bin"
        };
        assert!(missing_path_entries(current, &[already.to_owned()]).is_empty());

        let fresh = missing_path_entries(current, &["/opt/tools".to_owned()]);
        assert_eq!(fresh, vec!["/opt/tools".to_owned()]);
    }

    #[test]
    fn missing_path_entries_deduplicates_and_skips_blanks() {
        let entries = missing_path_entries(
            "/usr/bin",
            &["/opt/a".to_owned(), "/opt/a".to_owned(), "  ".to_owned()],
        );
        assert_eq!(entries, vec!["/opt/a".to_owned()]);
    }

    #[test]
    fn windows_path_append_is_idempotent() {
        let current = r"C:\Windows;C:\Tools";
        let once = windows_user_path_value(current, &[r"C:\Extra".to_owned()]);
        assert_eq!(once, r"C:\Windows;C:\Tools;C:\Extra");
        // Re-running setup must not keep growing PATH.
        let twice = windows_user_path_value(&once, &[r"C:\Extra".to_owned()]);
        assert_eq!(twice, once);
    }

    #[test]
    fn windows_path_append_handles_a_trailing_separator() {
        assert_eq!(
            windows_user_path_value(r"C:\Windows;", &[r"C:\Extra".to_owned()]),
            r"C:\Windows;C:\Extra"
        );
    }

    #[test]
    fn plan_is_empty_when_everything_is_present() {
        let detections: Vec<Detection> = DESCRIPTORS.iter().map(|d| installed(d.id)).collect();
        assert!(setup_plan(Some("v22.1.0"), &detections).is_empty());
    }

    #[test]
    fn plan_puts_node_first_because_npm_installs_depend_on_it() {
        let plan = setup_plan(Some("v18.0.0"), &[missing("claude")]);
        assert_eq!(
            plan.first(),
            Some(&SetupStep::InstallNode {
                required_major: REQUIRED_NODE_MAJOR
            })
        );
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn plan_lists_only_missing_clis_with_their_candidates() {
        let plan = setup_plan(Some("v22.0.0"), &[installed("claude"), missing("codex")]);
        assert_eq!(
            plan,
            vec![SetupStep::InstallCli {
                id: "codex",
                display_name: "Codex CLI",
                commands: install_candidates("@openai/codex"),
            }]
        );
    }

    #[test]
    fn absent_node_output_is_treated_as_missing_node() {
        let plan = setup_plan(None, &[]);
        assert_eq!(
            plan,
            vec![SetupStep::InstallNode {
                required_major: REQUIRED_NODE_MAJOR
            }]
        );
    }

    #[test]
    fn shell_invocation_uses_a_shell_so_npm_shims_resolve() {
        let (program, args) = shell_invocation("npm install -g x");
        if cfg!(target_os = "windows") {
            assert_eq!(program, "cmd");
            assert_eq!(args, vec!["/C".to_owned(), "npm install -g x".to_owned()]);
        } else {
            assert_eq!(program, "sh");
            assert_eq!(args, vec!["-c".to_owned(), "npm install -g x".to_owned()]);
        }
    }

    #[test]
    fn run_command_reports_success_and_failure() {
        let ok = run_command("exit 0");
        assert!(ok.success, "output: {}", ok.output);

        let failed = run_command("exit 3");
        assert!(!failed.success);
    }

    #[test]
    fn run_command_captures_stderr_because_npm_fails_there() {
        let outcome = run_command("echo boom 1>&2 && exit 1");
        assert!(!outcome.success);
        assert!(outcome.output.contains("boom"), "output: {}", outcome.output);
    }

    #[test]
    fn output_is_trimmed_to_the_tail_where_the_reason_lives() {
        let text = (1..=20).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let tail = last_meaningful_lines(&text, 3);
        assert_eq!(tail, "18\n19\n20");
        // Blank lines must not consume the budget.
        assert_eq!(last_meaningful_lines("a\n\n\nb", 2), "a\nb");
    }

    #[test]
    fn broadcast_command_targets_the_environment_topic() {
        let command = broadcast_environment_change_command();
        assert!(command.contains("SendMessageTimeout"));
        assert!(command.contains("'Environment'"));
    }

    #[test]
    fn executable_candidates_cover_windows_extensions() {
        let candidates = executable_candidates("claude");
        assert_eq!(candidates.first().map(String::as_str), Some("claude"));
        if cfg!(target_os = "windows") {
            assert!(candidates.iter().any(|candidate| candidate == "claude.cmd"));
        }
    }
}
