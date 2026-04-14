use rustcode::{
    agents_runtime::{AgentTaskStatus, AgentTaskStore},
    config::Settings,
    file_history::FileHistoryStore,
    runtime::{ApprovalAction, PendingApproval, QueryEngine, QueryProgressEvent, RuntimeMessage},
    session::{Message, Session, SessionInfo, SessionManager, SessionStatus},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::PathBuf, process::Command, sync::Arc};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;

type CommandResult<T> = Result<T, String>;

const REPO_ROOT: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Clone)]
struct DesktopState {
    inner: Arc<Mutex<GuiState>>,
}

struct GuiState {
    settings: Settings,
    working_dir: Option<PathBuf>,
    session_manager: SessionManager,
    current_session: Session,
    history: Vec<RuntimeMessage>,
    pending_approval: Option<PendingApproval>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapPayload {
    project_name: String,
    project_path: String,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnTargetDto {
    message_id: String,
    short_id: String,
    content_preview: String,
    timestamp: String,
    has_tracked_files: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RewindPreviewDto {
    message_id: String,
    restored_input: String,
    modified_files: Vec<String>,
    deleted_files: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskSummaryDto {
    id: String,
    title: String,
    status: String,
    agent_name: String,
    updated_at: String,
    summary: String,
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
        let cwd = initial_working_dir();
        let session_manager = SessionManager::for_working_dir(cwd.as_deref());
        let (current_session, history, pending_approval) =
            restore_or_create_session(&session_manager, &settings)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(GuiState {
                settings,
                working_dir: cwd,
                session_manager,
                current_session,
                history,
                pending_approval,
            })),
        })
    }
}

fn initial_working_dir() -> Option<PathBuf> {
    PathBuf::from(REPO_ROOT)
        .parent()
        .map(std::path::Path::to_path_buf)
}

fn build_bootstrap_payload(guard: &GuiState) -> CommandResult<BootstrapPayload> {
    let sessions = guard
        .session_manager
        .list()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(SessionSummaryDto::from)
        .collect();

    Ok(BootstrapPayload {
        project_name: project_name_from_root(guard.working_dir.as_deref()),
        project_path: guard
            .working_dir
            .clone()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        settings: guard.settings.clone(),
        should_run_onboarding: guard.settings.should_run_onboarding(),
        sessions,
        current_session: session_summary_dto(&guard.current_session),
        transcript: transcript_from_session(&guard.current_session),
        pending_approval: pending_approval_dto(guard.pending_approval.as_ref()),
    })
}

fn switch_workspace(guard: &mut GuiState, working_dir: Option<PathBuf>) -> CommandResult<()> {
    let session_manager = SessionManager::for_working_dir(working_dir.as_deref());
    let (current_session, history, pending_approval) =
        restore_or_create_session(&session_manager, &guard.settings)
            .map_err(|error| error.to_string())?;

    if let Some(path) = &working_dir {
        guard.settings.working_dir = path.clone();
    }

    guard.working_dir = working_dir;
    guard.session_manager = session_manager;
    guard.current_session = current_session;
    guard.history = history;
    guard.pending_approval = pending_approval;
    Ok(())
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

fn project_name_from_root(root: Option<&std::path::Path>) -> String {
    root.and_then(|path| path.file_name())
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "rustcode".to_string())
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

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn session_status_label(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::AwaitingApproval => "awaiting_approval",
        SessionStatus::Interrupted => "interrupted",
        SessionStatus::Completed => "completed",
    }
}

fn task_status_label(status: AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Pending => "pending",
        AgentTaskStatus::Running => "running",
        AgentTaskStatus::AwaitingApproval => "awaiting_approval",
        AgentTaskStatus::Completed => "completed",
        AgentTaskStatus::Failed => "failed",
        AgentTaskStatus::Cancelled => "cancelled",
    }
}

fn emit_turn_failed(
    app: &AppHandle,
    turn_id: &str,
    session_id: &str,
    error: &str,
) -> CommandResult<()> {
    app.emit(
        "turn_failed",
        json!({
            "turnId": turn_id,
            "sessionId": session_id,
            "error": error,
        }),
    )
    .map_err(|emit_error| emit_error.to_string())
}

impl From<SessionInfo> for SessionSummaryDto {
    fn from(value: SessionInfo) -> Self {
        Self {
            id: value.id,
            name: value.name,
            status: session_status_label(value.status).to_string(),
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
    build_bootstrap_payload(&guard)
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
async fn create_session(state: State<'_, DesktopState>) -> CommandResult<RestorePayload> {
    let mut guard = state.inner.lock().await;
    let session = guard
        .session_manager
        .create(Some("New Session"))
        .map_err(|error| error.to_string())?;
    guard.current_session = session.clone();
    guard.history = Vec::new();
    guard.pending_approval = None;

    Ok(RestorePayload {
        session: session_summary_dto(&session),
        transcript: transcript_from_session(&session),
        pending_approval: None,
    })
}

#[tauri::command]
async fn delete_session(
    state: State<'_, DesktopState>,
    session_id: String,
) -> CommandResult<RestorePayload> {
    let mut guard = state.inner.lock().await;
    guard
        .session_manager
        .delete(&session_id)
        .map_err(|error| error.to_string())?;

    if guard.current_session.id == session_id {
        let next_session = guard
            .session_manager
            .list()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|session| session.id != session_id)
            .and_then(|session| {
                guard
                    .session_manager
                    .load(&session.id)
                    .map_err(|error| error.to_string())
                    .ok()
                    .flatten()
            });

        if let Some(session) = next_session {
            let restored = session.restore_runtime_state();
            guard.history = restored.history;
            guard.pending_approval = restored.pending_approval;
            guard.current_session = session;
        } else {
            let session = guard
                .session_manager
                .create(Some("New Session"))
                .map_err(|error| error.to_string())?;
            guard.current_session = session.clone();
            guard.history = Vec::new();
            guard.pending_approval = None;
        }
    }

    Ok(RestorePayload {
        session: session_summary_dto(&guard.current_session),
        transcript: transcript_from_session(&guard.current_session),
        pending_approval: pending_approval_dto(guard.pending_approval.as_ref()),
    })
}

fn open_path_in_shell(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn()?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("unsupported platform"))
}

#[tauri::command]
async fn open_project_folder(state: State<'_, DesktopState>) -> CommandResult<()> {
    let guard = state.inner.lock().await;
    let path = guard
        .working_dir
        .clone()
        .ok_or_else(|| "Project folder unavailable".to_string())?;
    open_path_in_shell(&path).map_err(|error| error.to_string())
}

#[tauri::command]
async fn choose_working_directory(
    state: State<'_, DesktopState>,
) -> CommandResult<Option<BootstrapPayload>> {
    let selected = rfd::FileDialog::new().pick_folder();
    let Some(path) = selected else {
        return Ok(None);
    };

    let mut guard = state.inner.lock().await;
    switch_workspace(&mut guard, Some(path))?;
    build_bootstrap_payload(&guard).map(Some)
}

#[tauri::command]
async fn list_user_turn_targets(
    state: State<'_, DesktopState>,
) -> CommandResult<Vec<TurnTargetDto>> {
    let guard = state.inner.lock().await;
    let file_history = FileHistoryStore::for_project(guard.working_dir.as_deref()).ok();
    let mut items = Vec::new();

    for message in &guard.current_session.messages {
        if !message.role.eq_ignore_ascii_case("user") {
            continue;
        }
        let preview = if message.content.trim().is_empty() {
            "(empty)".to_string()
        } else {
            message
                .content
                .replace('\n', " ")
                .chars()
                .take(120)
                .collect()
        };
        let has_tracked_files = file_history
            .as_ref()
            .map(|store| store.file_history_has_any_changes(&guard.current_session, &message.id))
            .unwrap_or(false);
        items.push(TurnTargetDto {
            message_id: message.id.clone(),
            short_id: short_id(&message.id),
            content_preview: preview,
            timestamp: message.timestamp.to_rfc3339(),
            has_tracked_files,
        });
    }

    Ok(items)
}

#[tauri::command]
async fn preview_rewind(
    state: State<'_, DesktopState>,
    message_id: String,
) -> CommandResult<RewindPreviewDto> {
    let guard = state.inner.lock().await;
    let target = guard
        .current_session
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .ok_or_else(|| format!("Message not found: {}", message_id))?;
    if !target.role.eq_ignore_ascii_case("user") {
        return Err(format!(
            "Rewind requires a user message id, got {}",
            target.role
        ));
    }

    let mut modified_files = Vec::new();
    let mut warnings = Vec::new();
    if let Ok(store) = FileHistoryStore::for_project(guard.working_dir.as_deref()) {
        if let Ok(descriptors) =
            store.file_history_get_change_descriptors(&guard.current_session, &message_id)
        {
            for descriptor in descriptors {
                modified_files.push(descriptor.path.clone());
                if descriptor.truncated {
                    warnings.push(format!(
                        "Tracked file list is truncated after {}",
                        descriptor.path
                    ));
                }
            }
        }
    }
    modified_files.sort();
    modified_files.dedup();
    warnings.sort();
    warnings.dedup();

    Ok(RewindPreviewDto {
        message_id,
        restored_input: target.content.clone(),
        modified_files,
        deleted_files: Vec::new(),
        warnings,
    })
}

#[tauri::command]
async fn rewind_session(
    state: State<'_, DesktopState>,
    message_id: String,
    files_only: bool,
) -> CommandResult<RestorePayload> {
    let mut guard = state.inner.lock().await;
    let working_dir = guard.working_dir.clone();
    let session_manager = SessionManager::for_working_dir(working_dir.as_deref());

    let mut session = guard.current_session.clone();
    if let Ok(store) = FileHistoryStore::for_project(working_dir.as_deref()) {
        let _ = store.rewind_session_to_message(&session, &message_id);
    }
    if !files_only {
        session_manager
            .rewind_session_to_message(&mut session, &message_id)
            .map_err(|error| error.to_string())?;
        let restored = session.restore_runtime_state();
        guard.history = restored.history;
        guard.pending_approval = restored.pending_approval;
    }
    session_manager
        .save(&session)
        .map_err(|error| error.to_string())?;
    guard.current_session = session.clone();

    Ok(RestorePayload {
        session: session_summary_dto(&session),
        transcript: transcript_from_session(&session),
        pending_approval: pending_approval_dto(guard.pending_approval.as_ref()),
    })
}

#[tauri::command]
async fn branch_session(
    state: State<'_, DesktopState>,
    message_id: Option<String>,
) -> CommandResult<RestorePayload> {
    let mut guard = state.inner.lock().await;
    let working_dir = guard.working_dir.clone();
    let session_manager = SessionManager::for_working_dir(working_dir.as_deref());
    let forked = session_manager
        .create_fork_session(
            &guard.current_session,
            message_id.as_deref(),
            Some(&format!("{} (branch)", guard.current_session.name)),
        )
        .map_err(|error| error.to_string())?;
    let restored = forked.restore_runtime_state();
    guard.history = restored.history;
    guard.pending_approval = restored.pending_approval;
    guard.current_session = forked.clone();

    Ok(RestorePayload {
        session: session_summary_dto(&forked),
        transcript: transcript_from_session(&forked),
        pending_approval: pending_approval_dto(guard.pending_approval.as_ref()),
    })
}

#[tauri::command]
async fn list_active_tasks(state: State<'_, DesktopState>) -> CommandResult<Vec<TaskSummaryDto>> {
    let guard = state.inner.lock().await;
    let store = AgentTaskStore::for_project(guard.working_dir.as_deref())
        .map_err(|error| error.to_string())?;
    let tasks = store
        .list_for_parent(Some(&guard.current_session.id))
        .map_err(|error| error.to_string())?;
    Ok(tasks
        .into_iter()
        .map(|task| TaskSummaryDto {
            id: task.id,
            title: task.subject,
            status: task_status_label(task.status).to_string(),
            agent_name: task.agent_type,
            updated_at: task.updated_at.to_rfc3339(),
            summary: task.result_summary.or(task.error).unwrap_or_else(|| {
                task.description
                    .replace('\n', " ")
                    .chars()
                    .take(160)
                    .collect()
            }),
        })
        .collect())
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
    let (settings, working_dir, session_id, history_before, mut session) = {
        let mut guard = state.inner.lock().await;
        let working_dir = guard.working_dir.clone();
        let session_manager = SessionManager::for_working_dir(working_dir.as_deref());
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
            working_dir,
            guard.current_session.id.clone(),
            history_before,
            session,
        )
    };

    let session_manager = SessionManager::for_working_dir(working_dir.as_deref());
    if let Some(path) = &working_dir {
        let _ = std::env::set_current_dir(path);
    }

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
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let error_text = error.to_string();
            session_manager
                .mark_status(&mut session, SessionStatus::Interrupted)
                .map_err(|mark_error| mark_error.to_string())?;
            {
                let mut guard = state.inner.lock().await;
                guard.current_session = session;
            }
            emit_turn_failed(&app, &turn_id, &session_id, &error_text)?;
            return Err(error_text);
        }
    };

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
    let (settings, working_dir, session_id, history, mut session, pending) = {
        let guard = state.inner.lock().await;
        let pending = guard
            .pending_approval
            .clone()
            .ok_or_else(|| "No pending approval in current session".to_string())?;
        (
            guard.settings.clone(),
            guard.working_dir.clone(),
            guard.current_session.id.clone(),
            guard.history.clone(),
            guard.current_session.clone(),
            pending,
        )
    };

    let session_manager = SessionManager::for_working_dir(working_dir.as_deref());
    if let Some(path) = &working_dir {
        let _ = std::env::set_current_dir(path);
    }

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
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let error_text = error.to_string();
            session_manager
                .mark_status(&mut session, SessionStatus::Interrupted)
                .map_err(|mark_error| mark_error.to_string())?;
            {
                let mut guard = state.inner.lock().await;
                guard.current_session = session;
            }
            emit_turn_failed(&app, &turn_id, &session_id, &error_text)?;
            return Err(error_text);
        }
    };

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileNode {
    name: String,
    path: String,
    is_dir: bool,
    children: Option<Vec<FileNode>>,
}

fn read_dir_recursive(path: &std::path::Path) -> Vec<FileNode> {
    let mut nodes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = entry.metadata().ok();
            let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            
            // Skip hidden files and common ignore patterns
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }

            let entry_path = entry.path().to_string_lossy().to_string();
            let children = if is_dir {
                Some(read_dir_recursive(&entry.path()))
            } else {
                None
            };

            nodes.push(FileNode {
                name,
                path: entry_path,
                is_dir,
                children,
            });
        }
    }
    nodes.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            b.is_dir.cmp(&a.is_dir)
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });
    nodes
}

#[tauri::command]
async fn get_file_tree(state: State<'_, DesktopState>) -> CommandResult<Vec<FileNode>> {
    let guard = state.inner.lock().await;
    let Some(path) = &guard.working_dir else {
        return Ok(Vec::new());
    };
    Ok(read_dir_recursive(path))
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
            create_session,
            delete_session,
            open_project_folder,
            choose_working_directory,
            restore_session,
            list_user_turn_targets,
            preview_rewind,
            rewind_session,
            branch_session,
            list_active_tasks,
            submit_prompt,
            respond_to_approval,
            get_file_tree,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
