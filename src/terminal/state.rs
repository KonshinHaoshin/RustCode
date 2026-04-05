use crate::{
    config::{ApiProvider, FallbackTarget, Settings},
    onboarding::OnboardingDraft,
    runtime::RuntimeMessage,
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
}

#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: DisplayRole,
    pub content: String,
}

#[derive(Debug)]
pub struct ChatWorkerResult {
    pub history: Vec<RuntimeMessage>,
    pub result: anyhow::Result<String>,
}

#[derive(Debug)]
pub struct PendingChatRequest {
    pub receiver: Receiver<ChatWorkerResult>,
    pub base_history: Arc<Vec<RuntimeMessage>>,
    pub user_message: RuntimeMessage,
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
}

impl TerminalState {
    pub fn new(settings: Settings, initial_prompt: Option<String>) -> Self {
        let view = if settings.should_run_onboarding() {
            ViewMode::Onboarding
        } else {
            ViewMode::Chat
        };

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
            status: if view == ViewMode::Onboarding {
                "First run detected. Complete onboarding to start coding.".to_string()
            } else {
                "Ready.".to_string()
            },
            input: String::new(),
            should_quit: false,
            confirm_exit_deadline: None,
            messages: Vec::new(),
            conversation_history: Arc::new(Vec::new()),
            pending_response: None,
            thinking: false,
            initial_prompt,
            spinner_tick: 0,
            last_tick: Instant::now(),
            working_dir: std::env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "~".to_string()),
            scroll_offset: 0,
            chat_render_cache: Paragraph::new(Vec::<Line<'static>>::new()),
            chat_render_width: 0,
            chat_render_line_count: 0,
            chat_render_dirty: true,
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
}
