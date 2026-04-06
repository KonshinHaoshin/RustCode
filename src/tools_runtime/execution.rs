use crate::{
    runtime::types::{RuntimeToolCall, RuntimeToolResult},
    tools::{ToolError, ToolOutput, ToolRegistry},
    tools_runtime::{registry::builtin_tool_definitions, ToolDefinition},
};
use async_trait::async_trait;

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn definitions(&self) -> Vec<ToolDefinition>;
    async fn execute(&self, call: &RuntimeToolCall) -> RuntimeToolResult;
}

pub struct BuiltinToolExecutor {
    registry: ToolRegistry,
}

impl BuiltinToolExecutor {
    pub fn new() -> Self {
        Self {
            registry: ToolRegistry::new(),
        }
    }

    fn render_success(&self, output: ToolOutput) -> String {
        if output.content.trim().is_empty() {
            format!("Tool completed with output type {}", output.output_type)
        } else {
            output.content
        }
    }

    fn render_error(&self, error: ToolError) -> String {
        match error.code {
            Some(code) => format!("{} ({})", error.message, code),
            None => error.message,
        }
    }
}

impl Default for BuiltinToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for BuiltinToolExecutor {
    async fn definitions(&self) -> Vec<ToolDefinition> {
        builtin_tool_definitions(&self.registry)
    }

    async fn execute(&self, call: &RuntimeToolCall) -> RuntimeToolResult {
        match self
            .registry
            .execute(&call.name, call.arguments.clone())
            .await
        {
            Ok(output) => RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: self.render_success(output),
                is_error: false,
            },
            Err(error) => RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: self.render_error(error),
                is_error: true,
            },
        }
    }
}
