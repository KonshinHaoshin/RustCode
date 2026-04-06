//! Session Module - runtime transcript persistence

use crate::{
    config::project_sessions_dir,
    runtime::{RuntimeMessage, RuntimeRole, RuntimeToolCall, RuntimeToolResult},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Session manager
pub struct SessionManager {
    sessions_dir: PathBuf,
}

impl SessionManager {
    /// Create a new session manager rooted in the global config directory.
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            sessions_dir: home.join(".rustcode").join("sessions"),
        }
    }

    /// Create a session manager for a project working directory.
    pub fn for_working_dir(cwd: Option<&Path>) -> Self {
        if let Some(project_dir) = project_sessions_dir(cwd) {
            return Self {
                sessions_dir: project_dir,
            };
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

    pub fn load_latest(&self) -> anyhow::Result<Option<Session>> {
        let Some(info) = self.list()?.into_iter().next() else {
            return Ok(None);
        };
        self.load(&info.id)
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
        let session = serde_json::from_str(&content)?;
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

    /// Replace transcript contents for a session.
    pub fn save_transcript(
        &self,
        session: &mut Session,
        history: &[RuntimeMessage],
    ) -> anyhow::Result<()> {
        session.updated_at = Utc::now();
        session.messages = history.iter().map(Message::from).collect();
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
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<Message>,
}

impl Session {
    pub fn runtime_history(&self) -> Vec<RuntimeMessage> {
        self.messages.iter().map(RuntimeMessage::from).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RuntimeToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<RuntimeToolResult>,
    pub timestamp: DateTime<Utc>,
}

impl From<&RuntimeMessage> for Message {
    fn from(value: &RuntimeMessage) -> Self {
        Self {
            role: value.role.as_str().to_string(),
            content: value.content.clone(),
            tool_calls: value.tool_calls.clone(),
            tool_result: value.tool_result.clone(),
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
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
}

impl From<&Session> for SessionInfo {
    fn from(value: &Session) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            message_count: value.messages.len(),
        }
    }
}
