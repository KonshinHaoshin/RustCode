use crate::{
    agents_runtime::{spawn_agent_task, AgentTaskStatus, AgentTaskStore},
    config::{project_root_from, Settings},
    runtime::types::{RuntimeToolCall, RuntimeToolResult},
    services::agents::AgentsService,
    tools::{ToolError, ToolOutput, ToolRegistry},
    tools_runtime::{registry::builtin_tool_definitions, ToolDefinition},
};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct ToolExecutionContext {
    pub session_id: Option<String>,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn definitions(&self) -> Vec<ToolDefinition>;
    async fn execute(
        &self,
        call: &RuntimeToolCall,
        context: &ToolExecutionContext,
    ) -> RuntimeToolResult;
}

pub struct BuiltinToolExecutor {
    registry: ToolRegistry,
    settings: Settings,
    project_root: Option<PathBuf>,
    allowed_tools: Option<Vec<String>>,
    allow_task_tools: bool,
}

#[derive(Debug, Deserialize)]
struct TaskCreateInput {
    subject: String,
    description: String,
    agent_type: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct TaskIdInput {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct TaskUpdateInput {
    task_id: String,
    description: Option<String>,
    metadata: Option<serde_json::Value>,
}

impl BuiltinToolExecutor {
    pub fn new(settings: Settings, project_root: Option<PathBuf>) -> Self {
        Self {
            registry: ToolRegistry::new(),
            settings,
            project_root,
            allowed_tools: None,
            allow_task_tools: true,
        }
    }

    pub fn with_profile(
        settings: Settings,
        project_root: Option<PathBuf>,
        allowed_tools: Option<Vec<String>>,
        allow_task_tools: bool,
    ) -> Self {
        Self {
            registry: ToolRegistry::new(),
            settings,
            project_root,
            allowed_tools,
            allow_task_tools,
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

    fn is_allowed(&self, name: &str) -> bool {
        match &self.allowed_tools {
            Some(allowed) => allowed.iter().any(|tool| tool.eq_ignore_ascii_case(name)),
            None => true,
        }
    }

    fn task_definitions(&self) -> Vec<ToolDefinition> {
        if !self.allow_task_tools {
            return Vec::new();
        }

        vec![
            ToolDefinition {
                name: "task_create".to_string(),
                description: "Create and start a child agent task".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string" },
                        "description": { "type": "string" },
                        "agent_type": { "type": "string" },
                        "metadata": { "type": "object" }
                    },
                    "required": ["subject", "description", "agent_type"]
                }),
            },
            ToolDefinition {
                name: "task_list".to_string(),
                description: "List child agent tasks for the current session".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "task_get".to_string(),
                description: "Get details for a child agent task".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task_id": { "type": "string" }
                    },
                    "required": ["task_id"]
                }),
            },
            ToolDefinition {
                name: "task_update".to_string(),
                description: "Update task metadata or description".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task_id": { "type": "string" },
                        "description": { "type": "string" },
                        "metadata": { "type": "object" }
                    },
                    "required": ["task_id"]
                }),
            },
        ]
    }

    fn all_definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = builtin_tool_definitions(&self.registry)
            .into_iter()
            .filter(|definition| self.is_allowed(&definition.name))
            .collect::<Vec<_>>();
        definitions.extend(self.task_definitions());
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    async fn execute_task_tool(
        &self,
        call: &RuntimeToolCall,
        context: &ToolExecutionContext,
    ) -> anyhow::Result<RuntimeToolResult> {
        let store = AgentTaskStore::for_project(self.project_root.as_deref())?;

        let content = match call.name.as_str() {
            "task_create" => {
                let input: TaskCreateInput = serde_json::from_value(call.arguments.clone())?;
                if AgentsService::builtin_definition_by_name(&input.agent_type).is_none() {
                    return Err(anyhow::anyhow!("Unknown agent type: {}", input.agent_type));
                }

                let task = store.create(
                    input.subject,
                    input.description,
                    input.agent_type.clone(),
                    context.session_id.clone(),
                    input.metadata,
                )?;
                spawn_agent_task(
                    self.settings.clone(),
                    self.project_root.clone(),
                    task.id.clone(),
                )?;
                format!(
                    "Task created: #{} [{}] {}",
                    task.id, task.agent_type, task.subject
                )
            }
            "task_list" => {
                let tasks = store.list_for_parent(context.session_id.as_deref())?;
                if tasks.is_empty() {
                    "No tasks found".to_string()
                } else {
                    tasks
                        .into_iter()
                        .map(|task| {
                            format!(
                                "#{} [{}] {} ({})",
                                task.id,
                                task.agent_type,
                                task.subject,
                                format_task_status(task.status)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            "task_get" => {
                let input: TaskIdInput = serde_json::from_value(call.arguments.clone())?;
                let Some(task) = store.get(&input.task_id)? else {
                    return Err(anyhow::anyhow!("Task not found: {}", input.task_id));
                };
                let mut lines = vec![
                    format!("Task #{}: {}", task.id, task.subject),
                    format!("Agent: {}", task.agent_type),
                    format!("Status: {}", format_task_status(task.status)),
                    format!("Description: {}", task.description),
                ];
                if let Some(child_session_id) = task.child_session_id {
                    lines.push(format!("Child session: {}", child_session_id));
                }
                if let Some(summary) = task.result_summary {
                    lines.push(format!("Result: {}", summary));
                }
                if let Some(error) = task.error {
                    lines.push(format!("Error: {}", error));
                }
                lines.join("\n")
            }
            "task_update" => {
                let input: TaskUpdateInput = serde_json::from_value(call.arguments.clone())?;
                store.update_metadata(&input.task_id, input.description, input.metadata)?;
                format!("Task updated: #{}", input.task_id)
            }
            _ => return Err(anyhow::anyhow!("Unknown task tool: {}", call.name)),
        };

        Ok(RuntimeToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            is_error: false,
        })
    }
}

fn format_task_status(status: AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Pending => "pending",
        AgentTaskStatus::Running => "running",
        AgentTaskStatus::AwaitingApproval => "awaiting_approval",
        AgentTaskStatus::Completed => "completed",
        AgentTaskStatus::Failed => "failed",
        AgentTaskStatus::Cancelled => "cancelled",
    }
}

impl Default for BuiltinToolExecutor {
    fn default() -> Self {
        let settings = Settings::default();
        let project_root = project_root_from(Some(&settings.working_dir));
        Self::new(settings, project_root)
    }
}

#[async_trait]
impl ToolExecutor for BuiltinToolExecutor {
    async fn definitions(&self) -> Vec<ToolDefinition> {
        self.all_definitions()
    }

    async fn execute(
        &self,
        call: &RuntimeToolCall,
        context: &ToolExecutionContext,
    ) -> RuntimeToolResult {
        if call.name.starts_with("task_") {
            return match self.execute_task_tool(call, context).await {
                Ok(result) => result,
                Err(error) => RuntimeToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    content: error.to_string(),
                    is_error: true,
                },
            };
        }

        if !self.is_allowed(&call.name) {
            return RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!("Tool not allowed for this agent: {}", call.name),
                is_error: true,
            };
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builtin_profile_can_hide_task_tools() {
        let executor = BuiltinToolExecutor::with_profile(
            Settings::default(),
            None,
            Some(vec!["file_read".to_string()]),
            false,
        );

        let definitions = executor.definitions().await;
        assert!(!definitions.iter().any(|item| item.name == "task_create"));
    }

    #[tokio::test]
    async fn builtin_profile_can_expose_task_tools_with_allowlist() {
        let executor = BuiltinToolExecutor::with_profile(
            Settings::default(),
            None,
            Some(vec!["file_read".to_string()]),
            true,
        );

        let definitions = executor.definitions().await;
        assert!(definitions.iter().any(|item| item.name == "task_create"));
        assert!(definitions.iter().any(|item| item.name == "file_read"));
    }
}
