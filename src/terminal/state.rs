use crate::{
    config::{ApiProvider, FallbackTarget, Settings},
    onboarding::OnboardingDraft,
    runtime::{PendingApproval, QueryTurnResult, RuntimeMessage, RuntimeRole},
    session::{Session, SessionManager},
    terminal::theme::SPINNER_FRAMES,
};
use ratatui::{text::Line, widgets::Paragraph};
use std::{
    sync::{mpsc::Receiver, Arc},
    time::{Duration, Instant},
};

const SPINNER_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Debug, Clone)]
pub enum DisplayRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: DisplayRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct PendingApprovalViewModel {
    pub pending: PendingApproval,
    pub arguments_preview: String,
    pub focus_index: usize,
}

#[derive(Debug)]
pub struct ChatWorkerResult {
    pub turn: anyhow::Result<QueryTurnResult>,
}

#[derive(Debug)]
pub struct PendingChatRequest {
    pub receiver: Receiver<ChatWorkerResult>,
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
    pub thinking: bool,
    pub initial_prompt: Option<String>,
    pub spinner_tick: usize,
    pub last_tick: Instant,
    pub working_dir: String,
    pub scroll_offset: usize,
    pub chat_render_cache: Paragraph<'static>,
    pub chat_render_width: u16,
    pub chat_render_line_count: u16,
    pub chat_render_dirty: bool,
    pub session_manager: SessionManager,
    pub active_session: Option<Session>,
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
        let mut conversation_history = Arc::new(Vec::new());
        let mut messages = Vec::new();

        if view == ViewMode::Chat
            && settings.session.persist_transcript
            && settings.session.auto_restore_last_session
        {
            match session_manager.load_latest() {
                Ok(Some(session)) => {
                    conversation_history = Arc::new(session.runtime_history());
                    messages = Self::display_messages_from_history(&conversation_history);
                    restored_session_notice = Some(format!("Restored session {}", session.id));
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
            pending_approval: None,
            thinking: false,
            initial_prompt,
            spinner_tick: 0,
            last_tick: Instant::now(),
            working_dir,
            scroll_offset: 0,
            chat_render_cache: Paragraph::new(Vec::<Line<'static>>::new()),
            chat_render_width: 0,
            chat_render_line_count: 0,
            chat_render_dirty: true,
            session_manager,
            active_session,
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
        self.messages.push(DisplayMessage {
            role: DisplayRole::Assistant,
            content: format!(
                "Configured {}/{} with {} fallback target(s).",
                self.draft.provider_label(),
                self.draft.model,
                self.draft.fallback_chain.len()
            ),
        });
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
        self.refresh_display_messages();
    }

    pub fn set_pending_approval(&mut self, pending: Option<PendingApproval>) {
        self.pending_approval = pending.map(|pending| PendingApprovalViewModel {
            arguments_preview: format_arguments_preview(&pending.tool_call.arguments),
            pending,
            focus_index: 0,
        });
        self.mark_chat_render_dirty();
    }

    pub fn persist_current_session(&mut self) -> anyhow::Result<()> {
        if !self.settings.session.persist_transcript {
            return Ok(());
        }

        if self.active_session.is_none() {
            self.active_session = Some(self.session_manager.create(Some("tui-session"))?);
        }

        if let Some(session) = &mut self.active_session {
            self.session_manager
                .save_transcript(session, &self.conversation_history)?;
        }

        Ok(())
    }

    fn display_messages_from_history(history: &[RuntimeMessage]) -> Vec<DisplayMessage> {
        let mut messages = Vec::new();

        for message in history {
            match message.role {
                RuntimeRole::User => messages.push(DisplayMessage {
                    role: DisplayRole::User,
                    content: message.content.clone(),
                }),
                RuntimeRole::Assistant => {
                    if !message.content.trim().is_empty() {
                        messages.push(DisplayMessage {
                            role: DisplayRole::Assistant,
                            content: message.content.clone(),
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
                        });
                    }
                }
                RuntimeRole::System => messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: message.content.clone(),
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
                    });
                }
            }
        }

        messages
    }
}

fn format_arguments_preview(arguments: &serde_json::Value) -> String {
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

fn format_tool_body(content: &str) -> String {
    if content.trim().is_empty() {
        String::new()
    } else {
        format!("\n{}", content)
    }
}
