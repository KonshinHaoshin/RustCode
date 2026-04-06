use super::{
    state::{
        ChatWorkerResult, DisplayMessage, DisplayRole, FallbackField, OnboardingStep,
        PendingChatRequest, PrimaryField, TerminalState, ViewMode,
    },
    theme::{TerminalTheme, BLACK_CIRCLE, GUTTER},
};
use crate::{
    config::{ApiProtocol, ApiProvider, FallbackTarget, Settings},
    onboarding::OnboardingDraft,
    runtime::{ApprovalAction, QueryEngine, QueryTurnResult, RuntimeMessage, TurnStatus},
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Terminal,
};
use std::{
    io::{stdout, Stdout},
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

pub struct TerminalApp {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    state: TerminalState,
    theme: TerminalTheme,
}

impl TerminalApp {
    pub fn new(settings: Settings, initial_prompt: Option<String>) -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            state: TerminalState::new(settings, initial_prompt),
            theme: TerminalTheme::default(),
        })
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        let mut needs_redraw = true;

        loop {
            let mut state_changed = false;
            state_changed |= self.poll_pending_response();
            state_changed |= self.state.tick_spinner();
            state_changed |= self.clear_exit_confirmation_if_stale();

            if let Some(prompt) = self.state.consume_initial_prompt() {
                self.state.input = prompt;
                state_changed |= self.submit_prompt();
            }

            if state_changed {
                needs_redraw = true;
            }

            if needs_redraw {
                self.draw()?;
                needs_redraw = false;
            }

            if self.state.should_quit {
                break;
            }

            let timeout = self
                .state
                .time_until_next_spinner_frame()
                .unwrap_or(Duration::from_secs(60));

            if event::poll(timeout)? {
                if self.handle_event(event::read()?)? {
                    needs_redraw = true;
                }
            }
        }

        Ok(())
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        let theme = self.theme;
        let state = &mut self.state;

        self.terminal.draw(|frame| match state.view {
            ViewMode::Onboarding => draw_onboarding_view(frame, theme, state),
            ViewMode::Chat => draw_chat_view(frame, theme, state),
        })?;

        Ok(())
    }

    fn clear_exit_confirmation_if_stale(&mut self) -> bool {
        let had_deadline = self.state.confirm_exit_deadline.is_some();
        self.state.clear_exit_confirmation_if_stale();
        had_deadline && self.state.confirm_exit_deadline.is_none()
    }

    fn handle_event(&mut self, event: Event) -> anyhow::Result<bool> {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return Ok(false);
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    self.state.request_exit();
                    return Ok(true);
                }
                match self.state.view {
                    ViewMode::Chat => Ok(self.handle_chat_key(key)),
                    ViewMode::Onboarding => self.handle_onboarding_key(key),
                }
            }
            Event::Mouse(mouse) => {
                if self.state.view == ViewMode::Chat {
                    let old_offset = self.state.scroll_offset;
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            self.state.scroll_offset = self.state.scroll_offset.saturating_sub(3);
                        }
                        MouseEventKind::ScrollUp => {
                            self.state.scroll_offset = self.state.scroll_offset.saturating_add(3);
                        }
                        _ => {}
                    }
                    return Ok(self.state.scroll_offset != old_offset);
                }
                Ok(false)
            }
            Event::Resize(_, _) => Ok(true),
            _ => Ok(false),
        }
    }

    fn handle_chat_key(&mut self, key: KeyEvent) -> bool {
        if self.state.pending_approval.is_some() {
            return self.handle_approval_key(key);
        }

        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.state.input.push('\n');
                    true
                } else {
                    self.submit_prompt()
                }
            }
            KeyCode::Backspace => {
                let had_input = !self.state.input.is_empty();
                self.state.input.pop();
                had_input
            }
            KeyCode::Tab => {
                self.state.view = ViewMode::Onboarding;
                self.state.onboarding_step = OnboardingStep::Summary;
                self.state.status = "Opened configuration summary.".to_string();
                true
            }
            KeyCode::Esc => {
                let had_input = !self.state.input.is_empty();
                self.state.input.clear();
                had_input
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.input.push(ch);
                true
            }
            _ => false,
        }
    }

    fn handle_approval_key(&mut self, key: KeyEvent) -> bool {
        let Some(view_model) = self.state.pending_approval.as_mut() else {
            return false;
        };

        match key.code {
            KeyCode::Left => {
                view_model.focus_index = view_model.focus_index.saturating_sub(1);
                true
            }
            KeyCode::Right | KeyCode::Tab => {
                view_model.focus_index = (view_model.focus_index + 1) % 4;
                true
            }
            KeyCode::Esc => {
                view_model.focus_index = 0;
                true
            }
            KeyCode::Enter => {
                let selection = match view_model.focus_index {
                    1 => ApprovalSelection::DenyOnce,
                    2 => ApprovalSelection::AlwaysAllow,
                    3 => ApprovalSelection::AlwaysDeny,
                    _ => ApprovalSelection::AllowOnce,
                };
                self.resume_pending_approval(selection)
            }
            KeyCode::Char('a') => self.resume_pending_approval(ApprovalSelection::AllowOnce),
            KeyCode::Char('d') => self.resume_pending_approval(ApprovalSelection::DenyOnce),
            KeyCode::Char('A') => self.resume_pending_approval(ApprovalSelection::AlwaysAllow),
            KeyCode::Char('D') => self.resume_pending_approval(ApprovalSelection::AlwaysDeny),
            _ => false,
        }
    }

    fn handle_onboarding_key(&mut self, key: KeyEvent) -> anyhow::Result<bool> {
        let before = OnboardingSnapshot::capture(&self.state);

        match self.state.onboarding_step {
            OnboardingStep::Welcome => match key.code {
                KeyCode::Enter | KeyCode::Tab | KeyCode::Right => {
                    self.state.onboarding_step = OnboardingStep::Primary;
                    self.state.onboarding_focus = 0;
                }
                KeyCode::Esc if !self.state.settings.should_run_onboarding() => {
                    self.state.view = ViewMode::Chat;
                }
                _ => {}
            },
            OnboardingStep::Primary => self.handle_primary_key(key),
            OnboardingStep::FallbackList => self.handle_fallback_list_key(key),
            OnboardingStep::FallbackEdit => self.handle_fallback_edit_key(key),
            OnboardingStep::Summary => match key.code {
                KeyCode::Enter => self.state.complete_onboarding()?,
                KeyCode::Left | KeyCode::Esc => {
                    self.state.onboarding_step = OnboardingStep::FallbackList
                }
                _ => {}
            },
        }

        Ok(before != OnboardingSnapshot::capture(&self.state))
    }

    fn handle_primary_key(&mut self, key: KeyEvent) {
        let fields = self.state.active_primary_fields();
        match key.code {
            KeyCode::Up => {
                self.state.onboarding_focus = self.state.onboarding_focus.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Tab => {
                self.state.onboarding_focus =
                    (self.state.onboarding_focus + 1).min(fields.len().saturating_sub(1));
            }
            KeyCode::Left => self.adjust_primary(false),
            KeyCode::Right => self.adjust_primary(true),
            KeyCode::Enter => {
                if fields.get(self.state.onboarding_focus) == Some(&PrimaryField::Continue) {
                    self.state.onboarding_step = OnboardingStep::FallbackList;
                } else {
                    self.adjust_primary(true);
                }
            }
            KeyCode::Backspace => self.edit_primary(None),
            KeyCode::Esc => self.state.onboarding_step = OnboardingStep::Welcome,
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_primary(Some(ch))
            }
            _ => {}
        }
    }

    fn handle_fallback_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.state.selected_fallback = self.state.selected_fallback.saturating_sub(1)
            }
            KeyCode::Down => {
                if self.state.selected_fallback + 1 < self.state.draft.fallback_chain.len() {
                    self.state.selected_fallback += 1;
                }
            }
            KeyCode::Char(' ') => {
                let enabled = !self.state.draft.fallback_enabled;
                self.state.draft.set_fallback_enabled(enabled);
                self.state.selected_fallback = 0;
            }
            KeyCode::Char('a') => {
                let index = self.state.draft.add_fallback_target(ApiProvider::OpenAI);
                self.state.selected_fallback = index;
                self.state.editing_fallback = Some(index);
                self.state.onboarding_focus = 0;
                self.state.onboarding_step = OnboardingStep::FallbackEdit;
            }
            KeyCode::Char('d') => {
                if self.state.selected_fallback < self.state.draft.fallback_chain.len() {
                    self.state
                        .draft
                        .fallback_chain
                        .remove(self.state.selected_fallback);
                    if self.state.selected_fallback > 0 {
                        self.state.selected_fallback -= 1;
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if self.state.selected_fallback < self.state.draft.fallback_chain.len() {
                    self.state.editing_fallback = Some(self.state.selected_fallback);
                    self.state.onboarding_focus = 0;
                    self.state.onboarding_step = OnboardingStep::FallbackEdit;
                } else {
                    self.state.onboarding_step = OnboardingStep::Summary;
                }
            }
            KeyCode::Right | KeyCode::Tab => self.state.onboarding_step = OnboardingStep::Summary,
            KeyCode::Left | KeyCode::Esc => self.state.onboarding_step = OnboardingStep::Primary,
            _ => {}
        }
    }

    fn handle_fallback_edit_key(&mut self, key: KeyEvent) {
        let Some(target) = self.state.current_fallback().cloned() else {
            self.state.onboarding_step = OnboardingStep::FallbackList;
            self.state.editing_fallback = None;
            return;
        };
        let fields = self.state.active_fallback_fields(&target);

        match key.code {
            KeyCode::Up => {
                self.state.onboarding_focus = self.state.onboarding_focus.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Tab => {
                self.state.onboarding_focus =
                    (self.state.onboarding_focus + 1).min(fields.len().saturating_sub(1));
            }
            KeyCode::Left => self.adjust_fallback(false),
            KeyCode::Right => self.adjust_fallback(true),
            KeyCode::Enter => {
                if fields.get(self.state.onboarding_focus) == Some(&FallbackField::Save) {
                    self.state.editing_fallback = None;
                    self.state.onboarding_step = OnboardingStep::FallbackList;
                } else {
                    self.adjust_fallback(true);
                }
            }
            KeyCode::Backspace => self.edit_fallback(None),
            KeyCode::Esc => {
                self.state.editing_fallback = None;
                self.state.onboarding_step = OnboardingStep::FallbackList;
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_fallback(Some(ch))
            }
            _ => {}
        }
    }

    fn adjust_primary(&mut self, forward: bool) {
        let field = self
            .state
            .active_primary_fields()
            .get(self.state.onboarding_focus)
            .copied();
        match field {
            Some(PrimaryField::Provider) => {
                self.state.draft.prepare_for_provider_change(cycle_provider(
                    self.state.draft.provider,
                    forward,
                ));
            }
            Some(PrimaryField::Protocol) => self
                .state
                .draft
                .set_protocol(toggle_protocol(self.state.draft.protocol)),
            Some(PrimaryField::Continue) => {
                self.state.onboarding_step = OnboardingStep::FallbackList
            }
            _ => {}
        }
    }

    fn adjust_fallback(&mut self, forward: bool) {
        let Some(index) = self.state.editing_fallback else {
            return;
        };
        let Some(target) = self.state.draft.fallback_chain.get(index).cloned() else {
            return;
        };
        let field = self
            .state
            .active_fallback_fields(&target)
            .get(self.state.onboarding_focus)
            .copied();
        let Some(target) = self.state.draft.fallback_chain.get_mut(index) else {
            return;
        };
        match field {
            Some(FallbackField::Provider) => {
                *target = fallback_defaults(cycle_provider(target.provider, forward))
            }
            Some(FallbackField::Protocol) => {
                target.protocol = Some(toggle_protocol(
                    target.protocol.unwrap_or(ApiProtocol::OpenAi),
                ));
            }
            Some(FallbackField::Save) => {
                self.state.editing_fallback = None;
                self.state.onboarding_step = OnboardingStep::FallbackList;
            }
            _ => {}
        }
    }

    fn edit_primary(&mut self, ch: Option<char>) {
        let field = self
            .state
            .active_primary_fields()
            .get(self.state.onboarding_focus)
            .copied();
        match field {
            Some(PrimaryField::CustomName) => {
                edit_optional(&mut self.state.draft.custom_provider_name, ch)
            }
            Some(PrimaryField::BaseUrl) => edit_required(&mut self.state.draft.base_url, ch),
            Some(PrimaryField::Model) => edit_required(&mut self.state.draft.model, ch),
            Some(PrimaryField::ApiKey) => edit_optional(&mut self.state.draft.api_key, ch),
            _ => {}
        }
    }

    fn edit_fallback(&mut self, ch: Option<char>) {
        let Some(index) = self.state.editing_fallback else {
            return;
        };
        let Some(target_view) = self.state.draft.fallback_chain.get(index).cloned() else {
            return;
        };
        let field = self
            .state
            .active_fallback_fields(&target_view)
            .get(self.state.onboarding_focus)
            .copied();
        let Some(target) = self.state.draft.fallback_chain.get_mut(index) else {
            return;
        };
        match field {
            Some(FallbackField::CustomName) => edit_optional(&mut target.custom_provider_name, ch),
            Some(FallbackField::BaseUrl) => edit_optional(&mut target.base_url, ch),
            Some(FallbackField::Model) => edit_required(&mut target.model, ch),
            Some(FallbackField::ApiKey) => edit_optional(&mut target.api_key, ch),
            _ => {}
        }
    }

    fn submit_prompt(&mut self) -> bool {
        if self.state.thinking || self.state.pending_approval.is_some() {
            return false;
        }
        let prompt = self.state.input.trim().to_string();
        if prompt.is_empty() {
            return false;
        }

        let message = DisplayMessage {
            role: DisplayRole::User,
            content: prompt.clone(),
        };
        self.state.messages.push(message);
        self.state.input.clear();
        self.state.scroll_offset = 0;
        self.state.thinking = true;
        self.state.spinner_tick = 0;
        self.state.last_tick = std::time::Instant::now();
        self.state.status = format!(
            "Querying {}/{}",
            self.state.settings.api.provider_label(),
            self.state.settings.model
        );
        self.state.mark_chat_render_dirty();

        let user_message = RuntimeMessage::user(prompt);
        let base_history = Arc::clone(&self.state.conversation_history);
        self.state.pending_response = Some(PendingChatRequest {
            receiver: spawn_chat_request(
                self.state.settings.clone(),
                Arc::clone(&base_history),
                user_message.clone(),
            ),
            base_history,
            user_message: Some(user_message),
        });

        true
    }

    fn poll_pending_response(&mut self) -> bool {
        let Some(pending) = self.state.pending_response.take() else {
            return false;
        };
        match pending.receiver.try_recv() {
            Ok(ChatWorkerResult { turn }) => {
                self.state.thinking = false;
                match turn {
                    Ok(turn) => self.apply_turn_result(turn),
                    Err(error) => {
                        let mut history = (*pending.base_history).clone();
                        if let Some(user_message) = pending.user_message {
                            history.push(user_message);
                        }
                        self.state.replace_history(history);
                        self.state.messages.push(DisplayMessage {
                            role: DisplayRole::System,
                            content: format!("Request failed: {}", error),
                        });
                        self.state.status = "Request failed.".to_string();
                    }
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.state.pending_response = Some(pending);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let mut restored_history = (*pending.base_history).clone();
                if let Some(user_message) = pending.user_message {
                    restored_history.push(user_message);
                }
                self.state.replace_history(restored_history);
                self.state.thinking = false;
                self.state.status = "Request worker disconnected.".to_string();
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: "Request worker disconnected.".to_string(),
                });
                self.state.mark_chat_render_dirty();
                true
            }
        }
    }

    fn apply_turn_result(&mut self, turn: QueryTurnResult) {
        self.state.replace_history(turn.history);
        self.state.scroll_offset = 0;

        match turn.status {
            TurnStatus::Completed => {
                self.state.set_pending_approval(None);
                self.state.status = "Response received.".to_string();
            }
            TurnStatus::AwaitingApproval => {
                self.state.set_pending_approval(turn.pending_approval);
                self.state.status = "Tool approval required.".to_string();
            }
        }

        if let Err(error) = self.state.persist_current_session() {
            self.state.messages.push(DisplayMessage {
                role: DisplayRole::System,
                content: format!("Session save failed: {}", error),
            });
            self.state.status = "Session save failed.".to_string();
        }
    }

    fn resume_pending_approval(&mut self, selection: ApprovalSelection) -> bool {
        if self.state.thinking {
            return false;
        }
        let Some(view_model) = self.state.pending_approval.clone() else {
            return false;
        };

        match selection {
            ApprovalSelection::AlwaysAllow => update_local_permission_rules(
                &mut self.state.settings.permissions.allow_tools,
                &mut self.state.settings.permissions.deny_tools,
                &mut self.state.settings.permissions.ask_tools,
                &view_model.pending.tool_call.name,
            ),
            ApprovalSelection::AlwaysDeny => update_local_permission_rules(
                &mut self.state.settings.permissions.deny_tools,
                &mut self.state.settings.permissions.allow_tools,
                &mut self.state.settings.permissions.ask_tools,
                &view_model.pending.tool_call.name,
            ),
            ApprovalSelection::AllowOnce | ApprovalSelection::DenyOnce => {}
        }

        let action = match selection {
            ApprovalSelection::AllowOnce => ApprovalAction::AllowOnce(view_model.pending),
            ApprovalSelection::DenyOnce => ApprovalAction::DenyOnce(view_model.pending),
            ApprovalSelection::AlwaysAllow => ApprovalAction::AlwaysAllow(view_model.pending),
            ApprovalSelection::AlwaysDeny => ApprovalAction::AlwaysDeny(view_model.pending),
        };

        self.state.thinking = true;
        self.state.status = "Resuming after approval…".to_string();
        self.state.set_pending_approval(None);
        self.state.spinner_tick = 0;
        self.state.last_tick = std::time::Instant::now();
        let base_history = Arc::clone(&self.state.conversation_history);
        self.state.pending_response = Some(PendingChatRequest {
            receiver: spawn_approval_request(
                self.state.settings.clone(),
                Arc::clone(&base_history),
                action,
            ),
            base_history,
            user_message: None,
        });
        true
    }
}

#[derive(Clone, Copy)]
enum ApprovalSelection {
    AllowOnce,
    DenyOnce,
    AlwaysAllow,
    AlwaysDeny,
}

impl Drop for TerminalApp {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

#[derive(PartialEq, Eq)]
struct OnboardingSnapshot {
    view: ViewMode,
    onboarding_step: OnboardingStep,
    onboarding_focus: usize,
    selected_fallback: usize,
    editing_fallback: Option<usize>,
    fallback_enabled: bool,
    fallback_len: usize,
    status: String,
    draft: OnboardingDraft,
}

impl OnboardingSnapshot {
    fn capture(state: &TerminalState) -> Self {
        Self {
            view: state.view,
            onboarding_step: state.onboarding_step,
            onboarding_focus: state.onboarding_focus,
            selected_fallback: state.selected_fallback,
            editing_fallback: state.editing_fallback,
            fallback_enabled: state.draft.fallback_enabled,
            fallback_len: state.draft.fallback_chain.len(),
            status: state.status.clone(),
            draft: state.draft.clone(),
        }
    }
}

fn draw_onboarding_view(
    frame: &mut ratatui::Frame<'_>,
    theme: TerminalTheme,
    state: &TerminalState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(frame.size());

    frame.render_widget(
        Paragraph::new(theme.welcome_lines(chunks[0].width, &state.working_dir))
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[0],
    );
    render_onboarding(frame, chunks[1], theme, state);
    render_status_line(frame, chunks[2], theme, state);
}

fn draw_chat_view(frame: &mut ratatui::Frame<'_>, theme: TerminalTheme, state: &mut TerminalState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(frame.size());

    render_chat(frame, chunks[0], theme, state);
    render_prompt(frame, chunks[1], theme, state);
    render_status_line(frame, chunks[2], theme, state);
}

fn render_onboarding(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    theme: TerminalTheme,
    state: &TerminalState,
) {
    let lines = match state.onboarding_step {
        OnboardingStep::Welcome => vec![
            Line::from(Span::styled(
                "First-run onboarding",
                Style::default()
                    .fg(theme.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from("Configure a primary model and optional fallback chain."),
            Line::from("Press Enter to continue."),
        ],
        OnboardingStep::Primary => state
            .active_primary_fields()
            .into_iter()
            .enumerate()
            .map(|(index, field)| {
                render_primary_line(state, field, index == state.onboarding_focus, theme)
            })
            .collect(),
        OnboardingStep::FallbackList => render_fallback_list(state, theme),
        OnboardingStep::FallbackEdit => state
            .current_fallback()
            .map(|target| {
                state
                    .active_fallback_fields(target)
                    .into_iter()
                    .enumerate()
                    .map(|(index, field)| {
                        render_fallback_line(target, field, index == state.onboarding_focus, theme)
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![Line::from("No fallback selected.")]),
        OnboardingStep::Summary => state
            .draft
            .summary_lines()
            .into_iter()
            .map(|line| Line::from(Span::styled(line, Style::default().fg(theme.text))))
            .chain(std::iter::once(Line::default()))
            .chain(std::iter::once(Line::from(Span::styled(
                "Press Enter to save and open chat.",
                Style::default()
                    .fg(theme.shimmer)
                    .add_modifier(Modifier::BOLD),
            ))))
            .collect(),
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme.prompt_block().title(" Onboarding "))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn render_chat(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    theme: TerminalTheme,
    state: &mut TerminalState,
) {
    ensure_chat_cache(state, theme, area.width);
    let total_lines = state.chat_render_line_count;
    let visible = area.height;

    let max_scroll = total_lines.saturating_sub(visible);
    let scroll_up = state.scroll_offset as u16;
    let scroll_row = max_scroll.saturating_sub(scroll_up);

    let paragraph = std::mem::take(&mut state.chat_render_cache).scroll((scroll_row, 0));
    frame.render_widget(&paragraph, area);
    state.chat_render_cache = paragraph.scroll((0, 0));
}

fn ensure_chat_cache(state: &mut TerminalState, theme: TerminalTheme, width: u16) {
    if state.chat_render_dirty || state.chat_render_width != width {
        let lines = render_chat_lines(state, theme, width);
        let paragraph = Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false });
        state.chat_render_line_count = paragraph.line_count(width) as u16;
        state.chat_render_cache = paragraph;
        state.chat_render_width = width;
        state.chat_render_dirty = false;
    }
}

fn render_chat_lines(
    state: &TerminalState,
    theme: TerminalTheme,
    width: u16,
) -> Vec<Line<'static>> {
    if state.messages.is_empty() {
        return theme.empty_chat_lines(width, &state.working_dir);
    }

    let mut lines = Vec::new();

    for message in &state.messages {
        match message.role {
            DisplayRole::User => {
                for content_line in message.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!(" {}", content_line),
                        Style::default().fg(theme.text).bg(theme.user_msg_bg),
                    )));
                }
                if message.content.is_empty() {
                    lines.push(Line::from(Span::styled(
                        " ",
                        Style::default().bg(theme.user_msg_bg),
                    )));
                }
            }
            DisplayRole::Assistant => {
                for content_line in message.content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{} ", GUTTER), Style::default().fg(theme.subtle)),
                        Span::styled(content_line.to_string(), Style::default().fg(theme.text)),
                    ]));
                }
                if message.content.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{} ", GUTTER),
                        Style::default().fg(theme.subtle),
                    )));
                }
            }
            DisplayRole::System => {
                for content_line in message.content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{} ", BLACK_CIRCLE),
                            Style::default().fg(theme.error),
                        ),
                        Span::styled(content_line.to_string(), Style::default().fg(theme.error)),
                    ]));
                }
            }
            DisplayRole::Tool => {
                for content_line in message.content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{} ", GUTTER), Style::default().fg(theme.brand)),
                        Span::styled(content_line.to_string(), Style::default().fg(theme.muted)),
                    ]));
                }
            }
        }
        lines.push(Line::default());
    }

    if let Some(approval) = &state.pending_approval {
        lines.push(Line::from(Span::styled(
            "Approval required",
            Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!(
            "Tool: {}",
            approval.pending.tool_call.name
        )));
        lines.push(Line::from(format!("Reason: {}", approval.pending.reason)));
        for preview_line in approval.arguments_preview.lines() {
            lines.push(Line::from(format!("Args: {}", preview_line)));
        }
        lines.push(Line::from(render_approval_buttons(
            approval.focus_index,
            theme,
        )));
        lines.push(Line::default());
    }

    lines
}

fn render_prompt(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    theme: TerminalTheme,
    state: &TerminalState,
) {
    let mut lines: Vec<Line<'static>> = if state.pending_approval.is_some() {
        vec![
            Line::from(Span::styled(
                "Approval pending. Use a/d/A/D or Enter to continue.",
                theme.muted_style(),
            )),
            Line::from(Span::styled(
                "Chat input is paused until this tool decision is handled.",
                theme.muted_style(),
            )),
        ]
    } else if state.input.is_empty() {
        vec![Line::from(Span::styled(
            "What do you want to do?",
            theme.muted_style(),
        ))]
    } else {
        state
            .input
            .lines()
            .map(|line| {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(theme.text),
                ))
            })
            .collect::<Vec<_>>()
    };

    if state.thinking {
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(
                state.spinner_char().to_string(),
                Style::default().fg(theme.brand),
            ),
            Span::styled(" thinking…", theme.muted_style()),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme.prompt_block())
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn render_approval_buttons(focus_index: usize, theme: TerminalTheme) -> Vec<Span<'static>> {
    const LABELS: [&str; 4] = ["Allow Once", "Deny Once", "Always Allow", "Always Deny"];
    let mut spans = Vec::new();
    for (index, label) in LABELS.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if index == focus_index {
            Style::default()
                .fg(theme.panel)
                .bg(theme.brand)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted)
        };
        spans.push(Span::styled(format!("[{}]", label), style));
    }
    spans
}

fn render_status_line(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    theme: TerminalTheme,
    state: &TerminalState,
) {
    let dot_color = if state.thinking {
        theme.brand
    } else {
        theme.success
    };

    let provider_info = format!(
        "{}/{}",
        state.settings.api.provider_label(),
        state.settings.model
    );

    let mut spans = vec![
        Span::styled(format!("{} ", BLACK_CIRCLE), Style::default().fg(dot_color)),
        Span::styled(provider_info, theme.muted_style()),
    ];

    if !state.thinking && !state.status.is_empty() {
        spans.push(Span::styled("  ∙  ", theme.muted_style()));
        spans.push(Span::styled(state.status.clone(), theme.muted_style()));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

fn render_primary_line(
    state: &TerminalState,
    field: PrimaryField,
    focused: bool,
    theme: TerminalTheme,
) -> Line<'static> {
    let (label, value) = match field {
        PrimaryField::Provider => ("Provider", state.draft.provider_label()),
        PrimaryField::Protocol => ("Protocol", state.draft.protocol.as_str().to_string()),
        PrimaryField::CustomName => (
            "Custom name",
            state.draft.custom_provider_name.clone().unwrap_or_default(),
        ),
        PrimaryField::BaseUrl => ("Base URL", state.draft.base_url.clone()),
        PrimaryField::Model => ("Model", state.draft.model.clone()),
        PrimaryField::ApiKey => ("API key", mask_secret(state.draft.api_key.as_deref())),
        PrimaryField::Continue => ("Continue", "Review fallback chain".to_string()),
    };
    render_field_line(label, value, focused, theme)
}

fn render_fallback_line(
    target: &FallbackTarget,
    field: FallbackField,
    focused: bool,
    theme: TerminalTheme,
) -> Line<'static> {
    let (label, value) = match field {
        FallbackField::Provider => ("Provider", target.provider.as_str().to_string()),
        FallbackField::Protocol => (
            "Protocol",
            target
                .protocol
                .unwrap_or(ApiProtocol::OpenAi)
                .as_str()
                .to_string(),
        ),
        FallbackField::CustomName => (
            "Custom name",
            target.custom_provider_name.clone().unwrap_or_default(),
        ),
        FallbackField::BaseUrl => (
            "Base URL",
            target
                .base_url
                .clone()
                .unwrap_or_else(|| target.provider.default_base_url().to_string()),
        ),
        FallbackField::Model => ("Model", target.model.clone()),
        FallbackField::ApiKey => ("API key", mask_secret(target.api_key.as_deref())),
        FallbackField::Save => ("Save", "Return to fallback list".to_string()),
    };
    render_field_line(label, value, focused, theme)
}

fn render_fallback_list(state: &TerminalState, theme: TerminalTheme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "Fallbacks: {}",
                if state.draft.fallback_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
            Style::default().fg(theme.text),
        )),
        Line::default(),
    ];

    if state.draft.fallback_chain.is_empty() {
        lines.push(Line::from(Span::styled(
            "No fallback targets configured.",
            Style::default().fg(theme.muted),
        )));
    } else {
        for (index, target) in state.draft.fallback_chain.iter().enumerate() {
            let selected = index == state.selected_fallback;
            if selected {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", BLACK_CIRCLE),
                        Style::default().fg(theme.brand),
                    ),
                    Span::styled(
                        OnboardingDraft::fallback_target_label(target),
                        Style::default().fg(theme.text),
                    ),
                ]));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {}", OnboardingDraft::fallback_target_label(target)),
                    Style::default().fg(theme.text),
                )));
            }
        }
    }

    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "[a] add  [e]/Enter edit  [d] delete  [space] toggle",
        Style::default().fg(theme.muted),
    )));
    lines
}

fn render_field_line(
    label: &str,
    value: String,
    focused: bool,
    theme: TerminalTheme,
) -> Line<'static> {
    if focused {
        Line::from(vec![
            Span::styled(
                format!("{} ", BLACK_CIRCLE),
                Style::default().fg(theme.brand),
            ),
            Span::styled(format!("{label}: "), Style::default().fg(theme.brand)),
            Span::styled(value, Style::default().fg(theme.text)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ", Style::default().fg(theme.muted)),
            Span::styled(format!("{label}: "), Style::default().fg(theme.muted)),
            Span::styled(value, Style::default().fg(theme.text)),
        ])
    }
}

fn mask_secret(value: Option<&str>) -> String {
    match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => {
            let tail: String = value
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            format!("********{}", tail)
        }
        None => "not set".to_string(),
    }
}

fn edit_required(target: &mut String, ch: Option<char>) {
    match ch {
        Some(ch) => target.push(ch),
        None => {
            target.pop();
        }
    }
}

fn edit_optional(target: &mut Option<String>, ch: Option<char>) {
    let value = target.get_or_insert_with(String::new);
    match ch {
        Some(ch) => value.push(ch),
        None => {
            value.pop();
            if value.is_empty() {
                *target = None;
            }
        }
    }
}

fn toggle_protocol(protocol: ApiProtocol) -> ApiProtocol {
    match protocol {
        ApiProtocol::OpenAi => ApiProtocol::Anthropic,
        ApiProtocol::Anthropic => ApiProtocol::OpenAi,
    }
}

fn cycle_provider(current: ApiProvider, forward: bool) -> ApiProvider {
    const PROVIDERS: [ApiProvider; 6] = [
        ApiProvider::DeepSeek,
        ApiProvider::OpenAI,
        ApiProvider::DashScope,
        ApiProvider::OpenRouter,
        ApiProvider::Ollama,
        ApiProvider::Custom,
    ];
    let index = PROVIDERS
        .iter()
        .position(|provider| *provider == current)
        .unwrap_or(0);
    if forward {
        PROVIDERS[(index + 1) % PROVIDERS.len()]
    } else if index == 0 {
        PROVIDERS[PROVIDERS.len() - 1]
    } else {
        PROVIDERS[index - 1]
    }
}

fn fallback_defaults(provider: ApiProvider) -> FallbackTarget {
    let mut target = FallbackTarget {
        provider,
        protocol: None,
        custom_provider_name: None,
        api_key: None,
        base_url: None,
        model: provider.default_model().to_string(),
    };
    if provider == ApiProvider::Custom {
        target.protocol = Some(ApiProtocol::OpenAi);
        target.custom_provider_name = Some("custom".to_string());
        target.base_url = Some(provider.default_base_url().to_string());
    }
    target
}

fn spawn_chat_request(
    settings: Settings,
    base_history: Arc<Vec<RuntimeMessage>>,
    user_message: RuntimeMessage,
) -> mpsc::Receiver<ChatWorkerResult> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let original_user_message = user_message.clone();
        let payload = (|| -> anyhow::Result<ChatWorkerResult> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let fallback_user_message = user_message.clone();
            let result = runtime.block_on(async {
                let engine = QueryEngine::new(settings);
                engine.submit_message(&base_history, user_message).await
            });

            match result {
                Ok(turn) => Ok(ChatWorkerResult { turn: Ok(turn) }),
                Err(error) => {
                    let _ = fallback_user_message;
                    Err(error)
                }
            }
        })()
        .unwrap_or_else(|error| {
            let _ = original_user_message;
            ChatWorkerResult { turn: Err(error) }
        });
        let _ = sender.send(payload);
    });
    receiver
}

fn spawn_approval_request(
    settings: Settings,
    base_history: Arc<Vec<RuntimeMessage>>,
    action: ApprovalAction,
) -> mpsc::Receiver<ChatWorkerResult> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let payload = (|| -> anyhow::Result<ChatWorkerResult> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(async {
                let engine = QueryEngine::new(settings);
                engine.resume_after_approval(&base_history, action).await
            });

            match result {
                Ok(turn) => Ok(ChatWorkerResult { turn: Ok(turn) }),
                Err(error) => Err(error),
            }
        })()
        .unwrap_or_else(|error| ChatWorkerResult { turn: Err(error) });
        let _ = sender.send(payload);
    });
    receiver
}

fn update_local_permission_rules(
    target_rules: &mut Vec<String>,
    remove_from: &mut Vec<String>,
    ask_rules: &mut Vec<String>,
    tool_name: &str,
) {
    remove_from.retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
    ask_rules.retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
    if !target_rules
        .iter()
        .any(|rule| rule.eq_ignore_ascii_case(tool_name))
    {
        target_rules.push(tool_name.to_string());
    }
}
