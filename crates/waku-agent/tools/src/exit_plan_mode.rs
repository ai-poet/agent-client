// ExitPlanMode tool: leave planning mode and return to normal execution.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

pub struct ExitPlanModeTool;

#[derive(Debug, Deserialize)]
struct ExitPlanModeInput {
    #[serde(default)]
    summary: Option<String>,
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_EXIT_PLAN_MODE
    }

    fn description(&self) -> &str {
        "Exit plan mode and return to normal execution mode where all tools \
         are available. Optionally provide a summary of the plan."
    }

    fn permission_level(&self) -> PermissionLevel {
        // Honest: leaving plan mode changes nothing on disk. But `None` also
        // means the central backstop never asks anyone — see `self_gates`.
        PermissionLevel::None
    }

    /// Fork: this tool asks for itself.
    ///
    /// The central backstop in `execute_tool` only gates tools whose declared
    /// level is gated, and `None` is not — so with the default here the model
    /// could leave plan mode without anyone being consulted, and the
    /// bridge's "finished planning" dialog (keyed on this tool's name) was
    /// unreachable. Gating from inside also lets the plan summary ride along
    /// as the request's description, which is what the dialog shows.
    fn self_gates(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Summary of the plan you developed"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: ExitPlanModeInput = serde_json::from_value(input).unwrap_or(ExitPlanModeInput {
            summary: None,
        });

        // Fork: the user decides whether planning is done. Asked before the
        // success result is built, because the bridge flips its mode on this
        // tool's *metadata* — a refusal must return an error with none, so
        // the session stays in plan mode. The refusal text tells the model to
        // keep planning rather than to retry.
        let summary = params.summary.as_deref().unwrap_or("");
        if let Err(refused) = ctx.check_permission(self.name(), summary, false) {
            return ToolResult::error(refused.to_string());
        }

        debug!(summary = ?params.summary, "Exiting plan mode");

        let msg = if let Some(summary) = &params.summary {
            format!("Exited plan mode. Plan summary: {}", summary)
        } else {
            "Exited plan mode. All tools are now available.".to_string()
        };

        ToolResult::success(msg).with_metadata(json!({
            "type": "exit_plan_mode",
            "summary": params.summary,
        }))
    }
}
