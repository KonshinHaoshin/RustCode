use crate::config::project_tasks_dir;
use crate::{
    agents_runtime::types::{
        AgentTask, AgentTaskNotification, AgentTaskPendingApproval, AgentTaskStatus,
    },
    runtime::PendingApproval,
};
use chrono::Utc;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AgentTaskStore {
    tasks_dir: PathBuf,
}

impl AgentTaskStore {
    pub fn new(tasks_dir: PathBuf) -> Self {
        Self { tasks_dir }
    }

    pub fn for_project(project_root: Option<&Path>) -> anyhow::Result<Self> {
        let tasks_dir = project_tasks_dir(project_root)
            .ok_or_else(|| anyhow::anyhow!("Unable to determine project task directory"))?;
        Ok(Self::new(tasks_dir))
    }

    pub fn create(
        &self,
        subject: impl Into<String>,
        description: impl Into<String>,
        agent_type: impl Into<String>,
        parent_session_id: Option<String>,
        metadata: serde_json::Value,
    ) -> anyhow::Result<AgentTask> {
        std::fs::create_dir_all(&self.tasks_dir)?;
        let now = Utc::now();
        let task = AgentTask {
            id: uuid::Uuid::new_v4().to_string(),
            subject: subject.into(),
            description: description.into(),
            agent_type: agent_type.into(),
            parent_session_id,
            child_session_id: None,
            status: AgentTaskStatus::Pending,
            created_at: now,
            updated_at: now,
            result_summary: None,
            error: None,
            pending_approval: None,
            metadata,
            delivered_to_parent_at: None,
        };
        self.save(&task)?;
        Ok(task)
    }

    pub fn get(&self, id: &str) -> anyhow::Result<Option<AgentTask>> {
        let path = self.task_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&content)?))
    }

    pub fn list(&self) -> anyhow::Result<Vec<AgentTask>> {
        if !self.tasks_dir.exists() {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::new();
        for entry in std::fs::read_dir(&self.tasks_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let content = std::fs::read_to_string(path)?;
            if let Ok(task) = serde_json::from_str::<AgentTask>(&content) {
                tasks.push(task);
            }
        }
        tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(tasks)
    }

    pub fn list_for_parent(
        &self,
        parent_session_id: Option<&str>,
    ) -> anyhow::Result<Vec<AgentTask>> {
        let tasks = self.list()?;
        Ok(match parent_session_id {
            Some(parent_id) => tasks
                .into_iter()
                .filter(|task| task.parent_session_id.as_deref() == Some(parent_id))
                .collect(),
            None => tasks,
        })
    }

    pub fn mark_running(&self, id: &str) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            task.status = AgentTaskStatus::Running;
            task.updated_at = Utc::now();
            task.error = None;
            task.pending_approval = None;
        })
    }

    pub fn attach_child_session(&self, id: &str, child_session_id: &str) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            task.child_session_id = Some(child_session_id.to_string());
            task.updated_at = Utc::now();
        })
    }

    pub fn complete(&self, id: &str, summary: impl Into<String>) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            task.status = AgentTaskStatus::Completed;
            task.updated_at = Utc::now();
            task.result_summary = Some(summary.into());
            task.error = None;
            task.pending_approval = None;
        })
    }

    pub fn fail(&self, id: &str, error: impl Into<String>) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            task.status = AgentTaskStatus::Failed;
            task.updated_at = Utc::now();
            task.error = Some(error.into());
            task.pending_approval = None;
        })
    }

    pub fn mark_awaiting_approval(
        &self,
        id: &str,
        pending: &PendingApproval,
    ) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            task.status = AgentTaskStatus::AwaitingApproval;
            task.updated_at = Utc::now();
            task.error = None;
            task.pending_approval = Some(AgentTaskPendingApproval {
                tool_call: pending.tool_call.clone(),
                reason: pending.reason.clone(),
            });
        })
    }

    pub fn resume_after_approval(&self, id: &str) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            task.status = AgentTaskStatus::Running;
            task.updated_at = Utc::now();
            task.error = None;
            task.pending_approval = None;
        })
    }

    pub fn update_metadata(
        &self,
        id: &str,
        description: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        self.mutate_task(id, |task| {
            if let Some(description) = description {
                task.description = description;
            }
            if let Some(metadata) = metadata {
                task.metadata = metadata;
            }
            task.updated_at = Utc::now();
        })
    }

    pub fn drain_notifications(
        &self,
        parent_session_id: &str,
    ) -> anyhow::Result<Vec<AgentTaskNotification>> {
        let mut notifications = Vec::new();
        for task in self.list_for_parent(Some(parent_session_id))? {
            if task.delivered_to_parent_at.is_some() {
                continue;
            }
            if !matches!(
                task.status,
                AgentTaskStatus::Completed | AgentTaskStatus::Failed
            ) {
                continue;
            }
            notifications.push(AgentTaskNotification::from(&task));
            self.mutate_task(&task.id, |entry| {
                entry.delivered_to_parent_at = Some(Utc::now());
                entry.updated_at = Utc::now();
            })?;
        }
        Ok(notifications)
    }

    fn mutate_task(&self, id: &str, apply: impl FnOnce(&mut AgentTask)) -> anyhow::Result<()> {
        let mut task = self
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", id))?;
        apply(&mut task);
        self.save(&task)
    }

    fn save(&self, task: &AgentTask) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.tasks_dir)?;
        std::fs::write(
            self.task_path(&task.id),
            serde_json::to_string_pretty(task)?,
        )?;
        Ok(())
    }

    fn task_path(&self, id: &str) -> PathBuf {
        self.tasks_dir.join(format!("{id}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeToolCall;

    #[test]
    fn mark_awaiting_approval_persists_pending_tool() {
        let temp = tempfile::tempdir().unwrap();
        let store = AgentTaskStore::new(temp.path().join("tasks"));
        let task = store
            .create(
                "subject",
                "description",
                "explore",
                Some("parent-session".to_string()),
                serde_json::json!({}),
            )
            .unwrap();

        let pending = PendingApproval {
            tool_call: RuntimeToolCall {
                id: "call-1".to_string(),
                name: "execute_command".to_string(),
                arguments: serde_json::json!({ "command": "cargo test" }),
            },
            reason: "approval required".to_string(),
        };
        store.mark_awaiting_approval(&task.id, &pending).unwrap();

        let persisted = store.get(&task.id).unwrap().unwrap();
        assert_eq!(persisted.status, AgentTaskStatus::AwaitingApproval);
        assert_eq!(
            persisted
                .pending_approval
                .as_ref()
                .map(|item| &item.tool_call),
            Some(&pending.tool_call)
        );
    }
}
