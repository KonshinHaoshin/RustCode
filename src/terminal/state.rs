use crate::{
    agents_runtime::AgentTaskStatus,
    config::{ApiProvider, FallbackTarget, ProjectLocalPermissions, Settings},
    onboarding::OnboardingDraft,
    permissions::{events::PermissionEvent, PermissionsSettings},
    runtime::{PendingApproval, QueryProgressEvent, QueryTurnResult, RuntimeMessage, RuntimeRole},
    session::{Session, SessionInfo, SessionManager, SessionStatus, TranscriptEntryType},
    terminal::theme::SPINNER_FRAMES,
};
use ratatui::{layout::Rect, text::Line, widgets::Paragraph};
use std::{
    sync::{mpsc::Receiver, Arc},
    time::{Duration, Instant},
};

const SPINNER_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRole {
    User,
    Assistant,
    Thinking,
    System,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptViewMode {
    Main,
    Transcript,
}

#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: DisplayRole,
    pub content: String,
    pub message_id: Option<String>,
    pub parent_id: Option<String>,
    pub entry_type: Option<TranscriptEntryType>,
}

impl DisplayMessage {
    pub fn transient(role: DisplayRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            message_id: None,
            parent_id: None,
            entry_type: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskProgressItem {
    pub id: String,
    pub subject: String,
    pub agent_type: String,
    pub status: AgentTaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPoint {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: SelectionPoint,
    pub focus: SelectionPoint,
}

impl TextSelection {
    pub fn normalized(self) -> (SelectionPoint, SelectionPoint) {
        if self.anchor.line < self.focus.line
            || (self.anchor.line == self.focus.line && self.anchor.column <= self.focus.column)
        {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_single_point(self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Char,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionClickState {
    pub point: SelectionPoint,
    pub count: u8,
    pub at: Instant,
}

#[derive(Debug, Clone)]
pub struct PendingApprovalViewModel {
    pub pending: PendingApproval,
    pub arguments_preview: String,
    pub focus_index: usize,
    pub origin: PendingApprovalOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingApprovalOrigin {
    FreshTurn,
    RestoredSession,
    ChildTask {
        task_id: String,
        child_session_id: String,
        subject: String,
    },
}

#[derive(Debug, Clone)]
pub struct ResumePickerState {
    pub sessions: Vec<SessionInfo>,
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSelectorMode {
    Branch,
    Rewind { files_only: bool },
}

#[derive(Debug, Clone)]
pub struct MessageSelectorItem {
    pub message_id: String,
    pub preview: String,
    pub has_file_changes: bool,
}

#[derive(Debug, Clone)]
pub struct MessageSelectorConfirmationState {
    pub message_id: String,
    pub preview: String,
    pub changed_files: Vec<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MessageSelectorState {
    pub mode: MessageSelectorMode,
    pub items: Vec<MessageSelectorItem>,
    pub selected: usize,
    pub confirmation: Option<MessageSelectorConfirmationState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionSection {
    Allow,
    Deny,
    Ask,
    Recent,
}

#[derive(Debug, Clone)]
pub struct PermissionsViewState {
    pub section: PermissionSection,
    pub selected: usize,
    pub global_permissions: PermissionsSettings,
    pub local_permissions: ProjectLocalPermissions,
    pub recent_events: Vec<PermissionEvent>,
}

#[derive(Debug)]
pub struct ChatWorkerResult {
    pub outcome: anyhow::Result<ChatWorkerOutcome>,
}

#[derive(Debug)]
pub enum ChatWorkerOutcome {
    Turn(QueryTurnResult),
    TaskResume { message: String },
}

#[derive(Debug)]
pub enum ChatWorkerUpdate {
    Progress(QueryProgressEvent),
    Finished(ChatWorkerResult),
}

#[derive(Debug)]
pub struct PendingChatRequest {
    pub receiver: Receiver<ChatWorkerUpdate>,
    pub base_history: Arc<Vec<RuntimeMessage>>,
    pub user_message: Option<RuntimeMessage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Onboarding,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    Welcome,
    Primary,
    FallbackList,
    FallbackEdit,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryField {
    Provider,
    Protocol,
    CustomName,
    BaseUrl,
    Model,
    ApiKey,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackField {
    Provider,
    Protocol,
    CustomName,
    BaseUrl,
    Model,
    ApiKey,
    Save,
}

pub struct TerminalState {
    pub settings: Settings,
    pub draft: OnboardingDraft,
    pub view: ViewMode,
    pub onboarding_step: OnboardingStep,
    pub onboarding_focus: usize,
    pub selected_fallback: usize,
    pub editing_fallback: Option<usize>,
    pub status: String,
    pub input: String,
    pub should_quit: bool,
    pub confirm_exit_deadline: Option<Instant>,
    pub messages: Vec<DisplayMessage>,
    pub conversation_history: Arc<Vec<RuntimeMessage>>,
    pub pending_response: Option<PendingChatRequest>,
    pub pending_approval: Option<PendingApprovalViewModel>,
    pub resume_picker: Option<ResumePickerState>,
    pub message_selector: Option<MessageSelectorState>,
    pub permissions_view: Option<PermissionsViewState>,
    pub thinking: bool,
    pub transcript_mode: TranscriptViewMode,
    pub verbose_transcript: bool,
    pub initial_prompt: Option<String>,
    pub spinner_tick: usize,
    pub last_tick: Instant,
    pub working_dir: String,
    pub scroll_offset: usize,
    pub chat_render_cache: Paragraph<'static>,
    pub chat_render_width: u16,
    pub chat_render_line_count: u16,
    pub chat_render_dirty: bool,
    pub chat_plain_lines: Vec<String>,
    pub chat_area: Rect,
    pub chat_scroll_row: usize,
    pub selection: Option<TextSelection>,
    pub selection_mode: SelectionMode,
    pub selection_dragging: bool,
    pub last_selection_click: Option<SelectionClickState>,
    pub session_manager: SessionManager,
    pub active_session: Option<Session>,
    pub active_session_id: Option<String>,
    pub last_usage_total: Option<usize>,
    pub live_assistant_message: Option<usize>,
    pub live_thinking_message: Option<usize>,
    pub active_tasks: Vec<TaskProgressItem>,
}

impl TerminalState {
    pub fn new(settings: Settings, initial_prompt: Option<String>) -> Self {
        let view = if settings.should_run_onboarding() {
            ViewMode::Onboarding
        } else {
            ViewMode::Chat
        };
        let working_dir_path = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let working_dir = working_dir_path
            .to_str()
            .map(str::to_string)
            .unwrap_or_else(|| "~".to_string());
        let session_manager = SessionManager::for_working_dir(Some(&working_dir_path));
        let mut restored_session_notice = None;
        let mut active_session = None;
        let mut active_session_id = None;
        let mut conversation_history = Arc::new(Vec::new());
        let mut messages = Vec::new();
        let mut pending_approval = None;

        if view == ViewMode::Chat
            && settings.session.persist_transcript
            && settings.session.auto_restore_last_session
        {
            match session_manager.load_latest_resumable() {
                Ok(Some(session)) => {
                    let restored = session.restore_runtime_state();
                    conversation_history = Arc::new(restored.history);
                    messages = Self::display_messages_from_session(&session);
                    pending_approval =
                        restored
                            .pending_approval
                            .map(|pending| PendingApprovalViewModel {
                                arguments_preview: format_arguments_preview(
                                    &pending.tool_call.arguments,
                                ),
                                pending,
                                focus_index: 0,
                                origin: PendingApprovalOrigin::RestoredSession,
                            });
                    restored_session_notice = Some(restored.status_message);
                    active_session_id = Some(session.id.clone());
                    active_session = Some(session);
                }
                Ok(None) => {}
                Err(error) => {
                    restored_session_notice = Some(format!("Session restore failed: {}", error));
                }
            }
        }

        let mut status = if view == ViewMode::Onboarding {
            "First run detected. Complete onboarding to start coding.".to_string()
        } else {
            "Ready.".to_string()
        };
        if let Some(notice) = &restored_session_notice {
            status = notice.clone();
        }

        Self {
            draft: OnboardingDraft::from_settings(&settings),
            settings,
            view,
            onboarding_step: if view == ViewMode::Onboarding {
                OnboardingStep::Welcome
            } else {
                OnboardingStep::Summary
            },
            onboarding_focus: 0,
            selected_fallback: 0,
            editing_fallback: None,
            status,
            input: String::new(),
            should_quit: false,
            confirm_exit_deadline: None,
            messages,
            conversation_history,
            pending_response: None,
            pending_approval,
            resume_picker: None,
            message_selector: None,
            permissions_view: None,
            thinking: false,
            transcript_mode: TranscriptViewMode::Main,
            verbose_transcript: false,
            initial_prompt,
            spinner_tick: 0,
            last_tick: Instant::now(),
            working_dir,
            scroll_offset: 0,
            chat_render_cache: Paragraph::new(Vec::<Line<'static>>::new()),
            chat_render_width: 0,
            chat_render_line_count: 0,
            chat_render_dirty: true,
            chat_plain_lines: Vec::new(),
            chat_area: Rect::default(),
            chat_scroll_row: 0,
            selection: None,
            selection_mode: SelectionMode::Char,
            selection_dragging: false,
            last_selection_click: None,
            session_manager,
            active_session,
            active_session_id,
            last_usage_total: None,
            live_assistant_message: None,
            live_thinking_message: None,
            active_tasks: Vec::new(),
        }
    }

    pub fn consume_initial_prompt(&mut self) -> Option<String> {
        if self.view == ViewMode::Chat {
            self.initial_prompt.take()
        } else {
            None
        }
    }

    pub fn active_primary_fields(&self) -> Vec<PrimaryField> {
        let mut fields = vec![PrimaryField::Provider];
        if self.draft.provider == ApiProvider::Custom {
            fields.push(PrimaryField::Protocol);
            fields.push(PrimaryField::CustomName);
        }
        fields.extend([
            PrimaryField::BaseUrl,
            PrimaryField::Model,
            PrimaryField::ApiKey,
            PrimaryField::Continue,
        ]);
        fields
    }

    pub fn active_fallback_fields(&self, target: &FallbackTarget) -> Vec<FallbackField> {
        let mut fields = vec![FallbackField::Provider];
        if target.provider == ApiProvider::Custom {
            fields.push(FallbackField::Protocol);
            fields.push(FallbackField::CustomName);
        }
        fields.extend([
            FallbackField::BaseUrl,
            FallbackField::Model,
            FallbackField::ApiKey,
            FallbackField::Save,
        ]);
        fields
    }

    pub fn current_fallback(&self) -> Option<&FallbackTarget> {
        let index = self.editing_fallback?;
        self.draft.fallback_chain.get(index)
    }

    pub fn clear_exit_confirmation_if_stale(&mut self) {
        if self
            .confirm_exit_deadline
            .is_some_and(|deadline| Instant::now() > deadline)
        {
            self.confirm_exit_deadline = None;
        }
    }

    pub fn request_exit(&mut self) {
        if self
            .confirm_exit_deadline
            .is_some_and(|deadline| Instant::now() <= deadline)
        {
            self.should_quit = true;
        } else {
            self.confirm_exit_deadline = Some(Instant::now() + Duration::from_secs(2));
            self.status = "Press Ctrl+C again within 2s to exit.".to_string();
        }
    }

    pub fn complete_onboarding(&mut self) -> anyhow::Result<()> {
        self.draft.apply_to_settings(&mut self.settings);
        self.settings.mark_onboarding_complete();
        self.settings.save()?;
        self.view = ViewMode::Chat;
        self.status = "Onboarding complete. Provider settings saved.".to_string();
        self.messages.push(DisplayMessage::transient(
            DisplayRole::Assistant,
            format!(
                "Configured {}/{} with {} fallback target(s).",
                self.draft.provider_label(),
                self.draft.model,
                self.draft.fallback_chain.len()
            ),
        ));
        self.mark_chat_render_dirty();
        Ok(())
    }

    pub fn mark_chat_render_dirty(&mut self) {
        self.chat_render_dirty = true;
    }

    pub fn tick_spinner(&mut self) -> bool {
        if !self.thinking {
            return false;
        }
        if self.last_tick.elapsed() >= SPINNER_INTERVAL {
            self.spinner_tick = self.spinner_tick.wrapping_add(1);
            self.last_tick = Instant::now();
            return true;
        }
        false
    }

    pub fn time_until_next_spinner_frame(&self) -> Option<Duration> {
        if !self.thinking {
            return None;
        }
        Some(SPINNER_INTERVAL.saturating_sub(self.last_tick.elapsed()))
    }

    pub fn spinner_char(&self) -> char {
        let len = SPINNER_FRAMES.len();
        let cycle = len * 2 - 2;
        let pos = self.spinner_tick % cycle;
        if pos < len {
            SPINNER_FRAMES[pos]
        } else {
            SPINNER_FRAMES[cycle - pos]
        }
    }

    pub fn refresh_display_messages(&mut self) {
        self.messages = Self::display_messages_from_history(&self.conversation_history);
        self.mark_chat_render_dirty();
    }

    pub fn replace_history(&mut self, history: Vec<RuntimeMessage>) {
        self.conversation_history = Arc::new(history);
        self.live_assistant_message = None;
        self.live_thinking_message = None;
        self.clear_selection();
        self.refresh_display_messages();
    }

    pub fn reset_conversation(&mut self) {
        self.input.clear();
        self.replace_history(Vec::new());
        self.set_pending_approval(None);
        self.resume_picker = None;
        self.message_selector = None;
        self.permissions_view = None;
        self.active_session = None;
        self.active_session_id = None;
        self.last_usage_total = None;
        self.transcript_mode = TranscriptViewMode::Main;
        self.verbose_transcript = false;
        self.live_assistant_message = None;
        self.live_thinking_message = None;
        self.active_tasks.clear();
        self.scroll_offset = 0;
        self.thinking = false;
        self.clear_selection();
    }

    pub fn set_pending_approval(&mut self, pending: Option<PendingApproval>) {
        self.set_pending_approval_with_origin(pending, PendingApprovalOrigin::FreshTurn);
    }

    pub fn set_pending_approval_with_origin(
        &mut self,
        pending: Option<PendingApproval>,
        origin: PendingApprovalOrigin,
    ) {
        self.pending_approval = pending.map(|pending| PendingApprovalViewModel {
            arguments_preview: format_arguments_preview(&pending.tool_call.arguments),
            pending,
            focus_index: 0,
            origin,
        });
        self.mark_chat_render_dirty();
    }

    pub fn persist_current_session(&mut self) -> anyhow::Result<()> {
        if !self.settings.session.persist_transcript {
            return Ok(());
        }

        if self.active_session.is_none() {
            self.active_session = Some(self.session_manager.create(Some("tui-session"))?);
            self.active_session_id = self
                .active_session
                .as_ref()
                .map(|session| session.id.clone());
        }

        let session_status = self.current_session_status();
        let pending_approval =
            self.pending_approval
                .as_ref()
                .and_then(|pending| match pending.origin {
                    PendingApprovalOrigin::FreshTurn | PendingApprovalOrigin::RestoredSession => {
                        Some(pending.pending.clone())
                    }
                    PendingApprovalOrigin::ChildTask { .. } => None,
                });
        if let Some(session) = &mut self.active_session {
            self.session_manager.save_runtime_state(
                session,
                &self.conversation_history,
                session_status,
                pending_approval.as_ref(),
            )?;
            self.active_session_id = Some(session.id.clone());
            self.messages = Self::display_messages_from_session(session);
            self.mark_chat_render_dirty();
        }

        Ok(())
    }

    pub fn restore_session(&mut self, session: Session) {
        let restored = session.restore_runtime_state();
        self.replace_history(restored.history);
        self.messages = Self::display_messages_from_session(&session);
        self.pending_approval = restored
            .pending_approval
            .map(|pending| PendingApprovalViewModel {
                arguments_preview: format_arguments_preview(&pending.tool_call.arguments),
                pending,
                focus_index: 0,
                origin: PendingApprovalOrigin::RestoredSession,
            });
        self.active_session_id = Some(session.id.clone());
        self.status = restored.status_message;
        self.active_session = Some(session);
        self.resume_picker = None;
        self.message_selector = None;
        self.permissions_view = None;
        self.last_usage_total = None;
        self.live_assistant_message = None;
        self.live_thinking_message = None;
        self.active_tasks.clear();
        self.clear_selection();
        self.mark_chat_render_dirty();
    }

    pub fn set_active_tasks(&mut self, tasks: Vec<TaskProgressItem>) {
        self.active_tasks = tasks;
        self.mark_chat_render_dirty();
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_mode = SelectionMode::Char;
        self.selection_dragging = false;
    }

    pub fn has_selection(&self) -> bool {
        self.selection
            .is_some_and(|selection| !selection.is_single_point())
    }

    pub fn begin_selection(
        &mut self,
        anchor: SelectionPoint,
        focus: SelectionPoint,
        mode: SelectionMode,
    ) {
        self.selection = Some(TextSelection { anchor, focus });
        self.selection_mode = mode;
        self.selection_dragging = true;
        self.mark_chat_render_dirty();
    }

    pub fn update_selection(&mut self, point: SelectionPoint) {
        if let Some(selection) = self.selection.as_mut() {
            selection.focus = point;
            self.mark_chat_render_dirty();
        }
    }

    pub fn finish_selection(&mut self, point: SelectionPoint) {
        if let Some(selection) = self.selection.as_mut() {
            selection.focus = point;
        }
        self.selection_dragging = false;
        self.mark_chat_render_dirty();
    }

    pub fn selection_text(&self) -> Option<String> {
        let selection = self.selection?;
        let (start, end) = selection.normalized();
        if start == end {
            return None;
        }

        let mut parts = Vec::new();
        for line_index in start.line..=end.line {
            let line = self.chat_plain_lines.get(line_index)?;
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                parts.push(String::new());
                continue;
            }

            let start_column = if line_index == start.line {
                start.column.min(chars.len().saturating_sub(1))
            } else {
                0
            };
            let end_column = if line_index == end.line {
                end.column.min(chars.len().saturating_sub(1))
            } else {
                chars.len().saturating_sub(1)
            };

            if start_column > end_column {
                parts.push(String::new());
                continue;
            }

            let text = chars[start_column..=end_column].iter().collect::<String>();
            parts.push(text);
        }

        Some(parts.join("\n"))
    }

    pub fn append_streaming_assistant_text(&mut self, chunk: &str) {
        self.append_streaming_message_for_role(DisplayRole::Assistant, chunk);
    }

    pub fn append_streaming_thinking_text(&mut self, chunk: &str) {
        self.append_streaming_message_for_role(DisplayRole::Thinking, chunk);
    }

    fn append_streaming_message_for_role(&mut self, role: DisplayRole, chunk: &str) {
        if chunk.is_empty() {
            return;
        }

        let live_index = match role {
            DisplayRole::Assistant => &mut self.live_assistant_message,
            DisplayRole::Thinking => &mut self.live_thinking_message,
            _ => return,
        };

        let index = match *live_index {
            Some(index) => index,
            None => {
                self.messages.push(DisplayMessage {
                    role,
                    content: String::new(),
                    message_id: None,
                    parent_id: None,
                    entry_type: None,
                });
                let index = self.messages.len().saturating_sub(1);
                *live_index = Some(index);
                index
            }
        };

        if let Some(message) = self.messages.get_mut(index) {
            message.content.push_str(chunk);
            self.mark_chat_render_dirty();
        }
    }

    pub fn current_session_status(&self) -> SessionStatus {
        if self.pending_approval.as_ref().is_some_and(|pending| {
            !matches!(pending.origin, PendingApprovalOrigin::ChildTask { .. })
        }) {
            SessionStatus::AwaitingApproval
        } else if self.thinking {
            SessionStatus::Active
        } else {
            SessionStatus::Completed
        }
    }

    fn display_messages_from_history(history: &[RuntimeMessage]) -> Vec<DisplayMessage> {
        let mut messages = Vec::new();

        for message in history {
            match message.role {
                RuntimeRole::User => messages.push(DisplayMessage {
                    role: DisplayRole::User,
                    content: message.content.clone(),
                    message_id: None,
                    parent_id: None,
                    entry_type: None,
                }),
                RuntimeRole::Assistant => {
                    if !message.content.trim().is_empty() {
                        messages.push(DisplayMessage {
                            role: DisplayRole::Assistant,
                            content: message.content.clone(),
                            message_id: None,
                            parent_id: None,
                            entry_type: None,
                        });
                    }
                    for tool_call in &message.tool_calls {
                        messages.push(DisplayMessage {
                            role: DisplayRole::Tool,
                            content: format!(
                                "Tool request: {} {}",
                                tool_call.name,
                                format_arguments_preview(&tool_call.arguments)
                            ),
                            message_id: None,
                            parent_id: None,
                            entry_type: None,
                        });
                    }
                }
                RuntimeRole::System => messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: message.content.clone(),
                    message_id: None,
                    parent_id: None,
                    entry_type: None,
                }),
                RuntimeRole::Tool => {
                    let tool_result = message.tool_result.as_ref();
                    let label = tool_result
                        .map(|result| {
                            if result.is_error {
                                format!("Tool error: {}", result.name)
                            } else {
                                format!("Tool result: {}", result.name)
                            }
                        })
                        .unwrap_or_else(|| "Tool result".to_string());
                    messages.push(DisplayMessage {
                        role: DisplayRole::Tool,
                        content: format!("{}{}", label, format_tool_body(&message.content)),
                        message_id: None,
                        parent_id: None,
                        entry_type: None,
                    });
                }
            }
        }

        messages
    }

    pub fn display_messages_from_session(session: &Session) -> Vec<DisplayMessage> {
        let mut messages = Vec::new();

        for message in &session.messages {
            match message.role.as_str() {
                "user" => messages.push(DisplayMessage {
                    role: DisplayRole::User,
                    content: message.content.clone(),
                    message_id: Some(message.id.clone()),
                    parent_id: message.parent_id.clone(),
                    entry_type: Some(message.entry_type),
                }),
                "assistant" => {
                    if !message.content.trim().is_empty() {
                        messages.push(DisplayMessage {
                            role: DisplayRole::Assistant,
                            content: message.content.clone(),
                            message_id: Some(message.id.clone()),
                            parent_id: message.parent_id.clone(),
                            entry_type: Some(message.entry_type),
                        });
                    }
                    for tool_call in &message.tool_calls {
                        messages.push(DisplayMessage {
                            role: DisplayRole::Tool,
                            content: format!(
                                "Tool request: {} {}",
                                tool_call.name,
                                format_arguments_preview(&tool_call.arguments)
                            ),
                            message_id: Some(message.id.clone()),
                            parent_id: message.parent_id.clone(),
                            entry_type: Some(message.entry_type),
                        });
                    }
                }
                "system" => messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: message.content.clone(),
                    message_id: Some(message.id.clone()),
                    parent_id: message.parent_id.clone(),
                    entry_type: Some(message.entry_type),
                }),
                "tool" => {
                    let tool_result = message.tool_result.as_ref();
                    let label = tool_result
                        .map(|result| {
                            if result.is_error {
                                format!("Tool error: {}", result.name)
                            } else {
                                format!("Tool result: {}", result.name)
                            }
                        })
                        .unwrap_or_else(|| "Tool result".to_string());
                    messages.push(DisplayMessage {
                        role: DisplayRole::Tool,
                        content: format!("{}{}", label, format_tool_body(&message.content)),
                        message_id: Some(message.id.clone()),
                        parent_id: message.parent_id.clone(),
                        entry_type: Some(message.entry_type),
                    });
                }
                _ => messages.push(DisplayMessage {
                    role: DisplayRole::User,
                    content: message.content.clone(),
                    message_id: Some(message.id.clone()),
                    parent_id: message.parent_id.clone(),
                    entry_type: Some(message.entry_type),
                }),
            }
        }

        messages
    }
}

pub(crate) fn format_arguments_preview(arguments: &serde_json::Value) -> String {
    let pretty = serde_json::to_string_pretty(arguments)
        .unwrap_or_else(|_| arguments.to_string())
        .replace('\r', "");
    let lines = pretty.lines().take(12).collect::<Vec<_>>();
    let mut preview = lines.join(" ");
    if preview.len() > 1000 {
        preview.truncate(1000);
        preview.push_str("...");
    }
    preview
}

pub(crate) fn format_tool_body(content: &str) -> String {
    if content.trim().is_empty() {
        String::new()
    } else {
        format!("\n{}", content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_text_collects_multiline_range() {
        let settings = Settings::default();
        let mut state = TerminalState::new(settings, None);
        state.chat_plain_lines = vec![
            "hello world".to_string(),
            "second line".to_string(),
            "tail".to_string(),
        ];
        state.selection = Some(TextSelection {
            anchor: SelectionPoint { line: 0, column: 6 },
            focus: SelectionPoint { line: 1, column: 5 },
        });

        assert_eq!(state.selection_text().as_deref(), Some("world\nsecond"));
    }

    #[test]
    fn append_streaming_assistant_text_reuses_live_message() {
        let mut state = TerminalState::new(Settings::default(), None);

        state.append_streaming_assistant_text("hel");
        state.append_streaming_assistant_text("lo");

        assert_eq!(state.live_assistant_message, Some(0));
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "hello");
    }
}
