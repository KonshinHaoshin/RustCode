//! Session Module - runtime transcript persistence

use crate::{
    compact::is_compact_summary_content,
    config::project_sessions_dir,
    runtime::{PendingApproval, RuntimeMessage, RuntimeRole, RuntimeToolCall, RuntimeToolResult},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Session manager
pub struct SessionManager {
    sessions_dir: PathBuf,
    project_root: Option<PathBuf>,
}

impl SessionManager {
    /// Create a new session manager rooted in the global config directory.
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            sessions_dir: home.join(".rustcode").join("sessions"),
            project_root: None,
        }
    }

    /// Create a session manager for a project working directory.
    pub fn for_working_dir(cwd: Option<&Path>) -> Self {
        if let Some(project_root) = cwd.map(Path::to_path_buf) {
            if let Some(project_dir) = project_sessions_dir(Some(&project_root)) {
                return Self {
                    sessions_dir: project_dir,
                    project_root: Some(project_root),
                };
            }
        }
        Self::new()
    }

    /// List all sessions sorted by most recent update time.
    pub fn list(&self) -> anyhow::Result<Vec<SessionInfo>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        sessions.push(SessionInfo::from(&session));
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    pub fn list_recent(&self) -> anyhow::Result<Vec<SessionInfo>> {
        self.list()
    }

    pub fn load_latest(&self) -> anyhow::Result<Option<Session>> {
        let Some(info) = self.list()?.into_iter().next() else {
            return Ok(None);
        };
        self.load(&info.id)
    }

    pub fn load_latest_resumable(&self) -> anyhow::Result<Option<Session>> {
        self.load_latest()
    }

    /// Create a new session.
    pub fn create(&self, name: Option<&str>) -> anyhow::Result<Session> {
        std::fs::create_dir_all(&self.sessions_dir)?;

        let id = uuid::Uuid::new_v4().to_string();
        let session_name = name.unwrap_or(&id).to_string();
        let now = Utc::now();
        let session = Session {
            id,
            name: session_name,
            created_at: now,
            updated_at: now,
            project_root: self.project_root.clone(),
            parent_session_id: None,
            spawned_by_task_id: None,
            session_kind: SessionKind::Primary,
            status: SessionStatus::Active,
            pending_approval: None,
            messages: Vec::new(),
        };
        self.save(&session)?;
        Ok(session)
    }

    pub fn create_child_session(
        &self,
        parent_session_id: Option<&str>,
        task_id: &str,
        name: Option<&str>,
    ) -> anyhow::Result<Session> {
        std::fs::create_dir_all(&self.sessions_dir)?;

        let id = uuid::Uuid::new_v4().to_string();
        let session_name = name.unwrap_or("child-agent-session").to_string();
        let now = Utc::now();
        let session = Session {
            id,
            name: session_name,
            created_at: now,
            updated_at: now,
            project_root: self.project_root.clone(),
            parent_session_id: parent_session_id.map(str::to_string),
            spawned_by_task_id: Some(task_id.to_string()),
            session_kind: SessionKind::ChildAgent,
            status: SessionStatus::Active,
            pending_approval: None,
            messages: Vec::new(),
        };
        self.save(&session)?;
        Ok(session)
    }

    /// Load a session by ID.
    pub fn load(&self, id: &str) -> anyhow::Result<Option<Session>> {
        let path = self.sessions_dir.join(format!("{}.json", id));

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)?;
        let mut session: Session = serde_json::from_str(&content)?;
        session.normalize_for_runtime();
        Ok(Some(session))
    }

    /// Save a session.
    pub fn save(&self, session: &Session) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.sessions_dir)?;
        let path = self.sessions_dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(session)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Replace transcript contents and runtime state for a session.
    pub fn save_runtime_state(
        &self,
        session: &mut Session,
        history: &[RuntimeMessage],
        status: SessionStatus,
        pending_approval: Option<&PendingApproval>,
    ) -> anyhow::Result<()> {
        session.updated_at = Utc::now();
        session.status = status;
        session.pending_approval = pending_approval.map(StoredPendingApproval::from);
        session.messages = history.iter().map(Message::from).collect();
        self.save(session)
    }

    /// Replace transcript contents for a session.
    pub fn save_transcript(
        &self,
        session: &mut Session,
        history: &[RuntimeMessage],
    ) -> anyhow::Result<()> {
        let status = if session.pending_approval.is_some() {
            SessionStatus::AwaitingApproval
        } else {
            SessionStatus::Completed
        };
        let pending = session
            .pending_approval
            .as_ref()
            .map(PendingApproval::from_stored);
        self.save_runtime_state(session, history, status, pending.as_ref())
    }

    pub fn update_pending_approval(
        &self,
        session: &mut Session,
        pending: Option<StoredPendingApproval>,
    ) -> anyhow::Result<()> {
        session.updated_at = Utc::now();
        session.pending_approval = pending;
        session.status = if session.pending_approval.is_some() {
            SessionStatus::AwaitingApproval
        } else {
            SessionStatus::Completed
        };
        self.save(session)
    }

    pub fn mark_status(&self, session: &mut Session, status: SessionStatus) -> anyhow::Result<()> {
        session.updated_at = Utc::now();
        session.status = status;
        self.save(session)
    }

    /// Delete a session by ID.
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let path = self.sessions_dir.join(format!("{}.json", id));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_root: Option<PathBuf>,
    pub parent_session_id: Option<String>,
    pub spawned_by_task_id: Option<String>,
    pub session_kind: SessionKind,
    pub status: SessionStatus,
    pub pending_approval: Option<StoredPendingApproval>,
    pub messages: Vec<Message>,
}

impl Default for Session {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: String::new(),
            name: String::new(),
            created_at: now,
            updated_at: now,
            project_root: None,
            parent_session_id: None,
            spawned_by_task_id: None,
            session_kind: SessionKind::Primary,
            status: SessionStatus::Completed,
            pending_approval: None,
            messages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Primary,
    ChildAgent,
}

impl Default for SessionKind {
    fn default() -> Self {
        Self::Primary
    }
}

impl Session {
    pub fn runtime_history(&self) -> Vec<RuntimeMessage> {
        let mut history: Vec<RuntimeMessage> = Vec::new();

        for message in &self.messages {
            let runtime = RuntimeMessage::from(message);
            if history.is_empty() && runtime.role == RuntimeRole::Tool {
                continue;
            }

            if runtime.role == RuntimeRole::Tool {
                let has_tool_call = history.iter().rev().any(|entry| {
                    entry.role == RuntimeRole::Assistant
                        && entry.tool_calls.iter().any(|call| {
                            message
                                .tool_result
                                .as_ref()
                                .is_some_and(|result| result.tool_call_id == call.id)
                        })
                });
                if !has_tool_call {
                    history.push(RuntimeMessage::system(format!(
                        "Recovered orphaned tool result for {} during session restore.",
                        message
                            .tool_result
                            .as_ref()
                            .map(|result| result.name.as_str())
                            .unwrap_or("unknown tool")
                    )));
                    continue;
                }
            }

            history.push(runtime);
        }

        history
    }

    pub fn restore_pending_approval(&self) -> Option<PendingApproval> {
        self.pending_approval
            .as_ref()
            .map(PendingApproval::from_stored)
    }

    fn normalize_for_runtime(&mut self) {
        if self.status == SessionStatus::AwaitingApproval && self.pending_approval.is_none() {
            self.status = SessionStatus::Completed;
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    AwaitingApproval,
    Interrupted,
    Completed,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self::Completed
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptEntryType {
    Message,
    CommandResult,
    CompactBoundary,
    SystemNotice,
}

impl Default for TranscriptEntryType {
    fn default() -> Self {
        Self::Message
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RuntimeToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<RuntimeToolResult>,
    pub entry_type: TranscriptEntryType,
    pub timestamp: DateTime<Utc>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: "system".to_string(),
            content: String::new(),
            tool_calls: Vec::new(),
            tool_result: None,
            entry_type: TranscriptEntryType::Message,
            timestamp: Utc::now(),
        }
    }
}

impl From<&RuntimeMessage> for Message {
    fn from(value: &RuntimeMessage) -> Self {
        let entry_type = if value.is_compact_summary() || is_compact_summary_content(&value.content)
        {
            TranscriptEntryType::CompactBoundary
        } else if value.role == RuntimeRole::System {
            TranscriptEntryType::SystemNotice
        } else {
            TranscriptEntryType::Message
        };

        Self {
            role: value.role.as_str().to_string(),
            content: value.content.clone(),
            tool_calls: value.tool_calls.clone(),
            tool_result: value.tool_result.clone(),
            entry_type,
            timestamp: Utc::now(),
        }
    }
}

impl From<&Message> for RuntimeMessage {
    fn from(value: &Message) -> Self {
        let role = match value.role.as_str() {
            "system" => RuntimeRole::System,
            "assistant" => RuntimeRole::Assistant,
            "tool" => RuntimeRole::Tool,
            _ => RuntimeRole::User,
        };

        RuntimeMessage {
            role,
            content: value.content.clone(),
            tool_calls: value.tool_calls.clone(),
            tool_result: value.tool_result.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPendingApproval {
    pub tool_name: String,
    pub tool_call_id: String,
    pub arguments: serde_json::Value,
    pub reason: String,
}

impl From<&PendingApproval> for StoredPendingApproval {
    fn from(value: &PendingApproval) -> Self {
        Self {
            tool_name: value.tool_call.name.clone(),
            tool_call_id: value.tool_call.id.clone(),
            arguments: value.tool_call.arguments.clone(),
            reason: value.reason.clone(),
        }
    }
}

impl PendingApproval {
    fn from_stored(value: &StoredPendingApproval) -> Self {
        Self {
            tool_call: RuntimeToolCall {
                id: value.tool_call_id.clone(),
                name: value.tool_name.clone(),
                arguments: value.arguments.clone(),
            },
            reason: value.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub message_count: usize,
}

impl From<&Session> for SessionInfo {
    fn from(value: &Session) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            status: value.status,
            message_count: value.messages.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeToolCall;

    #[test]
    fn restore_pending_approval_round_trip() {
        let pending = PendingApproval {
            tool_call: RuntimeToolCall {
                id: "call-1".to_string(),
                name: "file_read".to_string(),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
            },
            reason: "Tool file_read requires approval".to_string(),
        };

        let session = Session {
            pending_approval: Some(StoredPendingApproval::from(&pending)),
            status: SessionStatus::AwaitingApproval,
            ..Session::default()
        };

        let restored = session
            .restore_pending_approval()
            .expect("pending approval");
        assert_eq!(restored, pending);
    }

    #[test]
    fn runtime_history_drops_orphaned_leading_tool_result() {
        let tool_result = RuntimeToolResult {
            tool_call_id: "call-1".to_string(),
            name: "file_read".to_string(),
            content: "ignored".to_string(),
            is_error: false,
        };
        let session = Session {
            messages: vec![Message {
                role: "tool".to_string(),
                content: "ignored".to_string(),
                tool_calls: Vec::new(),
                tool_result: Some(tool_result),
                entry_type: TranscriptEntryType::Message,
                timestamp: Utc::now(),
            }],
            ..Session::default()
        };

        assert!(session.runtime_history().is_empty());
    }

    #[test]
    fn compact_summary_is_persisted_as_compact_boundary() {
        let message = Message::from(&RuntimeMessage::compact_summary("summary"));

        assert_eq!(message.entry_type, TranscriptEntryType::CompactBoundary);
    }
}
