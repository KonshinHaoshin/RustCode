use crate::{
    agents_runtime::{spawn_agent_task, AgentTaskStatus, AgentTaskStore},
    config::{project_root_from, Settings},
    file_history::{FileHistoryBatchEntry, FileHistoryOrigin, FileHistoryStore},
    runtime::types::{RuntimeToolCall, RuntimeToolResult},
    services::agents::AgentsService,
    tools::{ToolError, ToolOutput, ToolRegistry},
    tools_runtime::{registry::builtin_tool_definitions, ToolDefinition},
};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;

const EXECUTE_COMMAND_TRACK_LIMIT: usize = 200;

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
                if let Some(parent_session_id) = task.parent_session_id {
                    lines.push(format!("Parent session: {}", parent_session_id));
                }
                if let Some(child_session_id) = task.child_session_id {
                    lines.push(format!("Child session: {}", child_session_id));
                }
                if let Some(pending) = task.pending_approval {
                    lines.push(format!("Pending approval tool: {}", pending.tool_call.name));
                    lines.push(format!("Pending approval reason: {}", pending.reason));
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
            metadata: std::collections::HashMap::new(),
        })
    }

    fn capture_file_history_metadata(
        &self,
        call: &RuntimeToolCall,
        context: &ToolExecutionContext,
    ) -> Option<serde_json::Value> {
        if !matches!(call.name.as_str(), "file_write" | "file_edit") {
            return None;
        }
        let session_id = context.session_id.as_deref()?;
        let file_path = call.arguments.get("file_path")?.as_str()?;
        let store = FileHistoryStore::for_project(self.project_root.as_deref()).ok()?;
        let mutation = store.capture_mutation(session_id, file_path).ok()?;
        serde_json::to_value(mutation).ok()
    }

    fn capture_execute_command_snapshot(
        &self,
        call: &RuntimeToolCall,
        context: &ToolExecutionContext,
    ) -> Option<(
        FileHistoryStore,
        String,
        crate::file_history::CommandSnapshot,
    )> {
        if call.name != "execute_command" {
            return None;
        }
        let session_id = context.session_id.as_deref()?.to_string();
        let store = FileHistoryStore::for_project(self.project_root.as_deref()).ok()?;
        let snapshot = store
            .capture_command_snapshot(&session_id, EXECUTE_COMMAND_TRACK_LIMIT)
            .ok()?;
        Some((store, session_id, snapshot))
    }

    fn apply_execute_command_file_history(
        &self,
        metadata: &mut std::collections::HashMap<String, serde_json::Value>,
        snapshot: Option<(
            FileHistoryStore,
            String,
            crate::file_history::CommandSnapshot,
        )>,
    ) {
        let Some((store, session_id, snapshot)) = snapshot else {
            return;
        };
        let Ok((batch, truncated)) =
            store.diff_command_snapshot(&snapshot, &session_id, EXECUTE_COMMAND_TRACK_LIMIT)
        else {
            return;
        };

        if !batch.is_empty() {
            let legacy_batch = batch.clone();
            let v2_batch = batch
                .into_iter()
                .map(|mutation| FileHistoryBatchEntry {
                    mutation,
                    origin: FileHistoryOrigin::ExecuteCommandSnapshot,
                })
                .collect::<Vec<_>>();
            if let Ok(value) = serde_json::to_value(legacy_batch) {
                metadata.insert("file_history_batch".to_string(), value);
            }
            if let Ok(value) = serde_json::to_value(v2_batch) {
                metadata.insert("file_history_batch_v2".to_string(), value);
            }
            metadata.insert(
                "file_history_origin".to_string(),
                serde_json::Value::String("execute_command_snapshot".to_string()),
            );
            metadata.insert(
                "file_history_tracking_mode".to_string(),
                serde_json::Value::String("bounded_project_snapshot".to_string()),
            );
        }
        if truncated {
            metadata.insert(
                "file_history_truncated".to_string(),
                serde_json::Value::Bool(true),
            );
        }
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
                    metadata: std::collections::HashMap::new(),
                },
            };
        }

        if !self.is_allowed(&call.name) {
            return RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!("Tool not allowed for this agent: {}", call.name),
                is_error: true,
                metadata: std::collections::HashMap::new(),
            };
        }

        let file_history_metadata = self.capture_file_history_metadata(call, context);
        let execute_command_snapshot = self.capture_execute_command_snapshot(call, context);
        match self
            .registry
            .execute(&call.name, call.arguments.clone())
            .await
        {
            Ok(mut output) => {
                if let Some(file_history) = file_history_metadata {
                    output
                        .metadata
                        .insert("file_history".to_string(), file_history);
                }
                self.apply_execute_command_file_history(
                    &mut output.metadata,
                    execute_command_snapshot,
                );
                let metadata = output.metadata.clone();
                let content = self.render_success(output);
                RuntimeToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    content,
                    is_error: false,
                    metadata,
                }
            }
            Err(error) => RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: self.render_error(error),
                is_error: true,
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    self.apply_execute_command_file_history(
                        &mut metadata,
                        execute_command_snapshot,
                    );
                    metadata
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeToolCall;

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

    #[tokio::test]
    async fn task_get_formats_pending_approval_tool() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().to_path_buf();
        let store = AgentTaskStore::for_project(Some(project_root.as_path())).unwrap();
        let task = store
            .create(
                "inspect",
                "inspect code",
                "explore",
                Some("parent-1".to_string()),
                serde_json::json!({}),
            )
            .unwrap();
        store.attach_child_session(&task.id, "child-1").unwrap();
        store
            .mark_awaiting_approval(
                &task.id,
                &crate::runtime::PendingApproval {
                    tool_call: RuntimeToolCall {
                        id: "call-1".to_string(),
                        name: "execute_command".to_string(),
                        arguments: serde_json::json!({"command": "cargo test"}),
                    },
                    reason: "approval required".to_string(),
                },
            )
            .unwrap();

        let executor = BuiltinToolExecutor::new(Settings::default(), Some(project_root));
        let result = executor
            .execute_task_tool(
                &RuntimeToolCall {
                    id: "tool-1".to_string(),
                    name: "task_get".to_string(),
                    arguments: serde_json::json!({"task_id": task.id}),
                },
                &ToolExecutionContext::default(),
            )
            .await
            .unwrap();

        assert!(result.content.contains("Parent session: parent-1"));
        assert!(result.content.contains("Child session: child-1"));
        assert!(result
            .content
            .contains("Pending approval tool: execute_command"));
    }
}
