use crate::{
    config::McpConfig,
    mcp::McpMessage,
    runtime::types::{RuntimeToolCall, RuntimeToolResult},
    tools_runtime::{ToolDefinition, ToolExecutionContext, ToolExecutor},
};
use async_trait::async_trait;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdout, Command},
};

const MCP_PREFIX: &str = "mcp_ext__";

pub struct ExternalMcpToolExecutor {
    servers: Vec<McpConfig>,
}

impl ExternalMcpToolExecutor {
    pub fn new(servers: Vec<McpConfig>) -> Self {
        Self { servers }
    }

    fn qualify_tool_name(server: &str, tool: &str) -> String {
        format!("{MCP_PREFIX}{server}__{tool}")
    }

    fn parse_qualified_tool_name(name: &str) -> Option<(&str, &str)> {
        let stripped = name.strip_prefix(MCP_PREFIX)?;
        let mut parts = stripped.splitn(2, "__");
        Some((parts.next()?, parts.next()?))
    }

    async fn spawn_server(config: &McpConfig) -> anyhow::Result<Child> {
        let mut command = Command::new(&config.command);
        command.args(&config.args);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &config.env {
            command.env(key, value);
        }
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::null());
        Ok(command.spawn()?)
    }

    async fn send_message(child: &mut Child, message: &McpMessage) -> anyhow::Result<()> {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdin unavailable"))?;
        let json = serde_json::to_string(message)?;
        let payload = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
        stdin.write_all(payload.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_message(reader: &mut BufReader<ChildStdout>) -> anyhow::Result<McpMessage> {
        let mut line = String::new();
        let mut content_length = None;

        loop {
            line.clear();
            let bytes = reader.read_line(&mut line).await?;
            if bytes == 0 {
                return Err(anyhow::anyhow!("Unexpected EOF while reading MCP headers"));
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(value) = trimmed.strip_prefix("Content-Length: ") {
                content_length = Some(value.parse::<usize>()?);
            }
        }

        let content_length =
            content_length.ok_or_else(|| anyhow::anyhow!("Missing MCP Content-Length header"))?;
        let mut buffer = vec![0u8; content_length];
        reader.read_exact(&mut buffer).await?;
        Ok(serde_json::from_slice(&buffer)?)
    }

    async fn initialize_and_request(
        config: &McpConfig,
        request: McpMessage,
    ) -> anyhow::Result<McpMessage> {
        let mut child = Self::spawn_server(config).await?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdout unavailable"))?;
        let mut reader = BufReader::new(stdout);

        Self::send_message(&mut child, &McpMessage::request(1, "initialize", None)).await?;
        let _ = Self::read_message(&mut reader).await?;
        Self::send_message(&mut child, &request).await?;
        let response = Self::read_message(&mut reader).await?;
        let _ = child.kill().await;
        Ok(response)
    }

    async fn list_server_tools(config: &McpConfig) -> anyhow::Result<Vec<ToolDefinition>> {
        let response =
            Self::initialize_and_request(config, McpMessage::request(2, "tools/list", None))
                .await?;
        if let Some(error) = response.error {
            return Err(anyhow::anyhow!(error.message));
        }

        let tools = response
            .result
            .and_then(|result| result.get("tools").cloned())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();

        Ok(tools
            .into_iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?.to_string();
                let description = tool
                    .get("description")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let input_schema = tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type":"object"}));

                Some(ToolDefinition {
                    name: Self::qualify_tool_name(&config.name, &name),
                    description,
                    input_schema,
                })
            })
            .collect())
    }

    async fn call_server_tool(
        config: &McpConfig,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<String> {
        let request = McpMessage::request(
            2,
            "tools/call",
            Some(serde_json::json!({
                "name": tool_name,
                "arguments": arguments,
            })),
        );
        let response = Self::initialize_and_request(config, request).await?;
        if let Some(error) = response.error {
            return Err(anyhow::anyhow!(error.message));
        }

        let content_blocks = response
            .result
            .and_then(|result| result.get("content").cloned())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();

        let text = content_blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(|value| value.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(text)
    }
}

#[async_trait]
impl ToolExecutor for ExternalMcpToolExecutor {
    async fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();
        for config in &self.servers {
            if config.command.trim().is_empty() {
                continue;
            }
            if let Ok(mut server_tools) = Self::list_server_tools(config).await {
                definitions.append(&mut server_tools);
            }
        }
        definitions
    }

    async fn execute(
        &self,
        call: &RuntimeToolCall,
        _context: &ToolExecutionContext,
    ) -> RuntimeToolResult {
        let Some((server_name, tool_name)) = Self::parse_qualified_tool_name(&call.name) else {
            return RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!("Not an external MCP tool: {}", call.name),
                is_error: true,
            };
        };

        let Some(config) = self
            .servers
            .iter()
            .find(|config| config.name == server_name)
        else {
            return RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!("Configured MCP server not found: {}", server_name),
                is_error: true,
            };
        };

        match Self::call_server_tool(config, tool_name, call.arguments.clone()).await {
            Ok(content) => RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content,
                is_error: false,
            },
            Err(error) => RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: error.to_string(),
                is_error: true,
            },
        }
    }
}
