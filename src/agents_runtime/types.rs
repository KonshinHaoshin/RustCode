use crate::runtime::RuntimeToolCall;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentMemoryScope {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionMode {
    Inherit,
    DenySensitive,
    BackgroundSafe,
}

impl Default for AgentPermissionMode {
    fn default() -> Self {
        Self::BackgroundSafe
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentContextStrategy {
    TaskOnly,
    TaskPlusCompactSummary,
    TaskPlusRecentAssistantSummary,
}

impl Default for AgentContextStrategy {
    fn default() -> Self {
        Self::TaskPlusCompactSummary
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Pending,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl Default for AgentTaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentTask {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub agent_type: String,
    pub parent_session_id: Option<String>,
    pub child_session_id: Option<String>,
    pub status: AgentTaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub pending_approval: Option<AgentTaskPendingApproval>,
    pub metadata: Value,
    pub delivered_to_parent_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTaskPendingApproval {
    pub tool_call: RuntimeToolCall,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskNotification {
    pub id: String,
    pub subject: String,
    pub status: AgentTaskStatus,
    pub result_summary: Option<String>,
    pub error: Option<String>,
}

impl From<&AgentTask> for AgentTaskNotification {
    fn from(value: &AgentTask) -> Self {
        Self {
            id: value.id.clone(),
            subject: value.subject.clone(),
            status: value.status,
            result_summary: value.result_summary.clone(),
            error: value.error.clone(),
        }
    }
}
