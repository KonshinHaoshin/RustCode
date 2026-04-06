use crate::{
    mcp,
    runtime::types::{RuntimeToolCall, RuntimeToolResult},
    tools_runtime::{ToolDefinition, ToolExecutionContext, ToolExecutor},
};
use async_trait::async_trait;
use std::sync::{Arc, OnceLock};

const MCP_PREFIX: &str = "mcp__";

pub struct McpToolExecutor {
    registry: OnceLock<Arc<mcp::ToolRegistry>>,
}

impl McpToolExecutor {
    pub fn new() -> Self {
        Self {
            registry: OnceLock::new(),
        }
    }

    fn static_definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "mcp__file_read".to_string(),
                description: "Read file contents through the MCP registry".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to read"}
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                name: "mcp__file_write".to_string(),
                description: "Write file contents through the MCP registry".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to write"},
                        "content": {"type": "string", "description": "Content to write"}
                    },
                    "required": ["path", "content"]
                }),
            },
            ToolDefinition {
                name: "mcp__execute_command".to_string(),
                description: "Execute a command through the MCP registry".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Command to execute"},
                        "cwd": {"type": "string", "description": "Working directory"}
                    },
                    "required": ["command"]
                }),
            },
            ToolDefinition {
                name: "mcp__search".to_string(),
                description: "Search files through the MCP registry".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Search pattern"},
                        "path": {"type": "string", "description": "Directory to search"}
                    },
                    "required": ["pattern"]
                }),
            },
        ]
    }

    async fn registry(&self) -> Arc<mcp::ToolRegistry> {
        if let Some(registry) = self.registry.get() {
            return Arc::clone(registry);
        }

        let registry = Arc::new(mcp::ToolRegistry::new());
        registry.register_builtin_tools().await;
        let _ = self.registry.set(Arc::clone(&registry));
        registry
    }

    fn to_mcp_name(name: &str) -> Option<&str> {
        name.strip_prefix(MCP_PREFIX)
    }

    fn format_result(
        &self,
        call: &RuntimeToolCall,
        value: serde_json::Value,
        is_error: bool,
    ) -> RuntimeToolResult {
        let content = if let Some(text) = value.get("content").and_then(|value| value.as_str()) {
            text.to_string()
        } else if let Some(stdout) = value.get("stdout").and_then(|value| value.as_str()) {
            let stderr = value
                .get("stderr")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if stderr.is_empty() {
                stdout.to_string()
            } else if stdout.is_empty() {
                stderr.to_string()
            } else {
                format!("{stdout}\n{stderr}")
            }
        } else if let Some(files) = value.get("files").and_then(|value| value.as_array()) {
            files
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
        };

        RuntimeToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            is_error,
        }
    }
}

impl Default for McpToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for McpToolExecutor {
    async fn definitions(&self) -> Vec<ToolDefinition> {
        Self::static_definitions()
    }

    async fn execute(
        &self,
        call: &RuntimeToolCall,
        _context: &ToolExecutionContext,
    ) -> RuntimeToolResult {
        let Some(name) = Self::to_mcp_name(&call.name) else {
            return RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!("Not an MCP tool: {}", call.name),
                is_error: true,
            };
        };

        let registry = self.registry().await;
        match registry.execute(name, call.arguments.clone()).await {
            Ok(value) => self.format_result(call, value, false),
            Err(error) => RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: error.to_string(),
                is_error: true,
            },
        }
    }
}
