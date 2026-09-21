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
                // The text is the engine's own rendering, which reduces an
                // image to a 32-character preview so the context is never
                // flooded. The pixels take the metadata sideband instead,
                // which reaches the transcript and not the model.
                let text = claurst_mcp::mcp_result_to_string(&result);
                if result.is_error {
                    ToolResult::error(text)
                } else {
                    match image_metadata(&result) {
                        Some(images) => ToolResult::success(text).with_metadata(images),
                        None => ToolResult::success(text),
                    }
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

/// The images an MCP result carried, in the shape the transcript reads.
///
/// `waku-core`'s `activity::collect_image_urls` accepts `{"type": "image",
/// "mime": .., "data": ..}` items under a `content` array and turns each
/// into a data URL — the same path every other provider's images take, so
/// nothing downstream needs to know these came over MCP. `None` when the
/// result holds no image, so text-only tools keep an empty sideband.
pub(crate) fn image_metadata(result: &claurst_mcp::CallToolResult) -> Option<Value> {
    let images: Vec<Value> = result
        .content
        .iter()
        .filter_map(|item| match item {
            claurst_mcp::McpContent::Image { data, mime_type } => Some(serde_json::json!({
                "type": "image",
                "mime": mime_type,
                "data": data,
            })),
            _ => None,
        })
        .collect();
    (!images.is_empty()).then(|| serde_json::json!({ "content": images }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_mcp::{CallToolResult, McpContent};

    #[test]
    fn an_image_in_the_result_becomes_transcript_metadata() {
        let result = CallToolResult {
            content: vec![
                McpContent::Text { text: "done".into() },
                McpContent::Image { data: "aGVsbG8=".into(), mime_type: "image/png".into() },
            ],
            is_error: false,
        };
        let meta = image_metadata(&result).expect("one image");
        let items = meta["content"].as_array().expect("array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "image");
        assert_eq!(items[0]["mime"], "image/png");
        assert_eq!(items[0]["data"], "aGVsbG8=");
    }

    /// A text-only tool keeps an empty sideband, so the plan-mode metadata
    /// and this one never have to share a document.
    #[test]
    fn a_text_only_result_carries_no_metadata() {
        let result = CallToolResult {
            content: vec![McpContent::Text { text: "just words".into() }],
            is_error: false,
        };
        assert!(image_metadata(&result).is_none());
    }
}
