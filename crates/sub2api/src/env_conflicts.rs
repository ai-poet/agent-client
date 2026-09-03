//! Environment variables that would override the routing this app writes.
//!
//! Fork addition, ported from cc-switch's `env_checker`. The desktop routes a
//! CLI by editing that CLI's own configuration file; an `ANTHROPIC_BASE_URL`
//! exported from a shell profile or set in the Windows user environment can
//! silently outrank that file, and the user then sees requests go somewhere
//! else with no hint why. This module only *finds* such variables — the
//! Providers page shows them with their source and leaves removal to the
//! user, since editing shell profiles and the registry from a settings page
//! is a good way to break someone's setup.

#[cfg(not(target_os = "windows"))]
use std::path::Path;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a conflicting variable was found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictSource {
    /// The running process's environment — the same thing a spawned CLI
    /// would inherit.
    Process,
    /// `HKCU\Environment` on Windows.
    WindowsUser,
    /// The machine-wide environment block on Windows.
    WindowsMachine,
    /// An `export` line in a shell startup file (1-based line).
    ShellFile { path: PathBuf, line: usize },
}

/// One variable that outranks a written configuration file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvConflict {
    /// The CLI whose routing it affects.
    pub provider_id: &'static str,
    pub name: String,
    /// The value, with secrets reduced to a recognisable stub.
    pub value_masked: String,
    pub source: ConflictSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Watch {
    Exact(&'static str),
    Prefix(&'static str),
}

/// The variables each CLI honours over its configuration file.
///
/// Claude Code reads every `ANTHROPIC_*` variable; Codex takes its key and
/// base URL from `OPENAI_*`; Grok Build reads only these two. Names outside
/// this list are ignored even when they look related, so `MY_ANTHROPIC_KEY`
/// never trips the warning.
fn watched(provider_id: &str) -> &'static [Watch] {
    match provider_id {
        "claude" => &[Watch::Prefix("ANTHROPIC_")],
        "codex" => &[
            Watch::Exact("OPENAI_API_KEY"),
            Watch::Exact("OPENAI_BASE_URL"),
        ],
        "grok" => &[
            Watch::Exact("XAI_API_KEY"),
            Watch::Exact("GROK_DEFAULT_MODEL"),
        ],
        _ => &[],
    }
}

const WATCHED_PROVIDERS: [&str; 3] = ["claude", "codex", "grok"];

/// Which CLI `name` affects, if any.
pub fn provider_for_variable(name: &str) -> Option<&'static str> {
    let upper = name.trim().to_ascii_uppercase();
    WATCHED_PROVIDERS.into_iter().find(|provider| {
        watched(provider).iter().any(|watch| match watch {
            Watch::Exact(exact) => upper == *exact,
            Watch::Prefix(prefix) => upper.starts_with(prefix),
        })
    })
}

/// Build a conflict for `name` when a CLI honours it; `None` otherwise.
/// Empty values are not conflicts — an unset export overrides nothing.
pub fn classify(name: &str, value: &str, source: ConflictSource) -> Option<EnvConflict> {
    if value.trim().is_empty() {
        return None;
    }
    let provider_id = provider_for_variable(name)?;
    Some(EnvConflict {
        provider_id,
        name: name.trim().to_owned(),
        value_masked: mask(name, value),
        source,
    })
}

/// Keys and tokens are reduced to their edges; anything else (a base URL,
/// a model name) is shown whole, since that is what the user needs to see
/// to recognise it.
pub fn mask(name: &str, value: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let secret = upper.contains("KEY") || upper.contains("TOKEN") || upper.contains("SECRET");
    let value = value.trim();
    if !secret {
        return value.to_owned();
    }
    if value.chars().count() <= 12 {
        return "***".to_owned();
    }
    let head: String = value.chars().take(6).collect();
    let tail: String = value
        .chars()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// Everything found, most actionable source first. A variable present both
/// in a persisted source and in the process environment is reported once,
/// under the persisted source — that is where it has to be removed.
pub fn scan() -> Vec<EnvConflict> {
    let mut conflicts = Vec::new();
    #[cfg(target_os = "windows")]
    conflicts.extend(scan_windows_environment());
    #[cfg(not(target_os = "windows"))]
    conflicts.extend(scan_shell_files(dirs::home_dir().as_deref()));

    for (name, value) in std::env::vars() {
        if conflicts
            .iter()
            .any(|known| known.name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        conflicts.extend(classify(&name, &value, ConflictSource::Process));
    }
    conflicts
}

/// The user and machine environment blocks, read through PowerShell so no
/// registry dependency is needed. Best-effort: no PowerShell, no result.
#[cfg(target_os = "windows")]
fn scan_windows_environment() -> Vec<EnvConflict> {
    use std::process::Command;

    let mut conflicts = Vec::new();
    for (scope, source) in [
        ("User", ConflictSource::WindowsUser),
        ("Machine", ConflictSource::WindowsMachine),
    ] {
        let script = format!(
            "[Environment]::GetEnvironmentVariables('{scope}') | ConvertTo-Json -Compress"
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let run = crate::cli_detect::run_with_timeout(command, std::time::Duration::from_secs(5));
        if !run.success {
            continue;
        }
        let Ok(serde_json::Value::Object(map)) =
            serde_json::from_str::<serde_json::Value>(run.output.trim())
        else {
            continue;
        };
        for (name, value) in map {
            let Some(value) = value.as_str() else {
                continue;
            };
            if conflicts
                .iter()
                .any(|known: &EnvConflict| known.name.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            conflicts.extend(classify(&name, value, source.clone()));
        }
    }
    conflicts
}

/// The startup files a login or interactive shell reads.
#[cfg(not(target_os = "windows"))]
fn shell_startup_files(home: Option<&Path>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(home) = home {
        for name in [".bashrc", ".bash_profile", ".zshrc", ".zprofile", ".zshenv", ".profile"] {
            files.push(home.join(name));
        }
        files.push(home.join(".config/fish/config.fish"));
    }
    files.push(PathBuf::from("/etc/profile"));
    files.push(PathBuf::from("/etc/zshrc"));
    files
}

#[cfg(not(target_os = "windows"))]
fn scan_shell_files(home: Option<&Path>) -> Vec<EnvConflict> {
    let mut conflicts = Vec::new();
    for path in shell_startup_files(home) {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (name, value, line) in parse_shell_exports(&contents) {
            if conflicts
                .iter()
                .any(|known: &EnvConflict| known.name.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            conflicts.extend(classify(
                &name,
                &value,
                ConflictSource::ShellFile {
                    path: path.clone(),
                    line,
                },
            ));
        }
    }
    conflicts
}

/// `export NAME=value`, `NAME=value`, and fish's `set -gx NAME value` lines,
/// as `(name, value, 1-based line)`. Comments and indented continuation
/// noise are skipped; quotes around the value are stripped.
pub fn parse_shell_exports(contents: &str) -> Vec<(String, String, usize)> {
    let mut found = Vec::new();
    for (index, raw) in contents.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("set ") {
            // fish: set -gx NAME value
            let mut parts = rest.split_whitespace().skip_while(|part| part.starts_with('-'));
            if let (Some(name), Some(value)) = (parts.next(), parts.next())
                && is_variable_name(name)
            {
                found.push((name.to_owned(), unquote(value), index + 1));
            }
            continue;
        }
        let assignment = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = assignment.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !is_variable_name(name) {
            continue;
        }
        found.push((name.to_owned(), unquote(value.trim()), index + 1));
    }
    found
}

fn is_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        && !name.starts_with(|character: char| character.is_ascii_digit())
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let value = value.split_once(" #").map(|(head, _)| head.trim()).unwrap_or(value);
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')))
        .unwrap_or(value)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_matches_only_the_watched_names() {
        assert_eq!(provider_for_variable("ANTHROPIC_BASE_URL"), Some("claude"));
        assert_eq!(provider_for_variable("anthropic_api_key"), Some("claude"));
        assert_eq!(provider_for_variable("OPENAI_API_KEY"), Some("codex"));
        assert_eq!(provider_for_variable("XAI_API_KEY"), Some("grok"));
        assert_eq!(provider_for_variable("MY_ANTHROPIC_API_KEY"), None);
        assert_eq!(provider_for_variable("OPENAI_ORG_ID"), None);
        assert_eq!(provider_for_variable("GROK_HOME"), None);
        assert!(classify("ANTHROPIC_BASE_URL", "   ", ConflictSource::Process).is_none());
    }

    #[test]
    fn secrets_are_masked_and_urls_are_not() {
        assert_eq!(
            mask("ANTHROPIC_BASE_URL", "https://gw.example.org"),
            "https://gw.example.org"
        );
        assert_eq!(mask("ANTHROPIC_API_KEY", "sk-ant-1234567890abcdef"), "sk-ant…def");
        assert_eq!(mask("OPENAI_API_KEY", "short"), "***");
        let conflict = classify(
            "ANTHROPIC_AUTH_TOKEN",
            "abcdefghijklmnopqrstuvwxyz",
            ConflictSource::WindowsUser,
        )
        .expect("conflict");
        assert_eq!(conflict.provider_id, "claude");
        assert_eq!(conflict.value_masked, "abcdef…xyz");
    }

    #[test]
    fn shell_file_scan_finds_exports_and_ignores_comments() {
        let contents = "\
# export ANTHROPIC_BASE_URL=https://commented.example.org
export ANTHROPIC_BASE_URL=\"https://gw.example.org\"
  OPENAI_API_KEY='sk-openai-123' # trailing note
set -gx XAI_API_KEY xai-fish-key
alias ll='ls -l'
export PATH=$PATH:/opt/bin
";
        let exports = parse_shell_exports(contents);
        assert_eq!(
            exports,
            vec![
                (
                    "ANTHROPIC_BASE_URL".to_owned(),
                    "https://gw.example.org".to_owned(),
                    2
                ),
                ("OPENAI_API_KEY".to_owned(), "sk-openai-123".to_owned(), 3),
                ("XAI_API_KEY".to_owned(), "xai-fish-key".to_owned(), 4),
                ("PATH".to_owned(), "$PATH:/opt/bin".to_owned(), 6),
            ]
        );
        let conflicts: Vec<_> = exports
            .into_iter()
            .filter_map(|(name, value, line)| {
                classify(
                    &name,
                    &value,
                    ConflictSource::ShellFile {
                        path: PathBuf::from("/home/u/.zshrc"),
                        line,
                    },
                )
            })
            .collect();
        assert_eq!(conflicts.len(), 3);
        assert_eq!(conflicts[0].provider_id, "claude");
        assert_eq!(conflicts[2].provider_id, "grok");
    }
}
