//! Existing Waku tools exposed through Sailry's ToolProvider boundary.
use super::output_store::OutputStore;
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
    outputs: Arc<OutputStore>,
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
            outputs: OutputStore::shared(),
        }
    }

    fn read_tool_output(&self, input: &Value) -> agent_core_tools::ToolResult {
        let output = &input["output"];
        let Some(call) = output["call"].as_str() else {
            return agent_core_tools::ToolResult::error("output.call is required");
        };
        let Some(revision) = output["revision"].as_str() else {
            return agent_core_tools::ToolResult::error("output.revision is required");
        };
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(32768) as usize;
        self.outputs
            .read(&self.context.session_id, call, revision, offset, limit)
            .map(|page| agent_core_tools::ToolResult::success(page.to_string()))
            .unwrap_or_else(agent_core_tools::ToolResult::error)
    }

    fn read_command(&self, input: &Value) -> agent_core_tools::ToolResult {
        let Some(id) = input["id"].as_str() else {
            return agent_core_tools::ToolResult::error("id is required");
        };
        let Some(stream) = input["stream"].as_str() else {
            return agent_core_tools::ToolResult::error("stream is required");
        };
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(32768) as usize;
        self.outputs
            .read_command(&self.context.session_id, id, stream, offset, limit)
            .map(|page| agent_core_tools::ToolResult::success(page.to_string()))
            .unwrap_or_else(agent_core_tools::ToolResult::error)
    }
}

#[async_trait]
impl ToolProvider<RuntimeSession> for ExistingTools {
    async fn tools(&self, _: &RuntimeSession) -> anyhow::Result<Vec<EngineTool>> {
        let mut definitions = self
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
            .collect::<Vec<_>>();
        definitions.push(EngineTool::new(
            "read_tool_output",
            "Read the original saved tool result as UTF-8 pages. Pass the output reference unchanged and continue with next_offset.",
            serde_json::json!({
                "type":"object","additionalProperties":false,"required":["output"],"properties":{
                    "output":{"type":"object","additionalProperties":false,"required":["call","revision"],"properties":{
                        "call":{"type":"string"},"revision":{"type":"string","pattern":"^[a-fA-F0-9]{64}$"}
                    }},
                    "offset":{"type":"integer","minimum":0},
                    "limit":{"type":"integer","minimum":1,"maximum":32768}
                }
            }).as_object().cloned().unwrap(),
        ));
        definitions.push(EngineTool::new(
            "read_command",
            "Read retained stdout or stderr from a command in this session. This never starts a command.",
            serde_json::json!({
                "type":"object","additionalProperties":false,"required":["id","stream"],"properties":{
                    "id":{"type":"string"},"stream":{"type":"string","enum":["stdout","stderr"]},
                    "offset":{"type":"integer","minimum":0},
                    "limit":{"type":"integer","minimum":1,"maximum":32768}
                }
            }).as_object().cloned().unwrap(),
        ));
        Ok(definitions)
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
        let result = match name.as_str() {
            "read_tool_output" => self.read_tool_output(&input),
            "read_command" => self.read_command(&input),
            _ => crate::runner::tools::execute_tool(&name, &input, &self.tools, &context).await,
        };
        if let Some(events) = &self.events {
            let _ = events.send(QueryEvent::ToolEnd {
                tool_name: name,
                tool_id: id.to_string(),
                result: result.content.clone(),
                is_error: result.is_error,
                metadata: result.metadata.clone(),
            });
        }
        let content = vec![ContentBlock::text(
            result
                .structured_content
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_else(|| result.content.clone()),
        )];
        let mut output = if result.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        };
        output.structured_content = result.structured_content;
        self.outputs.record(&self.context.session_id, id, &output);
        Ok(output)
    }

    fn concurrent(&self, _: &RuntimeSession, call: &CallToolRequestParams) -> bool {
        if matches!(call.name.as_ref(), "read_tool_output" | "read_command") {
            return true;
        }
        crate::runner::tools::runs_concurrently(call.name.as_ref(), &self.tools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::config::PermissionMode;
    use agent_core::permissions::{PermissionDecision, PermissionHandler};
    use sailry_protocol::SessionId;
    use std::path::PathBuf;

    struct Structured;
    #[async_trait]
    impl Tool for Structured {
        fn name(&self) -> &str {
            "fixture"
        }
        fn description(&self) -> &str {
            "fixture"
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type":"object"})
        }
        fn permission_level(&self) -> agent_core_tools::PermissionLevel {
            agent_core_tools::PermissionLevel::ReadOnly
        }
        async fn execute(&self, _: Value, _: &ToolContext) -> agent_core_tools::ToolResult {
            agent_core_tools::ToolResult::success("text")
                .with_structured_content(serde_json::json!({"kind":"file_lines"}))
        }
    }

    struct Command;
    #[async_trait]
    impl Tool for Command {
        fn name(&self) -> &str {
            "command"
        }
        fn description(&self) -> &str {
            "command"
        }
        fn permission_level(&self) -> agent_core_tools::PermissionLevel {
            agent_core_tools::PermissionLevel::ReadOnly
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type":"object"})
        }
        async fn execute(&self, _: Value, _: &ToolContext) -> agent_core_tools::ToolResult {
            agent_core_tools::ToolResult::success("hello").with_structured_content(
                serde_json::json!({
                    "kind":"command_result",
                    "data":{"stdout":{"text":"hello"},"stderr":{"text":""}}
                }),
            )
        }
    }

    struct Allow;
    impl PermissionHandler for Allow {
        fn check_permission(
            &self,
            _: &agent_core::permissions::PermissionRequest,
        ) -> PermissionDecision {
            PermissionDecision::Allow
        }
        fn request_permission(
            &self,
            _: &agent_core::permissions::PermissionRequest,
        ) -> PermissionDecision {
            PermissionDecision::Allow
        }
    }

    fn context() -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("."),
            permission_mode: PermissionMode::Default,
            permission_handler: Arc::new(Allow),
            cost_tracker: agent_core::cost::CostTracker::new(),
            session_id: "fixture".into(),
            file_history: Arc::new(parking_lot::Mutex::new(
                agent_core::file_history::FileHistory::new(),
            )),
            current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config: Default::default(),
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
            cancel_token: Default::default(),
        }
    }

    #[tokio::test]
    async fn preserves_structured_tool_content() {
        let context = context();
        let provider = ExistingTools::new(Arc::new(vec![Box::new(Structured)]), context, None);
        let session = RuntimeSession::new(SessionId::new(), Vec::new());
        let result = provider
            .call(
                &session,
                "call-1",
                CallToolRequestParams::new("fixture"),
                &Emitter::new(mpsc::unbounded_channel().0, Default::default()),
            )
            .await
            .unwrap();
        assert_eq!(
            result.structured_content,
            Some(serde_json::json!({"kind":"file_lines"}))
        );

        let revision = agent_core_engine::agent::output::result_revision(&result);
        let read = provider
            .call(
                &session,
                "read-1",
                CallToolRequestParams::new("read_tool_output").with_arguments(
                    serde_json::json!({
                        "output":{"call":"call-1","revision":revision},
                        "offset":0,
                        "limit":1024,
                    })
                    .as_object()
                    .cloned()
                    .unwrap(),
                ),
                &Emitter::new(mpsc::unbounded_channel().0, Default::default()),
            )
            .await
            .unwrap();
        assert!(read.content[0]
            .as_text()
            .unwrap()
            .text
            .contains("file_lines"));
        let names = ToolProvider::tools(&provider, &session)
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"read_tool_output".into()));
        assert!(names.contains(&"read_command".into()));
    }

    #[tokio::test]
    async fn reads_command_output_through_the_public_tool() {
        let provider = ExistingTools::new(Arc::new(vec![Box::new(Command)]), context(), None);
        let session = RuntimeSession::new(SessionId::new(), Vec::new());
        let emitter = Emitter::new(mpsc::unbounded_channel().0, Default::default());
        provider
            .call(
                &session,
                "command-1",
                CallToolRequestParams::new("command"),
                &emitter,
            )
            .await
            .unwrap();
        let result = provider
            .call(
                &session,
                "read-1",
                CallToolRequestParams::new("read_command").with_arguments(
                    serde_json::json!({"id":"command-1","stream":"stdout","offset":1,"limit":3})
                        .as_object()
                        .cloned()
                        .unwrap(),
                ),
                &emitter,
            )
            .await
            .unwrap();
        let page: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(page["text"], "ell");
        assert_eq!(page["next_offset"], 4);
    }
}
