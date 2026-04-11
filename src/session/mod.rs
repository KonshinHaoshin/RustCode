//! Session Module - runtime transcript persistence

use crate::{
    compact::is_compact_summary_content,
    config::project_sessions_dir,
    runtime::{PendingApproval, RuntimeMessage, RuntimeRole, RuntimeToolCall, RuntimeToolResult},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DEFAULT_PRIMARY_SESSION_NAMES: &[&str] = &["tui-session", "Desktop Session", "New Session"];

#[derive(Debug, Clone, Default)]
pub struct SessionQuery {
    pub text: Option<String>,
    pub kind: Option<SessionKind>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SessionRestoreState {
    pub history: Vec<RuntimeMessage>,
    pub pending_approval: Option<PendingApproval>,
    pub status_message: String,
    pub restore_notice: Option<String>,
    pub lineage_summary: Option<String>,
}

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

    pub fn search(&self, query: SessionQuery) -> anyhow::Result<Vec<SessionInfo>> {
        let mut sessions = self.list()?;
        if let Some(kind) = query.kind {
            sessions.retain(|session| session.session_kind == kind);
        }

        if let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            let needle = text.to_ascii_lowercase();
            sessions.retain(|session| {
                session.id.to_ascii_lowercase().starts_with(&needle)
                    || session.name.to_ascii_lowercase().contains(&needle)
                    || session
                        .latest_user_summary
                        .as_deref()
                        .map(|summary| summary.to_ascii_lowercase().contains(&needle))
                        .unwrap_or(false)
                    || session.session_kind.as_str().contains(&needle)
            });
        }

        if let Some(limit) = query.limit {
            sessions.truncate(limit);
        }

        Ok(sessions)
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
            forked_from_session_id: None,
            forked_from_message_id: None,
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
        let lineage_notice = format!(
            "Child agent session for task {}{}.",
            task_id,
            parent_session_id
                .map(|parent| format!(" from parent {}", parent))
                .unwrap_or_default()
        );
        let session = Session {
            id,
            name: session_name,
            created_at: now,
            updated_at: now,
            project_root: self.project_root.clone(),
            parent_session_id: parent_session_id.map(str::to_string),
            forked_from_session_id: None,
            forked_from_message_id: None,
            spawned_by_task_id: Some(task_id.to_string()),
            session_kind: SessionKind::ChildAgent,
            status: SessionStatus::Active,
            pending_approval: None,
            messages: vec![Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "system".to_string(),
                content: lineage_notice,
                tool_calls: Vec::new(),
                tool_result: None,
                entry_type: TranscriptEntryType::SystemNotice,
                parent_id: None,
                timestamp: now,
            }],
        };
        self.save(&session)?;
        Ok(session)
    }

    pub fn create_fork_session(
        &self,
        source: &Session,
        up_to_message_id: Option<&str>,
        name: Option<&str>,
    ) -> anyhow::Result<Session> {
        std::fs::create_dir_all(&self.sessions_dir)?;

        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let forked_from_message_id = up_to_message_id.map(str::to_string);
        let mut messages = source.clone_for_fork(up_to_message_id)?;
        let lineage_notice = format!(
            "Forked from session {}{}.",
            source.id,
            forked_from_message_id
                .as_deref()
                .map(|message_id| format!(" at {}", message_id))
                .unwrap_or_default()
        );
        let parent_id = messages.last().map(|message| message.id.clone());
        messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "system".to_string(),
            content: lineage_notice,
            tool_calls: Vec::new(),
            tool_result: None,
            entry_type: TranscriptEntryType::SystemNotice,
            parent_id,
            timestamp: now,
        });
        let session = Session {
            id: id.clone(),
            name: name
                .map(str::to_string)
                .unwrap_or_else(|| format!("{} (branch)", source.name)),
            created_at: now,
            updated_at: now,
            project_root: self
                .project_root
                .clone()
                .or_else(|| source.project_root.clone()),
            parent_session_id: Some(source.id.clone()),
            forked_from_session_id: Some(source.id.clone()),
            forked_from_message_id,
            spawned_by_task_id: None,
            session_kind: SessionKind::Forked,
            status: SessionStatus::Completed,
            pending_approval: None,
            messages,
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
        session.messages = reconcile_messages(&session.messages, history);
        if should_refresh_session_name(session) {
            if let Some(name) = infer_session_name(session, history) {
                session.name = name;
            }
        }
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

    pub fn rewind_session_to_message(
        &self,
        session: &mut Session,
        message_id: &str,
    ) -> anyhow::Result<String> {
        let index = session
            .messages
            .iter()
            .position(|message| message.id == message_id)
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;
        let target = session
            .messages
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;
        if !target.role.eq_ignore_ascii_case("user") {
            return Err(anyhow::anyhow!(
                "Rewind requires a user message id, got role {}",
                target.role
            ));
        }
        let restored_input = target.content.clone();
        session.messages.truncate(index + 1);
        session.pending_approval = None;
        session.status = SessionStatus::Completed;
        session.updated_at = Utc::now();
        self.save(session)?;
        Ok(restored_input)
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
    pub forked_from_session_id: Option<String>,
    pub forked_from_message_id: Option<String>,
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
            forked_from_session_id: None,
            forked_from_message_id: None,
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
    Forked,
    ChildAgent,
}

impl Default for SessionKind {
    fn default() -> Self {
        Self::Primary
    }
}

impl Session {
    pub fn restore_runtime_state(&self) -> SessionRestoreState {
        let pending_approval = self.restore_pending_approval();
        let lineage_summary = self.lineage_summary();
        let restore_notice = self.lineage_notice();
        let status_message = match (&lineage_summary, pending_approval.is_some()) {
            (Some(summary), true) => format!("Restored {} with pending approval", summary),
            (Some(summary), false) => format!("Restored {}", summary),
            (None, true) => format!("Restored session {} with pending approval", self.id),
            (None, false) => format!("Restored session {}", self.id),
        };
        SessionRestoreState {
            history: self.runtime_history(),
            pending_approval,
            status_message,
            restore_notice,
            lineage_summary,
        }
    }

    pub fn lineage_notice(&self) -> Option<String> {
        match self.session_kind {
            SessionKind::Forked => Some(format!(
                "Forked from session {}{}.",
                self.forked_from_session_id.as_deref().unwrap_or("unknown"),
                self.forked_from_message_id
                    .as_deref()
                    .map(|message_id| format!(" at {}", message_id))
                    .unwrap_or_default()
            )),
            SessionKind::ChildAgent => Some(format!(
                "Child agent session for task {}{}.",
                self.spawned_by_task_id.as_deref().unwrap_or("unknown"),
                self.parent_session_id
                    .as_deref()
                    .map(|parent| format!(" from parent {}", parent))
                    .unwrap_or_default()
            )),
            SessionKind::Primary => None,
        }
    }

    pub fn lineage_summary(&self) -> Option<String> {
        match self.session_kind {
            SessionKind::Forked => Some(format!(
                "forked session {} from {}",
                self.id,
                self.forked_from_session_id.as_deref().unwrap_or("unknown")
            )),
            SessionKind::ChildAgent => Some(format!(
                "child session {} for task {}",
                self.id,
                self.spawned_by_task_id.as_deref().unwrap_or("unknown")
            )),
            SessionKind::Primary => None,
        }
    }

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
        normalize_message_lineage(&mut self.messages);
    }

    pub fn clone_for_fork(&self, up_to_message_id: Option<&str>) -> anyhow::Result<Vec<Message>> {
        let slice_end = match up_to_message_id {
            Some(message_id) => {
                let index = self
                    .messages
                    .iter()
                    .position(|message| message.id == message_id)
                    .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;
                let target = &self.messages[index];
                if !target.role.eq_ignore_ascii_case("user") {
                    return Err(anyhow::anyhow!(
                        "Branch requires a user message id, got role {}",
                        target.role
                    ));
                }
                index + 1
            }
            None => self.messages.len(),
        };

        let mut parent_id = None;
        let mut cloned = Vec::new();
        for message in self.messages.iter().take(slice_end) {
            let mut forked = message.clone();
            forked.id = uuid::Uuid::new_v4().to_string();
            forked.parent_id = parent_id.clone();
            parent_id = Some(forked.id.clone());
            cloned.push(forked);
        }
        Ok(cloned)
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
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RuntimeToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<RuntimeToolResult>,
    pub entry_type: TranscriptEntryType,
    pub parent_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: "system".to_string(),
            content: String::new(),
            tool_calls: Vec::new(),
            tool_result: None,
            entry_type: TranscriptEntryType::Message,
            parent_id: None,
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
            id: uuid::Uuid::new_v4().to_string(),
            role: value.role.as_str().to_string(),
            content: value.content.clone(),
            tool_calls: value.tool_calls.clone(),
            tool_result: value.tool_result.clone(),
            entry_type,
            parent_id: None,
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
    pub session_kind: SessionKind,
    pub forked_from_session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub spawned_by_task_id: Option<String>,
    pub message_count: usize,
    pub latest_user_summary: Option<String>,
}

impl From<&Session> for SessionInfo {
    fn from(value: &Session) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            status: value.status,
            session_kind: value.session_kind,
            forked_from_session_id: value.forked_from_session_id.clone(),
            parent_session_id: value.parent_session_id.clone(),
            spawned_by_task_id: value.spawned_by_task_id.clone(),
            message_count: value.messages.len(),
            latest_user_summary: value
                .messages
                .iter()
                .rev()
                .find(|message| message.role.eq_ignore_ascii_case("user"))
                .map(|message| summarize_session_text(&message.content, 72)),
        }
    }
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Primary => "primary",
            SessionKind::Forked => "forked",
            SessionKind::ChildAgent => "child_agent",
        }
    }

    pub fn parse_filter(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "primary" | "main" | "root" => Some(SessionKind::Primary),
            "fork" | "forked" | "branch" | "branched" => Some(SessionKind::Forked),
            "child" | "child_agent" | "child-agent" | "agent" | "subagent" => {
                Some(SessionKind::ChildAgent)
            }
            _ => None,
        }
    }
}

fn should_refresh_session_name(session: &Session) -> bool {
    if session.session_kind != SessionKind::Primary {
        return false;
    }

    session.name.trim().is_empty()
        || session.name == session.id
        || DEFAULT_PRIMARY_SESSION_NAMES
            .iter()
            .any(|name| session.name.eq_ignore_ascii_case(name))
}

fn infer_session_name(session: &Session, history: &[RuntimeMessage]) -> Option<String> {
    history
        .iter()
        .find(|message| message.role == RuntimeRole::User && !message.content.trim().is_empty())
        .map(|message| summarize_session_text(&message.content, 48))
        .filter(|name| !name.is_empty())
        .or_else(|| {
            (!session.id.is_empty()).then(|| {
                format!(
                    "Session {}",
                    &session.id.chars().take(8).collect::<String>()
                )
            })
        })
}

fn summarize_session_text(text: &str, max: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max {
        compact
    } else {
        compact.chars().take(max).collect::<String>() + "..."
    }
}

fn reconcile_messages(existing: &[Message], history: &[RuntimeMessage]) -> Vec<Message> {
    let mut messages = Vec::with_capacity(history.len());
    let mut parent_id = None;
    let mut diverged = false;

    for (index, runtime) in history.iter().enumerate() {
        let reused = if !diverged {
            existing
                .get(index)
                .filter(|message| runtime_matches_message(runtime, message))
        } else {
            None
        };

        let mut message = if let Some(existing) = reused {
            existing.clone()
        } else {
            diverged = true;
            Message::from(runtime)
        };
        message.parent_id = parent_id.clone();
        parent_id = Some(message.id.clone());
        messages.push(message);
    }

    messages
}

fn runtime_matches_message(runtime: &RuntimeMessage, message: &Message) -> bool {
    message.role == runtime.role.as_str()
        && message.content == runtime.content
        && message.tool_calls == runtime.tool_calls
        && message.tool_result == runtime.tool_result
}

fn normalize_message_lineage(messages: &mut [Message]) {
    let mut parent_id = None;
    for message in messages {
        if message.id.trim().is_empty() {
            message.id = uuid::Uuid::new_v4().to_string();
        }
        message.parent_id = parent_id.clone();
        parent_id = Some(message.id.clone());
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
            metadata: std::collections::HashMap::new(),
        };
        let session = Session {
            messages: vec![Message {
                id: "tool-1".to_string(),
                role: "tool".to_string(),
                content: "ignored".to_string(),
                tool_calls: Vec::new(),
                tool_result: Some(tool_result),
                entry_type: TranscriptEntryType::Message,
                parent_id: None,
                timestamp: Utc::now(),
            }],
            ..Session::default()
        };

        assert!(session.runtime_history().is_empty());
    }

    #[test]
    fn create_fork_session_clones_history_with_new_ids() {
        let manager = SessionManager::new();
        let source = Session {
            id: "source".to_string(),
            name: "Source".to_string(),
            messages: vec![
                Message {
                    id: "user-1".to_string(),
                    role: "user".to_string(),
                    content: "first".to_string(),
                    tool_calls: Vec::new(),
                    tool_result: None,
                    entry_type: TranscriptEntryType::Message,
                    parent_id: None,
                    timestamp: Utc::now(),
                },
                Message {
                    id: "assistant-1".to_string(),
                    role: "assistant".to_string(),
                    content: "answer".to_string(),
                    tool_calls: Vec::new(),
                    tool_result: None,
                    entry_type: TranscriptEntryType::Message,
                    parent_id: Some("user-1".to_string()),
                    timestamp: Utc::now(),
                },
            ],
            ..Session::default()
        };

        let fork = manager
            .create_fork_session(&source, Some("user-1"), Some("Fork"))
            .unwrap();
        assert_eq!(fork.session_kind, SessionKind::Forked);
        assert_eq!(fork.messages.len(), 2);
        assert_ne!(fork.messages[0].id, "user-1");
        assert_eq!(
            fork.messages[1].entry_type,
            TranscriptEntryType::SystemNotice
        );
        assert_eq!(fork.forked_from_session_id.as_deref(), Some("source"));
    }

    #[test]
    fn create_child_session_includes_lineage_notice() {
        let manager = SessionManager::new();
        let child = manager
            .create_child_session(Some("parent-1"), "task-1", Some("Child"))
            .unwrap();

        assert_eq!(child.session_kind, SessionKind::ChildAgent);
        assert_eq!(child.messages.len(), 1);
        assert_eq!(
            child.messages[0].entry_type,
            TranscriptEntryType::SystemNotice
        );
        assert!(child.messages[0].content.contains("task task-1"));
    }

    #[test]
    fn restore_runtime_state_reports_child_lineage() {
        let session = Session {
            id: "child-session".to_string(),
            parent_session_id: Some("parent-1".to_string()),
            spawned_by_task_id: Some("task-1".to_string()),
            session_kind: SessionKind::ChildAgent,
            ..Session::default()
        };

        let restored = session.restore_runtime_state();
        assert!(restored
            .status_message
            .contains("child session child-session for task task-1"));
        assert!(restored.restore_notice.is_some());
        assert_eq!(
            restored.lineage_summary.as_deref(),
            Some("child session child-session for task task-1")
        );
    }

    #[test]
    fn compact_summary_is_persisted_as_compact_boundary() {
        let message = Message::from(&RuntimeMessage::compact_summary("summary"));

        assert_eq!(message.entry_type, TranscriptEntryType::CompactBoundary);
    }

    #[test]
    fn save_runtime_state_auto_names_primary_session_from_first_user_turn() {
        let manager = SessionManager::new();
        let mut session = Session {
            id: "session-1".to_string(),
            name: "New Session".to_string(),
            session_kind: SessionKind::Primary,
            ..Session::default()
        };

        manager
            .save_runtime_state(
                &mut session,
                &[
                    RuntimeMessage::user("Investigate why the Android build keeps failing"),
                    RuntimeMessage::assistant("Looking into it"),
                ],
                SessionStatus::Completed,
                None,
            )
            .unwrap();

        assert!(session
            .name
            .starts_with("Investigate why the Android build"));
    }

    #[test]
    fn session_info_captures_latest_user_summary() {
        let session = Session {
            messages: vec![
                Message::from(&RuntimeMessage::user("first prompt")),
                Message::from(&RuntimeMessage::assistant("response")),
                Message::from(&RuntimeMessage::user("latest prompt summary text")),
            ],
            ..Session::default()
        };

        let info = SessionInfo::from(&session);
        assert_eq!(
            info.latest_user_summary.as_deref(),
            Some("latest prompt summary text")
        );
    }

    #[test]
    fn session_kind_parse_filter_accepts_aliases() {
        assert_eq!(
            SessionKind::parse_filter("branch"),
            Some(SessionKind::Forked)
        );
        assert_eq!(
            SessionKind::parse_filter("subagent"),
            Some(SessionKind::ChildAgent)
        );
        assert_eq!(
            SessionKind::parse_filter("main"),
            Some(SessionKind::Primary)
        );
        assert_eq!(SessionKind::parse_filter("unknown"), None);
    }
}
