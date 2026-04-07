use rustcode::{
    config::Settings,
    runtime::{ApprovalAction, PendingApproval, QueryEngine, QueryProgressEvent, RuntimeMessage},
    session::{Message, Session, SessionInfo, SessionManager, SessionStatus},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;

type CommandResult<T> = Result<T, String>;

#[derive(Clone)]
struct DesktopState {
    inner: Arc<Mutex<GuiState>>,
}

struct GuiState {
    settings: Settings,
    session_manager: SessionManager,
    current_session: Session,
    history: Vec<RuntimeMessage>,
    pending_approval: Option<PendingApproval>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapPayload {
    settings: Settings,
    should_run_onboarding: bool,
    sessions: Vec<SessionSummaryDto>,
    current_session: SessionSummaryDto,
    transcript: Vec<TranscriptMessageDto>,
    pending_approval: Option<PendingApprovalDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestorePayload {
    session: SessionSummaryDto,
    transcript: Vec<TranscriptMessageDto>,
    pending_approval: Option<PendingApprovalDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitPayload {
    session: SessionSummaryDto,
    transcript: Vec<TranscriptMessageDto>,
    pending_approval: Option<PendingApprovalDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummaryDto {
    id: String,
    name: String,
    status: String,
    session_kind: String,
    updated_at: String,
    message_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptMessageDto {
    id: String,
    role: String,
    content: String,
    entry_type: String,
    parent_id: Option<String>,
    timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingApprovalDto {
    tool_call_id: String,
    tool_name: String,
    reason: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovalChoice {
    AllowOnce,
    DenyOnce,
    AlwaysAllow,
    AlwaysDeny,
}

impl DesktopState {
    fn load() -> anyhow::Result<Self> {
        let settings = Settings::load()?;
        let cwd = current_working_dir();
        let session_manager = SessionManager::for_working_dir(cwd.as_deref());
        let (current_session, history, pending_approval) =
            restore_or_create_session(&session_manager, &settings)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(GuiState {
                settings,
                session_manager,
                current_session,
                history,
                pending_approval,
            })),
        })
    }
}

fn current_working_dir() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

fn restore_or_create_session(
    session_manager: &SessionManager,
    settings: &Settings,
) -> anyhow::Result<(Session, Vec<RuntimeMessage>, Option<PendingApproval>)> {
    if settings.session.auto_restore_last_session {
        if let Some(session) = session_manager.load_latest_resumable()? {
            let restored = session.restore_runtime_state();
            return Ok((session, restored.history, restored.pending_approval));
        }
    }

    let session = session_manager.create(Some("Desktop Session"))?;
    Ok((session, Vec::new(), None))
}

fn session_summary_dto(session: &Session) -> SessionSummaryDto {
    let info = SessionInfo::from(session);
    SessionSummaryDto::from(info)
}

fn transcript_from_session(session: &Session) -> Vec<TranscriptMessageDto> {
    session
        .messages
        .iter()
        .cloned()
        .map(TranscriptMessageDto::from)
        .collect()
}

fn pending_approval_dto(pending: Option<&PendingApproval>) -> Option<PendingApprovalDto> {
    pending.cloned().map(PendingApprovalDto::from)
}

impl From<SessionInfo> for SessionSummaryDto {
    fn from(value: SessionInfo) -> Self {
        Self {
            id: value.id,
            name: value.name,
            status: format!("{:?}", value.status).to_ascii_lowercase(),
            session_kind: format!("{:?}", value.session_kind).to_ascii_lowercase(),
            updated_at: value.updated_at.to_rfc3339(),
            message_count: value.message_count,
        }
    }
}

impl From<Message> for TranscriptMessageDto {
    fn from(value: Message) -> Self {
        Self {
            id: value.id,
            role: value.role,
            content: value.content,
            entry_type: format!("{:?}", value.entry_type).to_ascii_lowercase(),
            parent_id: value.parent_id,
            timestamp: value.timestamp.to_rfc3339(),
        }
    }
}

impl From<PendingApproval> for PendingApprovalDto {
    fn from(value: PendingApproval) -> Self {
        Self {
            tool_call_id: value.tool_call.id,
            tool_name: value.tool_call.name,
            reason: value.reason,
            arguments: value.tool_call.arguments,
        }
    }
}

struct TauriProgressSink {
    app: AppHandle,
    turn_id: String,
    session_id: String,
}

impl rustcode::runtime::ProgressSink for TauriProgressSink {
    fn emit(&mut self, event: QueryProgressEvent) {
        let event_name = match &event {
            QueryProgressEvent::ModelRequest { .. } => "model_request",
            QueryProgressEvent::ThinkingText(_) => "thinking_text_chunk",
            QueryProgressEvent::AssistantText(_) => "assistant_text_chunk",
            QueryProgressEvent::ToolCall(_) => "tool_call",
            QueryProgressEvent::ToolResult(_) => "tool_result",
            QueryProgressEvent::AwaitingApproval(_) => "awaiting_approval",
        };

        let payload = match event {
            QueryProgressEvent::ModelRequest { target } => json!({
                "turnId": self.turn_id,
                "sessionId": self.session_id,
                "target": target,
            }),
            QueryProgressEvent::ThinkingText(delta) => json!({
                "turnId": self.turn_id,
                "sessionId": self.session_id,
                "delta": delta,
            }),
            QueryProgressEvent::AssistantText(delta) => json!({
                "turnId": self.turn_id,
                "sessionId": self.session_id,
                "delta": delta,
            }),
            QueryProgressEvent::ToolCall(call) => json!({
                "turnId": self.turn_id,
                "sessionId": self.session_id,
                "toolCall": call,
            }),
            QueryProgressEvent::ToolResult(result) => json!({
                "turnId": self.turn_id,
                "sessionId": self.session_id,
                "toolResult": result,
            }),
            QueryProgressEvent::AwaitingApproval(pending) => json!({
                "turnId": self.turn_id,
                "sessionId": self.session_id,
                "pendingApproval": PendingApprovalDto::from(pending),
            }),
        };

        let _ = self.app.emit(event_name, payload);
    }
}

fn map_session_status(pending: Option<&PendingApproval>) -> SessionStatus {
    if pending.is_some() {
        SessionStatus::AwaitingApproval
    } else {
        SessionStatus::Completed
    }
}

#[tauri::command]
async fn bootstrap_gui_state(state: State<'_, DesktopState>) -> CommandResult<BootstrapPayload> {
    let guard = state.inner.lock().await;
    let sessions = guard
        .session_manager
        .list()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(SessionSummaryDto::from)
        .collect();

    Ok(BootstrapPayload {
        settings: guard.settings.clone(),
        should_run_onboarding: guard.settings.should_run_onboarding(),
        sessions,
        current_session: session_summary_dto(&guard.current_session),
        transcript: transcript_from_session(&guard.current_session),
        pending_approval: pending_approval_dto(guard.pending_approval.as_ref()),
    })
}

#[tauri::command]
async fn load_settings(state: State<'_, DesktopState>) -> CommandResult<Settings> {
    let guard = state.inner.lock().await;
    Ok(guard.settings.clone())
}

#[tauri::command]
async fn save_settings(
    state: State<'_, DesktopState>,
    settings: Settings,
) -> CommandResult<Settings> {
    settings.save().map_err(|error| error.to_string())?;
    let mut guard = state.inner.lock().await;
    guard.settings = settings.clone();
    Ok(settings)
}

#[tauri::command]
async fn complete_onboarding(state: State<'_, DesktopState>) -> CommandResult<Settings> {
    let mut guard = state.inner.lock().await;
    guard.settings.mark_onboarding_complete();
    guard.settings.save().map_err(|error| error.to_string())?;
    Ok(guard.settings.clone())
}

#[tauri::command]
async fn list_sessions(state: State<'_, DesktopState>) -> CommandResult<Vec<SessionSummaryDto>> {
    let guard = state.inner.lock().await;
    guard
        .session_manager
        .list()
        .map_err(|error| error.to_string())
        .map(|items| items.into_iter().map(SessionSummaryDto::from).collect())
}

#[tauri::command]
async fn restore_session(
    state: State<'_, DesktopState>,
    session_id: String,
) -> CommandResult<RestorePayload> {
    let mut guard = state.inner.lock().await;
    let Some(session) = guard
        .session_manager
        .load(&session_id)
        .map_err(|error| error.to_string())?
    else {
        return Err(format!("Session not found: {}", session_id));
    };
    let restored = session.restore_runtime_state();
    guard.history = restored.history;
    guard.pending_approval = restored.pending_approval;
    guard.current_session = session;

    Ok(RestorePayload {
        session: session_summary_dto(&guard.current_session),
        transcript: transcript_from_session(&guard.current_session),
        pending_approval: pending_approval_dto(guard.pending_approval.as_ref()),
    })
}

#[tauri::command]
async fn submit_prompt(
    app: AppHandle,
    state: State<'_, DesktopState>,
    prompt: String,
) -> CommandResult<SubmitPayload> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }

    let turn_id = Uuid::new_v4().to_string();
    let session_manager = SessionManager::for_working_dir(current_working_dir().as_deref());
    let (settings, session_id, history_before, mut session) = {
        let mut guard = state.inner.lock().await;
        let provisional = RuntimeMessage::user(prompt.clone());
        let history_before = guard.history.clone();
        let mut provisional_history = history_before.clone();
        provisional_history.push(provisional);
        let mut session = guard.current_session.clone();
        session_manager
            .save_runtime_state(
                &mut session,
                &provisional_history,
                SessionStatus::Active,
                None,
            )
            .map_err(|error| error.to_string())?;
        guard.history = provisional_history;
        guard.pending_approval = None;
        guard.current_session = session.clone();

        (
            guard.settings.clone(),
            guard.current_session.id.clone(),
            history_before,
            session,
        )
    };

    app.emit(
        "turn_started",
        json!({ "turnId": turn_id, "sessionId": session_id, "prompt": prompt }),
    )
    .map_err(|error| error.to_string())?;

    let engine = QueryEngine::new(settings);
    let mut progress = TauriProgressSink {
        app: app.clone(),
        turn_id: turn_id.clone(),
        session_id: session_id.clone(),
    };

    let result = engine
        .submit_message_with_context_and_progress(
            &history_before,
            RuntimeMessage::user(prompt),
            Some(session_id.clone()),
            &mut progress,
        )
        .await
        .map_err(|error| error.to_string())?;

    let pending = result.pending_approval.clone();
    session_manager
        .save_runtime_state(
            &mut session,
            &result.history,
            map_session_status(pending.as_ref()),
            pending.as_ref(),
        )
        .map_err(|error| error.to_string())?;

    let payload = SubmitPayload {
        session: session_summary_dto(&session),
        transcript: transcript_from_session(&session),
        pending_approval: pending_approval_dto(pending.as_ref()),
    };
    let completion_transcript = payload.transcript.clone();
    let completion_pending = payload.pending_approval.clone();

    {
        let mut guard = state.inner.lock().await;
        guard.history = result.history;
        guard.pending_approval = pending;
        guard.current_session = session.clone();
    }

    app.emit(
        "turn_completed",
        json!({
            "turnId": turn_id,
            "sessionId": session_id,
            "transcript": completion_transcript,
            "pendingApproval": completion_pending,
        }),
    )
    .map_err(|error| error.to_string())?;

    Ok(payload)
}

#[tauri::command]
async fn respond_to_approval(
    app: AppHandle,
    state: State<'_, DesktopState>,
    action: ApprovalChoice,
) -> CommandResult<SubmitPayload> {
    let turn_id = Uuid::new_v4().to_string();
    let session_manager = SessionManager::for_working_dir(current_working_dir().as_deref());
    let (settings, session_id, history, mut session, pending) = {
        let guard = state.inner.lock().await;
        let pending = guard
            .pending_approval
            .clone()
            .ok_or_else(|| "No pending approval in current session".to_string())?;
        (
            guard.settings.clone(),
            guard.current_session.id.clone(),
            guard.history.clone(),
            guard.current_session.clone(),
            pending,
        )
    };

    let approval_action = match action {
        ApprovalChoice::AllowOnce => ApprovalAction::AllowOnce(pending),
        ApprovalChoice::DenyOnce => ApprovalAction::DenyOnce(pending),
        ApprovalChoice::AlwaysAllow => ApprovalAction::AlwaysAllow(pending),
        ApprovalChoice::AlwaysDeny => ApprovalAction::AlwaysDeny(pending),
    };

    app.emit(
        "turn_started",
        json!({ "turnId": turn_id, "sessionId": session_id, "resume": true }),
    )
    .map_err(|error| error.to_string())?;

    let engine = QueryEngine::new(settings);
    let mut progress = TauriProgressSink {
        app: app.clone(),
        turn_id: turn_id.clone(),
        session_id: session_id.clone(),
    };

    let result = engine
        .resume_after_approval_with_context_and_progress(
            &history,
            approval_action,
            Some(session_id.clone()),
            &mut progress,
        )
        .await
        .map_err(|error| error.to_string())?;

    let pending = result.pending_approval.clone();
    session_manager
        .save_runtime_state(
            &mut session,
            &result.history,
            map_session_status(pending.as_ref()),
            pending.as_ref(),
        )
        .map_err(|error| error.to_string())?;

    let payload = SubmitPayload {
        session: session_summary_dto(&session),
        transcript: transcript_from_session(&session),
        pending_approval: pending_approval_dto(pending.as_ref()),
    };
    let completion_transcript = payload.transcript.clone();
    let completion_pending = payload.pending_approval.clone();

    {
        let mut guard = state.inner.lock().await;
        guard.history = result.history;
        guard.pending_approval = pending;
        guard.current_session = session.clone();
    }

    app.emit(
        "turn_completed",
        json!({
            "turnId": turn_id,
            "sessionId": session_id,
            "transcript": completion_transcript,
            "pendingApproval": completion_pending,
        }),
    )
    .map_err(|error| error.to_string())?;

    Ok(payload)
}

pub fn run() {
    tauri::Builder::default()
        .manage(DesktopState::load().expect("failed to initialize desktop state"))
        .invoke_handler(tauri::generate_handler![
            bootstrap_gui_state,
            load_settings,
            save_settings,
            complete_onboarding,
            list_sessions,
            restore_session,
            submit_prompt,
            respond_to_approval,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
