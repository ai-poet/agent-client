//! The `agent_teams_*` tools as the engine sees them.
//!
//! Each one hands its arguments to the team runtime on a blocking thread
//! (the runtime does file I/O under locks and may wait for a member to
//! stop) and returns the runtime's text. The caller's identity is the
//! session id the engine runs the call under: the captain's own, or the
//! member's.

use std::sync::Arc;

use agent_teams::prompts::is_member_tool;
use agent_teams::runtime::schema::{ToolSpec, tool_specs};
use async_trait::async_trait;
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use serde_json::Value;

use super::TeamHost;

struct TeamTool {
    host: std::sync::Weak<TeamHost>,
    spec: ToolSpec,
}

#[async_trait]
impl Tool for TeamTool {
    fn name(&self) -> &str {
        self.spec.name
    }

    fn description(&self) -> &str {
        self.spec.description
    }

    /// Team bookkeeping touches only the team's own state directory, through
    /// the runtime's rules; there is nothing here for the user to approve.
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        self.spec.input_schema.clone()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let Some(runtime) = self.host.upgrade().and_then(|host| host.runtime()) else {
            return ToolResult::error("AgentTeams is not available in this session.");
        };
        let name = self.spec.name;
        let caller = ctx.session_id.clone();
        let outcome =
            tokio::task::spawn_blocking(move || runtime.call_tool(name, &caller, &input)).await;
        if let Some(host) = self.host.upgrade() {
            host.refresh_snapshot();
        }
        match outcome {
            Ok(Ok(text)) => ToolResult::success(text),
            Ok(Err(error)) => ToolResult::error(error),
            Err(error) => {
                ToolResult::error(format!("AgentTeams: the call did not finish ({error})"))
            }
        }
    }
}

fn tools_where(host: &Arc<TeamHost>, keep: impl Fn(&str) -> bool) -> Vec<Box<dyn Tool>> {
    tool_specs()
        .into_iter()
        .filter(|spec| keep(spec.name))
        .map(|spec| {
            Box::new(TeamTool {
                host: Arc::downgrade(host),
                spec,
            }) as Box<dyn Tool>
        })
        .collect()
}

/// All fourteen: the captain also claims, updates and reports on its own
/// takeovers.
pub(super) fn captain_tools(host: &Arc<TeamHost>) -> Vec<Box<dyn Tool>> {
    tools_where(host, |_| true)
}

/// The four a member keeps.
pub(super) fn member_tools(host: &Arc<TeamHost>) -> Vec<Box<dyn Tool>> {
    tools_where(host, is_member_tool)
}
