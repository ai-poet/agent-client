// ExitPlanMode tool: leave planning mode and return to normal execution.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

pub struct ExitPlanModeTool;

/// Fork: the longest plan the tool takes, in characters — the bound ZCode
/// sets on the same parameter.
pub const MAX_PLAN_CHARS: usize = 20_000;

/// Fork: the refusal for a call that carries no plan. Exported so the
/// driver can tell it apart from the user sending the plan back.
pub const MISSING_PLAN_ERROR: &str = "ExitPlanMode needs the complete plan in `plan`: the Markdown the user reads and approves, not a summary of it and not a pointer to your reply. Call it again with the whole plan.";

#[derive(Debug, Default, Deserialize)]
struct ExitPlanModeInput {
    #[serde(default)]
    plan: Option<String>,
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_EXIT_PLAN_MODE
    }

    /// Fork: the plan is the parameter, as in Claude Code and ZCode. The
    /// tool used to take an optional `summary` and nothing else, so the plan
    /// the user was asked to approve had nowhere to go: models wrote "let me
    /// present the plan" and handed over a one-line summary, and the dialog
    /// showed that.
    fn description(&self) -> &str {
        "Hand your plan to the user for approval and, once they approve it, \
         leave plan mode so you can carry it out. Put the complete plan in \
         `plan`, in Markdown: it is exactly what the user reads and approves, \
         so it must be the whole plan, not a summary of it and not a pointer \
         to text in your reply. Call this only once the plan is written and \
         every question is settled, and do not ask in text whether the plan \
         is okay: this tool is that question. If the user sends the plan back, \
         revise it and call this again with the new plan."
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
    /// unreachable. Gating from inside also lets the plan ride along as the
    /// request's description, which is what the dialog shows.
    fn self_gates(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The complete plan, in Markdown, for the user to read and approve."
                }
            },
            "required": ["plan"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: ExitPlanModeInput = serde_json::from_value(input).unwrap_or_default();
        let plan = params.plan.as_deref().map(str::trim).unwrap_or_default();

        // Fork: a call without the plan never reaches the user — there is
        // nothing for them to approve. Sent back to the model to call again.
        if plan.is_empty() {
            return ToolResult::error(MISSING_PLAN_ERROR.to_owned());
        }
        let length = plan.chars().count();
        if length > MAX_PLAN_CHARS {
            return ToolResult::error(format!(
                "The plan is {length} characters; ExitPlanMode takes at most {MAX_PLAN_CHARS}. \
                 Tighten it and call again."
            ));
        }

        // Fork: the user decides whether planning is done. Asked before the
        // success result is built, because the bridge flips its mode on this
        // tool's *metadata* — a refusal must return an error with none, so
        // the session stays in plan mode. The refusal text tells the model to
        // keep planning rather than to retry.
        if let Err(refused) = ctx.check_permission(self.name(), plan, false) {
            return ToolResult::error(refused.to_string());
        }

        debug!(plan_chars = length, "Exiting plan mode");

        ToolResult::success(
            "The user approved the plan and plan mode is over. Carry the plan out now; \
             update your todo list first if you keep one."
                .to_owned(),
        )
        .with_metadata(json!({
            "type": "exit_plan_mode",
            "plan": plan,
        }))
    }
}
