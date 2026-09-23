// PowerShell tool: execute PowerShell commands (Windows-native).
//
// On Windows, PowerShell provides richer scripting than cmd.exe.
// On non-Windows platforms, attempts to use `pwsh` (PowerShell Core).
//
// Security model
// ──────────────
// Before any execution the command is passed through `classify_ps_command`.
// The resulting `PsRiskLevel` drives the permission gate:
//
//   Critical → always blocked (hard error, never executed)
//   High     → requires explicit user approval (once / session / deny)
//   Medium   → requires approval only when ctx.require_confirmation is set
//   Low      → executes directly

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use claurst_core::ps_classifier::{PsRiskLevel, classify_ps_command};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::process::Command;
use tracing::debug;

pub struct PowerShellTool;

#[derive(Debug, Deserialize)]
struct PowerShellInput {
    command: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "default_timeout")]
    timeout: u64,
    /// When true, Medium-risk commands also prompt for approval.
    #[serde(default)]
    require_confirmation: bool,
}

fn default_timeout() -> u64 { 120_000 }

// ---------------------------------------------------------------------------
// Risk-label helpers (used in messages shown to the user)
// ---------------------------------------------------------------------------

fn risk_label(level: PsRiskLevel) -> &'static str {
    match level {
        PsRiskLevel::Critical => "Critical",
        PsRiskLevel::High     => "High",
        PsRiskLevel::Medium   => "Medium",
        PsRiskLevel::Low      => "Low",
    }
}

fn risk_explanation(level: PsRiskLevel, command: &str) -> String {
    match level {
        PsRiskLevel::Critical => format!(
            "PowerShell command classified as CRITICAL risk — execution blocked.\n\
             Reason: the command contains destructive or remote-code-execution patterns.\n\
             Command: {}",
            command
        ),
        PsRiskLevel::High => {
            "[High risk] This may modify system-wide security policy, the registry (HKLM), user accounts, or firewall rules.".to_string()
        }
        PsRiskLevel::Medium => {
            "[Medium risk] This may delete files, control services, or make network requests.".to_string()
        }
        PsRiskLevel::Low => String::new(), // never shown
    }
}

// ---------------------------------------------------------------------------
// Tool implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Tool for PowerShellTool {
    // Gates itself: calls `ctx.check_permission*` in `execute()` (#210).
    fn self_gates(&self) -> bool { true }

    fn name(&self) -> &str { "PowerShell" }

    fn description(&self) -> &str {
        "Execute a PowerShell command. Use for Windows-native operations, .NET APIs, \
         registry access, and Windows-specific system administration."
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Execute }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The PowerShell command or script to execute"
                },
                "description": {
                    "type": "string",
                    "description": "Description of what this command does"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in ms (default 120000, max 600000)"
                },
                "require_confirmation": {
                    "type": "boolean",
                    "description": "When true, Medium-risk commands also prompt for approval"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: PowerShellInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        // ── Step 1: classify the command ─────────────────────────────────────
        let risk = classify_ps_command(&params.command);

        // ── Step 2: apply the risk gate ──────────────────────────────────────
        match risk {
            PsRiskLevel::Critical => {
                // Hard block — never executed regardless of permission mode.
                return ToolResult::error(risk_explanation(PsRiskLevel::Critical, &params.command));
            }

            PsRiskLevel::High => {
                // Require explicit user permission (same once/session/deny
                // pattern as BashTool: delegate to ctx.check_permission which
                // in interactive mode shows the TUI dialog).
                let desc = format!(
                    "[{} risk] {}",
                    risk_label(risk),
                    params.description.as_deref().unwrap_or(&params.command)
                );
                let details = risk_explanation(PsRiskLevel::High, &params.command);
                if let Err(e) = ctx.check_permission_with_details_and_path(
                    self.name(),
                    &desc,
                    &details,
                    std::path::PathBuf::from(&params.command),
                    false,
                ) {
                    return ToolResult::error(e.to_string());
                }
            }

            PsRiskLevel::Medium => {
                // Only gate if the caller set require_confirmation, or if the
                // context permission mode is Default (non-bypass, non-accept).
                let needs_gate = params.require_confirmation
                    || matches!(
                        ctx.permission_mode,
                        claurst_core::config::PermissionMode::Default
                            | claurst_core::config::PermissionMode::Plan
                    );

                if needs_gate {
                    let desc = format!(
                        "[{} risk] {}",
                        risk_label(risk),
                        params.description.as_deref().unwrap_or(&params.command)
                    );
                    let details = risk_explanation(PsRiskLevel::Medium, &params.command);
                    if let Err(e) = ctx.check_permission_with_details_and_path(
                        self.name(),
                        &desc,
                        &details,
                        std::path::PathBuf::from(&params.command),
                        false,
                    ) {
                        return ToolResult::error(e.to_string());
                    }
                }
            }

            PsRiskLevel::Low => {
                // Standard (non-risk-gated) permission check — honours bypass
                // and plan-mode rules, but does not show a dialog.
                let reason = params
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("This will execute a PowerShell command.")
                    .to_string();

                if let Err(e) = ctx.check_permission_for_path(
                    self.name(),
                    &reason,
                    std::path::PathBuf::from(&params.command),
                    false,
                ) {
                    return ToolResult::error(e.to_string());
                }
            }
        }

        // ── Step 3: execute ──────────────────────────────────────────────────
        // Fork (Waku): PowerShell 7 when installed, then Windows PowerShell;
        // execution policy bypassed for this one invocation (a machine's
        // default Restricted policy refuses even a one-line script); output
        // told to be UTF-8, since Windows PowerShell otherwise encodes a
        // redirected stream in the console code page; and both pipes read at
        // once through `crate::capture`, which also kills the whole tree on a
        // timeout.
        let exe = powershell_executable();
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
             $OutputEncoding = [System.Text.Encoding]::UTF8; {}",
            params.command
        );

        debug!(
            command = %params.command,
            risk    = ?risk,
            "Executing PowerShell command"
        );

        let timeout_ms = params.timeout.min(600_000);
        let timeout_dur = Duration::from_millis(timeout_ms);

        let mut process = Command::new(exe);
        process
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
            ])
            .arg(&script)
            .current_dir(&ctx.working_dir);
        let captured = match crate::capture::run_captured(process, timeout_dur).await {
            Ok(captured) => captured,
            Err(e) => return ToolResult::error(format!("Failed to spawn PowerShell: {}", e)),
        };
        if captured.timed_out {
            return ToolResult::error(format!("PowerShell command timed out after {}ms", timeout_ms));
        }

        let exit_code = captured.exit_code.unwrap_or(-1);
        let mut output = captured.stdout.trim_end().to_string();
        let stderr = captured.stderr.trim_end();
        if !stderr.is_empty() {
            if !output.is_empty() { output.push('\n'); }
            output.push_str("STDERR:\n");
            output.push_str(stderr);
        }
        if output.is_empty() { output = "(no output)".to_string(); }

        // Truncate very long output (same limit as BashTool)
        const MAX_OUTPUT_LEN: usize = 100_000;
        if output.len() > MAX_OUTPUT_LEN {
            // Fork departure (Waku): character boundaries, not byte
            // offsets. See `pty_bash::truncate_output`.
            let half = MAX_OUTPUT_LEN / 2;
            let start = &output[..crate::pty_bash::floor_char_boundary(&output, half)];
            let end = &output[crate::pty_bash::ceil_char_boundary(
                &output,
                output.len() - half,
            )..];
            output = format!(
                "{}\n\n... ({} characters truncated) ...\n\n{}",
                start,
                output.len() - MAX_OUTPUT_LEN,
                end
            );
        }

        if exit_code != 0 {
            ToolResult::error(format!("PowerShell exited with code {}\n{}", exit_code, output))
        } else {
            ToolResult::success(output)
        }
    }
}

/// Fork (Waku): PowerShell 7 (`pwsh`) when it is on PATH, else Windows
/// PowerShell. Resolved once.
fn powershell_executable() -> &'static std::path::Path {
    static EXE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        which::which("pwsh").unwrap_or_else(|_| {
            if cfg!(windows) {
                std::path::PathBuf::from("powershell")
            } else {
                std::path::PathBuf::from("pwsh")
            }
        })
    })
}
