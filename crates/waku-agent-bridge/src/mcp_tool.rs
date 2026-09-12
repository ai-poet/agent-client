//! MCP server tools, presented to the model as ordinary tools.
//!
//! The engine's `McpManager` connects to the configured servers and can call
//! their tools, but it does not itself implement the `Tool` trait the query
//! loop dispatches on — upstream keeps that adapter in its CLI crate, which is
//! not vendored. This is the same adapter, with one correction: it declares
//! `self_gates`, so the approval it raises is the only one. Without that the
//! central backstop in `execute_tool` would gate an `Execute`-level tool a
//! second time and the user would be asked twice for one call.

use std::sync::Arc;

use async_trait::async_trait;
use claurst_core::ToolDefinition;
// `PermissionLevel` exists in both `claurst_core` and `claurst_tools` as
// distinct types; the `Tool` trait is written against the tools crate's.
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use serde_json::Value;

pub struct McpTool {
    definition: ToolDefinition,
    server: String,
    manager: Arc<claurst_mcp::McpManager>,
}

impl McpTool {
    /// One wrapper per tool the connected servers advertise. Names arrive
    /// already prefixed with the server (`<server>_<tool>`), which is what
    /// keeps two servers' `search` tools apart and what `McpManager::call_tool`
    /// routes on.
    pub fn all(manager: &Arc<claurst_mcp::McpManager>) -> Vec<Box<dyn Tool>> {
        manager
            .all_tool_definitions()
            .into_iter()
            .map(|(server, definition)| {
                Box::new(Self {
                    definition,
                    server,
                    manager: manager.clone(),
                }) as Box<dyn Tool>
            })
            .collect()
    }

    fn bare_name(&self) -> &str {
        let prefix = format!("{}_", self.server);
        self.definition
            .name
            .strip_prefix(&prefix)
            .unwrap_or(&self.definition.name)
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    /// An MCP tool is an external process doing whatever it does; nothing
    /// about its schema says whether it reads or writes. Execute is the level
    /// that asks.
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    /// The approval below names the server and tool, which the generic
    /// backstop cannot. Declaring this keeps it the only prompt.
    fn self_gates(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        self.definition.input_schema.clone()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let description = format!("Run `{}` on MCP server `{}`", self.bare_name(), self.server);
        if let Err(error) = ctx.check_permission(self.name(), &description, false) {
            return ToolResult::error(error.to_string());
        }

        let arguments = (!input.is_null()).then_some(input);
        match self.manager.call_tool(&self.definition.name, arguments).await {
            Ok(result) => {
                let text = claurst_mcp::mcp_result_to_string(&result);
                if result.is_error {
                    ToolResult::error(text)
                } else {
                    ToolResult::success(text)
                }
            }
            Err(error) => ToolResult::error(format!(
                "MCP tool `{}` on `{}` failed: {error}",
                self.bare_name(),
                self.server
            )),
        }
    }
}
