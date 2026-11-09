//! Existing Waku tools exposed through Sailry's ToolProvider boundary.
use agent_core_engine::agent::catalog::ToolProvider;
use agent_core_engine::agent::engine::Emitter;
use agent_core_engine::store::agent::RuntimeSession;
use agent_core_tools::{Tool, ToolContext};
use async_trait::async_trait;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Tool as EngineTool,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::QueryEvent;

pub struct ExistingTools {
    tools: Arc<Vec<Box<dyn Tool>>>,
    context: ToolContext,
    events: Option<mpsc::UnboundedSender<QueryEvent>>,
}

impl ExistingTools {
    pub fn new(
        tools: Arc<Vec<Box<dyn Tool>>>,
        context: ToolContext,
        events: Option<mpsc::UnboundedSender<QueryEvent>>,
    ) -> Self {
        Self {
            tools,
            context,
            events,
        }
    }
}

#[async_trait]
impl ToolProvider<RuntimeSession> for ExistingTools {
    async fn tools(&self, _: &RuntimeSession) -> anyhow::Result<Vec<EngineTool>> {
        Ok(self
            .tools
            .iter()
            .map(|tool| {
                let definition = tool.to_definition();
                EngineTool::new(
                    definition.name,
                    definition.description,
                    definition
                        .input_schema
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                )
            })
            .collect())
    }

    async fn call(
        &self,
        _: &RuntimeSession,
        id: &str,
        call: CallToolRequestParams,
        emit: &Emitter,
    ) -> Result<CallToolResult, ErrorData> {
        let name = call.name.to_string();
        let input = Value::Object(call.arguments.unwrap_or_default());
        if let Some(events) = &self.events {
            let _ = events.send(QueryEvent::ToolStart {
                tool_name: name.clone(),
                tool_id: id.to_string(),
                input_json: serde_json::to_string(&input).unwrap_or_default(),
            });
        }
        let mut context = self.context.clone();
        context.cancel_token = emit.cancel_token().clone();
        let result = crate::runner::tools::execute_tool(&name, &input, &self.tools, &context).await;
        if let Some(events) = &self.events {
            let _ = events.send(QueryEvent::ToolEnd {
                tool_name: name,
                tool_id: id.to_string(),
                result: result.content.clone(),
                is_error: result.is_error,
                metadata: result.metadata.clone(),
            });
        }
        let content = vec![ContentBlock::text(result.content)];
        Ok(if result.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        })
    }

    fn concurrent(&self, _: &RuntimeSession, call: &CallToolRequestParams) -> bool {
        crate::runner::tools::runs_concurrently(call.name.as_ref(), &self.tools)
    }
}
