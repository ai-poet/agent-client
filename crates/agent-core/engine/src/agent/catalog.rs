//! Minimal tool-provider boundary used verbatim by Sailry's scheduler.
use crate::agent::engine::Emitter;
use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, CallToolResult, ErrorData, Tool};

#[async_trait]
pub trait ToolProvider<S>: Send + Sync {
    async fn tools(&self, session: &S) -> anyhow::Result<Vec<Tool>>;
    async fn call(
        &self,
        session: &S,
        id: &str,
        call: CallToolRequestParams,
        emit: &Emitter,
    ) -> Result<CallToolResult, ErrorData>;
    fn concurrent(&self, _: &S, _: &CallToolRequestParams) -> bool {
        false
    }
}

pub fn fault(error: sailry_protocol::Fault) -> ErrorData {
    ErrorData {
        code: rmcp::model::ErrorCode(-32000),
        message: error.message.into(),
        data: None,
    }
}
