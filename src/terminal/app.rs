use super::{
    state::{
        ChatWorkerResult, DisplayMessage, DisplayRole, FallbackField, OnboardingStep,
        PendingChatRequest, PermissionSection, PermissionsViewState, PrimaryField,
        ResumePickerState, SelectionClickState, SelectionMode, SelectionPoint, TerminalState,
        ViewMode,
    },
    theme::{TerminalTheme, BLACK_CIRCLE, GUTTER},
};
use crate::{
    compact::CompactService,
    config::{
        add_project_local_permission_rule, load_project_local_settings,
        remove_project_local_permission_rule, ApiProtocol, ApiProvider, FallbackTarget,
        ProjectPermissionRuleKind, Settings,
    },
    input::{format_help_text, format_status_text, InputProcessor, LocalCommand, ProcessedInput},
    onboarding::OnboardingDraft,
    permissions::events::{PermissionEvent, PermissionEventStore},
    runtime::{ApprovalAction, QueryEngine, QueryTurnResult, RuntimeMessage, TurnStatus},
    session::SessionInfo,
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEventKind,
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
    io::{stdout, Stdout, Write},
    process::{Command, Stdio},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
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
                    if self.state.view == ViewMode::Chat && self.state.has_selection() {
                        match copy_text_to_clipboard(
                            self.state.selection_text().unwrap_or_default(),
                        ) {
                            Ok(()) => {
                                self.state.status = "Selection copied to clipboard.".to_string();
                            }
                            Err(error) => {
                                self.state.status = format!("Clipboard copy failed: {}", error);
                            }
                        }
                        return Ok(true);
                    }
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
                    if let Some(point) =
                        selection_point_for_mouse(&self.state, mouse.column, mouse.row)
                    {
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                let click_count = next_click_count(
                                    self.state.last_selection_click,
                                    point,
                                    Instant::now(),
                                );
                                self.state.last_selection_click = Some(SelectionClickState {
                                    point,
                                    count: click_count,
                                    at: Instant::now(),
                                });
                                let (anchor, focus, mode) =
                                    selection_seed_for_click(&self.state, point, click_count);
                                self.state.begin_selection(anchor, focus, mode);
                                return Ok(true);
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                if self.state.selection_dragging {
                                    let focus = expand_selection_focus(
                                        &self.state,
                                        self.state.selection_mode,
                                        point,
                                    );
                                    self.state.update_selection(focus);
                                    return Ok(true);
                                }
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                if self.state.selection_dragging {
                                    let focus = expand_selection_focus(
                                        &self.state,
                                        self.state.selection_mode,
                                        point,
                                    );
                                    self.state.finish_selection(focus);
                                    if self.state.has_selection() {
                                        match copy_text_to_clipboard(
                                            self.state.selection_text().unwrap_or_default(),
                                        ) {
                                            Ok(()) => {
                                                self.state.status =
                                                    "Selection copied to clipboard.".to_string();
                                            }
                                            Err(error) => {
                                                self.state.status =
                                                    format!("Clipboard copy failed: {}", error);
                                            }
                                        }
                                    } else {
                                        self.state.clear_selection();
                                    }
                                    return Ok(true);
                                }
                            }
                            _ => {}
                        }
                    }

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
        if self.state.resume_picker.is_some() {
            return self.handle_resume_picker_key(key);
        }

        if self.state.permissions_view.is_some() {
            return self.handle_permissions_view_key(key);
        }

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
                if self.state.has_selection() {
                    self.state.clear_selection();
                    self.state.mark_chat_render_dirty();
                    return true;
                }
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

    fn handle_resume_picker_key(&mut self, key: KeyEvent) -> bool {
        if self.state.resume_picker.is_none() {
            return false;
        }

        match key.code {
            KeyCode::Up => {
                if let Some(picker) = self.state.resume_picker.as_mut() {
                    picker.selected = picker.selected.saturating_sub(1);
                }
                true
            }
            KeyCode::Down => {
                if let Some(picker) = self.state.resume_picker.as_mut() {
                    if picker.selected + 1 < picker.sessions.len() {
                        picker.selected += 1;
                    }
                }
                true
            }
            KeyCode::Enter => {
                let session_id = self.state.resume_picker.as_ref().and_then(|picker| {
                    picker
                        .sessions
                        .get(picker.selected)
                        .map(|session| session.id.clone())
                });
                if let Some(session_id) = session_id {
                    return self.resume_session_by_id(&session_id);
                }
                false
            }
            KeyCode::Esc => {
                self.state.resume_picker = None;
                self.state.status = "Closed resume picker.".to_string();
                self.state.mark_chat_render_dirty();
                true
            }
            _ => false,
        }
    }

    fn handle_permissions_view_key(&mut self, key: KeyEvent) -> bool {
        if self.state.permissions_view.is_none() {
            return false;
        }

        match key.code {
            KeyCode::Left => {
                if let Some(view) = self.state.permissions_view.as_mut() {
                    view.section = match view.section {
                        PermissionSection::Allow => PermissionSection::Recent,
                        PermissionSection::Deny => PermissionSection::Allow,
                        PermissionSection::Ask => PermissionSection::Deny,
                        PermissionSection::Recent => PermissionSection::Ask,
                    };
                    view.selected = 0;
                }
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Right | KeyCode::Tab => {
                if let Some(view) = self.state.permissions_view.as_mut() {
                    view.section = match view.section {
                        PermissionSection::Allow => PermissionSection::Deny,
                        PermissionSection::Deny => PermissionSection::Ask,
                        PermissionSection::Ask => PermissionSection::Recent,
                        PermissionSection::Recent => PermissionSection::Allow,
                    };
                    view.selected = 0;
                }
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Up => {
                if let Some(view) = self.state.permissions_view.as_mut() {
                    view.selected = view.selected.saturating_sub(1);
                }
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Down => {
                if let Some(view) = self.state.permissions_view.as_mut() {
                    let max_index = permissions_item_count(view).saturating_sub(1);
                    view.selected = (view.selected + 1).min(max_index);
                }
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Char('d')
                if self
                    .state
                    .permissions_view
                    .as_ref()
                    .is_some_and(|view| view.section != PermissionSection::Recent) =>
            {
                self.remove_selected_local_rule()
            }
            KeyCode::Char('a')
                if self
                    .state
                    .permissions_view
                    .as_ref()
                    .is_some_and(|view| view.section == PermissionSection::Recent) =>
            {
                self.add_recent_event_rule(ProjectPermissionRuleKind::Allow)
            }
            KeyCode::Char('n')
                if self
                    .state
                    .permissions_view
                    .as_ref()
                    .is_some_and(|view| view.section == PermissionSection::Recent) =>
            {
                self.add_recent_event_rule(ProjectPermissionRuleKind::Deny)
            }
            KeyCode::Char('k')
                if self
                    .state
                    .permissions_view
                    .as_ref()
                    .is_some_and(|view| view.section == PermissionSection::Recent) =>
            {
                self.add_recent_event_rule(ProjectPermissionRuleKind::Ask)
            }
            KeyCode::Esc => {
                self.state.permissions_view = None;
                self.state.status = "Closed permissions view.".to_string();
                self.state.mark_chat_render_dirty();
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
        let prompt = self.state.input.trim().to_string();
        if prompt.is_empty() {
            return false;
        }

        match InputProcessor::new().process(&prompt) {
            ProcessedInput::LocalCommand(command) => {
                self.state.input.clear();
                self.execute_local_command(command)
            }
            ProcessedInput::Prompt(prompt) => {
                if self.state.thinking || self.state.pending_approval.is_some() {
                    return false;
                }
                self.submit_runtime_prompt(prompt);
                true
            }
        }
    }

    fn submit_runtime_prompt(&mut self, prompt: String) {
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
    }

    fn execute_local_command(&mut self, command: LocalCommand) -> bool {
        match command {
            LocalCommand::Help => {
                self.push_system_message(format_help_text());
                self.state.status = "Displayed slash command help.".to_string();
                true
            }
            LocalCommand::Clear => {
                self.state.reset_conversation();
                self.state.status = "Cleared conversation and reset active session.".to_string();
                self.state.mark_chat_render_dirty();
                true
            }
            LocalCommand::Compact { instructions } => self.compact_current_history(instructions),
            LocalCommand::Permissions => self.open_permissions_view(),
            LocalCommand::Model { model } => {
                let detail = if let Some(model) = model {
                    self.state.settings.model = model.clone();
                    format!(
                        "Active model changed to {}/{}.",
                        self.state.settings.api.provider_label(),
                        model
                    )
                } else {
                    format!(
                        "Active model: {}/{}.",
                        self.state.settings.api.provider_label(),
                        self.state.settings.model
                    )
                };
                self.push_system_message(detail.clone());
                self.state.status = detail;
                true
            }
            LocalCommand::Status => {
                self.push_system_message(format_status_text(
                    &self.state.settings,
                    self.state.active_session_id.as_deref(),
                    self.state.conversation_history.len(),
                    self.state.pending_approval.is_some(),
                    self.state.last_usage_total,
                ));
                self.state.status = "Displayed runtime status.".to_string();
                true
            }
            LocalCommand::Resume { session_id } => match session_id {
                Some(session_id) => self.resume_session_by_id(&session_id),
                None => self.open_resume_picker(),
            },
        }
    }

    fn compact_current_history(&mut self, instructions: Option<String>) -> bool {
        if self.state.thinking {
            return false;
        }
        if self.state.pending_approval.is_some() {
            self.push_system_message("Cannot compact while tool approval is pending.".to_string());
            self.state.status = "Compact blocked by pending approval.".to_string();
            return true;
        }

        let history = (*self.state.conversation_history).clone();
        let settings = self.state.settings.clone();
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                self.push_system_message(format!(
                    "Failed to initialize compact runtime: {}",
                    error
                ));
                self.state.status = "Compact failed.".to_string();
                return true;
            }
        };

        match runtime.block_on(async {
            CompactService::new(settings)
                .compact_history(&history, instructions.as_deref())
                .await
        }) {
            Ok(outcome) => {
                self.state.replace_history(outcome.history);
                self.state.status = "Conversation compacted.".to_string();
                if let Err(error) = self.state.persist_current_session() {
                    self.push_system_message(format!("Session save failed: {}", error));
                    self.state.status = "Session save failed.".to_string();
                }
                true
            }
            Err(error) => {
                self.push_system_message(format!("Compact failed: {}", error));
                self.state.status = "Compact failed.".to_string();
                true
            }
        }
    }

    fn push_system_message(&mut self, content: String) {
        self.state.messages.push(DisplayMessage {
            role: DisplayRole::System,
            content,
        });
        self.state.mark_chat_render_dirty();
    }

    fn open_resume_picker(&mut self) -> bool {
        match self.state.session_manager.list_recent() {
            Ok(mut sessions) => {
                if let Some(active_id) = &self.state.active_session_id {
                    sessions.retain(|session| session.id != *active_id);
                }
                if sessions.is_empty() {
                    self.state.status = "No resumable sessions found in this project.".to_string();
                } else {
                    self.state.resume_picker = Some(ResumePickerState {
                        sessions,
                        selected: 0,
                    });
                    self.state.permissions_view = None;
                    self.state.status = "Select a session to resume.".to_string();
                }
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Failed to list sessions: {}", error);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Failed to list sessions: {}", error),
                });
                self.state.mark_chat_render_dirty();
                true
            }
        }
    }

    fn resume_session_by_id(&mut self, session_id: &str) -> bool {
        match self.state.session_manager.load(session_id) {
            Ok(Some(session)) => {
                self.state.restore_session(session);
                true
            }
            Ok(None) => {
                self.state.status = format!("Session {} not found.", session_id);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Session {} not found.", session_id),
                });
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Session restore failed: {}", error);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Session restore failed: {}", error),
                });
                self.state.mark_chat_render_dirty();
                true
            }
        }
    }

    fn open_permissions_view(&mut self) -> bool {
        let global_settings = match Settings::load_global() {
            Ok(settings) => settings.permissions,
            Err(error) => {
                self.state.status = format!("Failed to load global permissions: {}", error);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Failed to load global permissions: {}", error),
                });
                self.state.mark_chat_render_dirty();
                return true;
            }
        };

        let local_permissions = match load_project_local_settings(None) {
            Ok(local) => local
                .and_then(|settings| settings.permissions)
                .unwrap_or_default(),
            Err(error) => {
                self.state.status = format!("Failed to load local permissions: {}", error);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Failed to load local permissions: {}", error),
                });
                self.state.mark_chat_render_dirty();
                return true;
            }
        };

        let recent_events = match PermissionEventStore::load(None) {
            Ok(mut events) => {
                events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                events
            }
            Err(error) => {
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Failed to load permission events: {}", error),
                });
                Vec::new()
            }
        };

        self.state.permissions_view = Some(PermissionsViewState {
            section: PermissionSection::Allow,
            selected: 0,
            global_permissions: global_settings,
            local_permissions,
            recent_events,
        });
        self.state.resume_picker = None;
        self.state.status = "Inspecting project-local permissions.".to_string();
        self.state.mark_chat_render_dirty();
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
        self.state.last_usage_total = turn.usage.as_ref().map(|usage| usage.total_tokens);
        self.state.replace_history(turn.history);
        self.state.scroll_offset = 0;

        match turn.status {
            TurnStatus::Completed => {
                self.state.set_pending_approval(None);
                self.state.status = if turn.was_compacted {
                    "Response received and conversation compacted.".to_string()
                } else {
                    "Response received.".to_string()
                };
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

    fn remove_selected_local_rule(&mut self) -> bool {
        let Some(view) = self.state.permissions_view.as_mut() else {
            return false;
        };
        let Some(tool_name) = selected_local_rule(view).map(str::to_string) else {
            self.state.status = "No local rule selected.".to_string();
            return true;
        };

        let kind = match view.section {
            PermissionSection::Allow => ProjectPermissionRuleKind::Allow,
            PermissionSection::Deny => ProjectPermissionRuleKind::Deny,
            PermissionSection::Ask => ProjectPermissionRuleKind::Ask,
            PermissionSection::Recent => return false,
        };

        match remove_project_local_permission_rule(None, kind, &tool_name) {
            Ok(local_permissions) => {
                view.local_permissions = local_permissions;
                let current_rules = selected_section_rules(view);
                if view.selected >= current_rules.len() && !current_rules.is_empty() {
                    view.selected = current_rules.len() - 1;
                } else if current_rules.is_empty() {
                    view.selected = 0;
                }
                refresh_runtime_permissions(&mut self.state.settings.permissions, view);
                self.state.status = format!("Removed local rule for {}.", tool_name);
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Failed to update local permissions: {}", error);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Failed to update local permissions: {}", error),
                });
                self.state.mark_chat_render_dirty();
                true
            }
        }
    }

    fn add_recent_event_rule(&mut self, kind: ProjectPermissionRuleKind) -> bool {
        let Some(view) = self.state.permissions_view.as_mut() else {
            return false;
        };
        let Some(event) = view.recent_events.get(view.selected).cloned() else {
            self.state.status = "No recent permission event selected.".to_string();
            return true;
        };

        match add_project_local_permission_rule(None, kind, &event.tool_name) {
            Ok(local_permissions) => {
                view.local_permissions = local_permissions;
                refresh_runtime_permissions(&mut self.state.settings.permissions, view);
                view.section = match kind {
                    ProjectPermissionRuleKind::Allow => PermissionSection::Allow,
                    ProjectPermissionRuleKind::Deny => PermissionSection::Deny,
                    ProjectPermissionRuleKind::Ask => PermissionSection::Ask,
                };
                view.selected = 0;
                self.state.status = format!("Saved project-local rule for {}.", event.tool_name);
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Failed to save local permissions: {}", error);
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Failed to save local permissions: {}", error),
                });
                self.state.mark_chat_render_dirty();
                true
            }
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

        if matches!(
            selection,
            ApprovalSelection::DenyOnce | ApprovalSelection::AlwaysDeny
        ) {
            let _ = PermissionEventStore::append(
                None,
                PermissionEvent {
                    tool_name: view_model.pending.tool_call.name.clone(),
                    decision: match selection {
                        ApprovalSelection::AlwaysDeny => "always_deny".to_string(),
                        _ => "deny_once".to_string(),
                    },
                    reason: view_model.pending.reason.clone(),
                    timestamp: chrono::Utc::now(),
                },
            );
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
        if let Err(error) = self.state.persist_current_session() {
            self.state.messages.push(DisplayMessage {
                role: DisplayRole::System,
                content: format!("Session save failed: {}", error),
            });
        }
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
    state.chat_area = area;
    state.chat_scroll_row = scroll_row as usize;

    let paragraph = std::mem::take(&mut state.chat_render_cache).scroll((scroll_row, 0));
    frame.render_widget(&paragraph, area);
    state.chat_render_cache = paragraph.scroll((0, 0));
}

fn ensure_chat_cache(state: &mut TerminalState, theme: TerminalTheme, width: u16) {
    if state.chat_render_dirty || state.chat_render_width != width {
        let rendered_lines = render_chat_lines(state, theme, width);
        state.chat_plain_lines = rendered_lines
            .iter()
            .map(|line| line.text.clone())
            .collect();
        let lines = rendered_lines
            .iter()
            .enumerate()
            .map(|(index, line)| line.to_line(selection_range_for_line(state, index), theme))
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(lines).alignment(Alignment::Left);
        state.chat_render_line_count = rendered_lines.len() as u16;
        state.chat_render_cache = paragraph;
        state.chat_render_width = width;
        state.chat_render_dirty = false;
    }
}

#[derive(Clone)]
struct ChatRenderLine {
    text: String,
    style: Style,
}

impl ChatRenderLine {
    fn to_line(&self, selection: Option<(usize, usize)>, theme: TerminalTheme) -> Line<'static> {
        if let Some((start, end)) = selection {
            let chars = self.text.chars().collect::<Vec<_>>();
            if chars.is_empty() || start >= chars.len() {
                return Line::from(Span::styled(self.text.clone(), self.style));
            }

            let end = end.min(chars.len().saturating_sub(1));
            let before = chars[..start].iter().collect::<String>();
            let selected = chars[start..=end].iter().collect::<String>();
            let after = if end + 1 < chars.len() {
                chars[end + 1..].iter().collect::<String>()
            } else {
                String::new()
            };

            let mut spans = Vec::new();
            if !before.is_empty() {
                spans.push(Span::styled(before, self.style));
            }
            spans.push(Span::styled(
                selected,
                self.style
                    .bg(theme.shimmer)
                    .fg(ratatui::style::Color::Black)
                    .add_modifier(Modifier::BOLD),
            ));
            if !after.is_empty() {
                spans.push(Span::styled(after, self.style));
            }
            Line::from(spans)
        } else {
            Line::from(Span::styled(self.text.clone(), self.style))
        }
    }
}

fn render_chat_lines(
    state: &TerminalState,
    theme: TerminalTheme,
    width: u16,
) -> Vec<ChatRenderLine> {
    if state.messages.is_empty() {
        return theme
            .empty_chat_lines(width, &state.working_dir)
            .into_iter()
            .map(|line| ChatRenderLine {
                text: line_to_plain_text(&line),
                style: Style::default().fg(theme.text),
            })
            .collect();
    }

    let mut lines = Vec::new();

    for message in &state.messages {
        match message.role {
            DisplayRole::User => {
                for content_line in message.content.lines() {
                    push_wrapped_line(
                        &mut lines,
                        format!(" {}", content_line),
                        Style::default().fg(theme.text).bg(theme.user_msg_bg),
                        width,
                    );
                }
                if message.content.is_empty() {
                    push_wrapped_line(
                        &mut lines,
                        " ".to_string(),
                        Style::default().bg(theme.user_msg_bg),
                        width,
                    );
                }
            }
            DisplayRole::Assistant => {
                for content_line in message.content.lines() {
                    push_wrapped_line(
                        &mut lines,
                        format!("{} {}", GUTTER, content_line),
                        Style::default().fg(theme.text),
                        width,
                    );
                }
                if message.content.is_empty() {
                    push_wrapped_line(
                        &mut lines,
                        format!("{} ", GUTTER),
                        Style::default().fg(theme.subtle),
                        width,
                    );
                }
            }
            DisplayRole::System => {
                for content_line in message.content.lines() {
                    push_wrapped_line(
                        &mut lines,
                        format!("{} {}", BLACK_CIRCLE, content_line),
                        Style::default().fg(theme.error),
                        width,
                    );
                }
            }
            DisplayRole::Tool => {
                for content_line in message.content.lines() {
                    push_wrapped_line(
                        &mut lines,
                        format!("{} {}", GUTTER, content_line),
                        Style::default().fg(theme.muted),
                        width,
                    );
                }
            }
        }
        lines.push(ChatRenderLine {
            text: String::new(),
            style: Style::default(),
        });
    }

    if let Some(approval) = &state.pending_approval {
        push_wrapped_line(
            &mut lines,
            "Approval required".to_string(),
            Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD),
            width,
        );
        if matches!(
            approval.origin,
            super::state::PendingApprovalOrigin::RestoredSession
        ) {
            push_wrapped_line(
                &mut lines,
                "Restored from previous session.".to_string(),
                theme.muted_style(),
                width,
            );
        }
        push_wrapped_line(
            &mut lines,
            format!("Tool: {}", approval.pending.tool_call.name),
            Style::default().fg(theme.text),
            width,
        );
        push_wrapped_line(
            &mut lines,
            format!("Reason: {}", approval.pending.reason),
            Style::default().fg(theme.text),
            width,
        );
        for preview_line in approval.arguments_preview.lines() {
            push_wrapped_line(
                &mut lines,
                format!("Args: {}", preview_line),
                Style::default().fg(theme.text),
                width,
            );
        }
        push_wrapped_line(
            &mut lines,
            line_to_plain_text(&Line::from(render_approval_buttons(
                approval.focus_index,
                theme,
            ))),
            Style::default().fg(theme.text),
            width,
        );
        lines.push(ChatRenderLine {
            text: String::new(),
            style: Style::default(),
        });
    }

    if let Some(picker) = &state.resume_picker {
        push_wrapped_line(
            &mut lines,
            "Resume session".to_string(),
            Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD),
            width,
        );
        push_wrapped_line(
            &mut lines,
            "Use Up/Down and Enter to restore. Esc closes.".to_string(),
            theme.muted_style(),
            width,
        );
        for (index, session) in picker.sessions.iter().enumerate() {
            push_wrapped_line(
                &mut lines,
                line_to_plain_text(&render_resume_session_line(
                    session,
                    index == picker.selected,
                    theme,
                )),
                Style::default().fg(if index == picker.selected {
                    theme.text
                } else {
                    theme.muted
                }),
                width,
            );
        }
        lines.push(ChatRenderLine {
            text: String::new(),
            style: Style::default(),
        });
    }

    if let Some(view) = &state.permissions_view {
        lines.extend(
            render_permissions_view_lines(view, theme)
                .into_iter()
                .map(|line| ChatRenderLine {
                    text: line_to_plain_text(&line),
                    style: Style::default().fg(theme.text),
                }),
        );
    }

    lines
}

fn push_wrapped_line(lines: &mut Vec<ChatRenderLine>, text: String, style: Style, width: u16) {
    for wrapped in wrap_plain_line(&text, width.max(1) as usize) {
        lines.push(ChatRenderLine {
            text: wrapped,
            style,
        });
    }
}

fn wrap_plain_line(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return vec![String::new()];
    }

    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

fn line_to_plain_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

fn selection_range_for_line(state: &TerminalState, line_index: usize) -> Option<(usize, usize)> {
    let selection = state.selection?;
    let (start, end) = selection.normalized();
    if line_index < start.line || line_index > end.line {
        return None;
    }

    let line_len = state
        .chat_plain_lines
        .get(line_index)
        .map(|line| line.chars().count())
        .unwrap_or_default();
    if line_len == 0 {
        return None;
    }

    let start_column = if line_index == start.line {
        start.column.min(line_len.saturating_sub(1))
    } else {
        0
    };
    let end_column = if line_index == end.line {
        end.column.min(line_len.saturating_sub(1))
    } else {
        line_len.saturating_sub(1)
    };

    (start_column <= end_column).then_some((start_column, end_column))
}

fn selection_point_for_mouse(
    state: &TerminalState,
    column: u16,
    row: u16,
) -> Option<SelectionPoint> {
    let area = state.chat_area;
    if column < area.x || column >= area.x.saturating_add(area.width) {
        return None;
    }
    if row < area.y || row >= area.y.saturating_add(area.height) {
        return None;
    }

    let local_row = row.saturating_sub(area.y) as usize;
    let line_index = state.chat_scroll_row + local_row;
    let line = state.chat_plain_lines.get(line_index)?;
    let line_len = line.chars().count();
    if line_len == 0 {
        return Some(SelectionPoint {
            line: line_index,
            column: 0,
        });
    }

    let local_col = column.saturating_sub(area.x) as usize;
    Some(SelectionPoint {
        line: line_index,
        column: local_col.min(line_len.saturating_sub(1)),
    })
}

fn next_click_count(
    previous: Option<SelectionClickState>,
    point: SelectionPoint,
    now: Instant,
) -> u8 {
    const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(450);

    if let Some(previous) = previous {
        if previous.point.line == point.line
            && previous.point.column.abs_diff(point.column) <= 1
            && now.duration_since(previous.at) <= MULTI_CLICK_WINDOW
        {
            return previous.count.saturating_add(1).min(3);
        }
    }

    1
}

fn selection_seed_for_click(
    state: &TerminalState,
    point: SelectionPoint,
    click_count: u8,
) -> (SelectionPoint, SelectionPoint, SelectionMode) {
    match click_count {
        2 => {
            let (start, end) = word_bounds_at(state, point);
            (
                SelectionPoint {
                    line: point.line,
                    column: start,
                },
                SelectionPoint {
                    line: point.line,
                    column: end,
                },
                SelectionMode::Word,
            )
        }
        3 => {
            let end = state
                .chat_plain_lines
                .get(point.line)
                .map(|line| line.chars().count().saturating_sub(1))
                .unwrap_or(0);
            (
                SelectionPoint {
                    line: point.line,
                    column: 0,
                },
                SelectionPoint {
                    line: point.line,
                    column: end,
                },
                SelectionMode::Line,
            )
        }
        _ => (point, point, SelectionMode::Char),
    }
}

fn expand_selection_focus(
    state: &TerminalState,
    mode: SelectionMode,
    point: SelectionPoint,
) -> SelectionPoint {
    match mode {
        SelectionMode::Char => point,
        SelectionMode::Word => {
            let (_, end) = word_bounds_at(state, point);
            SelectionPoint {
                line: point.line,
                column: end,
            }
        }
        SelectionMode::Line => SelectionPoint {
            line: point.line,
            column: state
                .chat_plain_lines
                .get(point.line)
                .map(|line| line.chars().count().saturating_sub(1))
                .unwrap_or(0),
        },
    }
}

fn word_bounds_at(state: &TerminalState, point: SelectionPoint) -> (usize, usize) {
    let Some(line) = state.chat_plain_lines.get(point.line) else {
        return (point.column, point.column);
    };
    let chars = line.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return (0, 0);
    }

    let index = point.column.min(chars.len().saturating_sub(1));
    if !is_word_char(chars[index]) {
        if let Some((word_start, word_end)) = nearest_word_bounds(&chars, index) {
            return (word_start, word_end);
        }
        return (index, index);
    }

    let mut start = index;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }

    let mut end = index;
    while end + 1 < chars.len() && is_word_char(chars[end + 1]) {
        end += 1;
    }

    (start, end)
}

fn nearest_word_bounds(chars: &[char], origin: usize) -> Option<(usize, usize)> {
    for distance in 1..chars.len() {
        if let Some(left) = origin
            .checked_sub(distance)
            .filter(|idx| is_word_char(chars[*idx]))
        {
            return Some(expand_word_bounds(chars, left));
        }
        let right = origin + distance;
        if right < chars.len() && is_word_char(chars[right]) {
            return Some(expand_word_bounds(chars, right));
        }
    }
    None
}

fn expand_word_bounds(chars: &[char], index: usize) -> (usize, usize) {
    let mut start = index;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index;
    while end + 1 < chars.len() && is_word_char(chars[end + 1]) {
        end += 1;
    }
    (start, end)
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '/' | '\\' | '.')
}

fn copy_text_to_clipboard(text: String) -> anyhow::Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    if cfg!(target_os = "windows") {
        let mut child = Command::new("clip")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(anyhow::anyhow!("clip exited with status {}", status));
        }
        return Ok(());
    }

    let osc52 = format!("\u{1b}]52;c;{}\u{7}", {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
    });
    stdout().write_all(osc52.as_bytes())?;
    stdout().flush()?;
    Ok(())
}

fn render_prompt(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    theme: TerminalTheme,
    state: &TerminalState,
) {
    let mut lines: Vec<Line<'static>> = if state.resume_picker.is_some() {
        vec![
            Line::from(Span::styled(
                "Resume picker open. Select a session to restore.",
                theme.muted_style(),
            )),
            Line::from(Span::styled(
                "Chat input is paused until the picker is closed.",
                theme.muted_style(),
            )),
        ]
    } else if state.permissions_view.is_some() {
        vec![
            Line::from(Span::styled(
                "Permissions view open. d removes local rule; a/n/k promote recent event.",
                theme.muted_style(),
            )),
            Line::from(Span::styled(
                "Use Left/Right to switch section. Esc closes.",
                theme.muted_style(),
            )),
        ]
    } else if state.pending_approval.is_some() {
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
    } else if state.has_selection() {
        vec![
            Line::from(Span::styled(
                "Selection copied. Drag to adjust; Ctrl+C copies again.",
                theme.muted_style(),
            )),
            Line::from(Span::styled(
                "Esc clears selection. Double-click selects word; triple-click selects line.",
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

fn render_resume_session_line(
    session: &SessionInfo,
    selected: bool,
    theme: TerminalTheme,
) -> Line<'static> {
    let marker = if selected { BLACK_CIRCLE } else { " " };
    let status = format!("{:?}", session.status).to_ascii_lowercase();
    Line::from(vec![
        Span::styled(
            format!("{} ", marker),
            Style::default().fg(if selected { theme.brand } else { theme.muted }),
        ),
        Span::styled(session.name.clone(), Style::default().fg(theme.text)),
        Span::styled("  ", theme.muted_style()),
        Span::styled(session.id.clone(), theme.muted_style()),
        Span::styled("  ", theme.muted_style()),
        Span::styled(status, Style::default().fg(theme.subtle)),
    ])
}

fn render_permissions_view_lines(
    view: &PermissionsViewState,
    theme: TerminalTheme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Permissions",
            Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(render_permission_tabs(view.section, theme)),
    ];

    match view.section {
        PermissionSection::Allow | PermissionSection::Deny | PermissionSection::Ask => {
            let rules = selected_section_rules(view);
            if rules.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No project-local rules in this section.",
                    theme.muted_style(),
                )));
            } else {
                for (index, rule) in rules.iter().enumerate() {
                    let selected = index == view.selected;
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{} ", if selected { BLACK_CIRCLE } else { " " }),
                            Style::default().fg(if selected { theme.brand } else { theme.muted }),
                        ),
                        Span::styled(rule.clone(), Style::default().fg(theme.text)),
                    ]));
                }
            }
            lines.push(Line::from(Span::styled(
                format!(
                    "Global {} rules: {}",
                    permission_section_name(view.section).to_ascii_lowercase(),
                    selected_global_rules(view).join(", ")
                ),
                theme.muted_style(),
            )));
            lines.push(Line::from(Span::styled(
                "Press d to remove the selected local rule.",
                theme.muted_style(),
            )));
        }
        PermissionSection::Recent => {
            if view.recent_events.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No recent permission events.",
                    theme.muted_style(),
                )));
            } else {
                for (index, event) in view.recent_events.iter().enumerate() {
                    let selected = index == view.selected;
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{} ", if selected { BLACK_CIRCLE } else { " " }),
                            Style::default().fg(if selected { theme.brand } else { theme.muted }),
                        ),
                        Span::styled(event.tool_name.clone(), Style::default().fg(theme.text)),
                        Span::styled("  ", theme.muted_style()),
                        Span::styled(event.decision.clone(), theme.muted_style()),
                    ]));
                }
            }
            lines.push(Line::from(Span::styled(
                "Press a/n/k to add the selected event to allow/deny/ask.",
                theme.muted_style(),
            )));
        }
    }

    lines.push(Line::default());
    lines
}

fn render_permission_tabs(section: PermissionSection, theme: TerminalTheme) -> Vec<Span<'static>> {
    let tabs = [
        PermissionSection::Allow,
        PermissionSection::Deny,
        PermissionSection::Ask,
        PermissionSection::Recent,
    ];
    let mut spans = Vec::new();
    for (index, tab) in tabs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if tab == section {
            Style::default()
                .fg(theme.panel)
                .bg(theme.brand)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted)
        };
        spans.push(Span::styled(
            format!("[{}]", permission_section_name(tab)),
            style,
        ));
    }
    spans
}

fn permission_section_name(section: PermissionSection) -> &'static str {
    match section {
        PermissionSection::Allow => "Allow",
        PermissionSection::Deny => "Deny",
        PermissionSection::Ask => "Ask",
        PermissionSection::Recent => "Recent",
    }
}

fn permissions_item_count(view: &PermissionsViewState) -> usize {
    match view.section {
        PermissionSection::Allow | PermissionSection::Deny | PermissionSection::Ask => {
            selected_section_rules(view).len().max(1)
        }
        PermissionSection::Recent => view.recent_events.len().max(1),
    }
}

fn selected_section_rules(view: &PermissionsViewState) -> &[String] {
    match view.section {
        PermissionSection::Allow => &view.local_permissions.allow_tools,
        PermissionSection::Deny => &view.local_permissions.deny_tools,
        PermissionSection::Ask => &view.local_permissions.ask_tools,
        PermissionSection::Recent => &[],
    }
}

fn selected_global_rules(view: &PermissionsViewState) -> &[String] {
    match view.section {
        PermissionSection::Allow => &view.global_permissions.allow_tools,
        PermissionSection::Deny => &view.global_permissions.deny_tools,
        PermissionSection::Ask => &view.global_permissions.ask_tools,
        PermissionSection::Recent => &[],
    }
}

fn selected_local_rule(view: &PermissionsViewState) -> Option<&str> {
    selected_section_rules(view)
        .get(view.selected)
        .map(String::as_str)
}

fn refresh_runtime_permissions(
    runtime_permissions: &mut crate::permissions::PermissionsSettings,
    view: &PermissionsViewState,
) {
    runtime_permissions.mode = view
        .local_permissions
        .mode
        .unwrap_or(view.global_permissions.mode);
    runtime_permissions.allow_tools = merge_runtime_rules(
        &view.global_permissions.allow_tools,
        &view.local_permissions.allow_tools,
    );
    runtime_permissions.deny_tools = merge_runtime_rules(
        &view.global_permissions.deny_tools,
        &view.local_permissions.deny_tools,
    );
    runtime_permissions.ask_tools = merge_runtime_rules(
        &view.global_permissions.ask_tools,
        &view.local_permissions.ask_tools,
    );
}

fn merge_runtime_rules(global: &[String], local: &[String]) -> Vec<String> {
    let mut merged = global.to_vec();
    for rule in local {
        if !merged.iter().any(|entry| entry.eq_ignore_ascii_case(rule)) {
            merged.push(rule.clone());
        }
    }
    merged
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::state::TextSelection;

    #[test]
    fn wrap_plain_line_splits_at_width() {
        assert_eq!(
            wrap_plain_line("abcdef", 2),
            vec!["ab".to_string(), "cd".to_string(), "ef".to_string()]
        );
    }

    #[test]
    fn selection_point_uses_visible_scroll_offset() {
        let mut state = TerminalState::new(Settings::default(), None);
        state.chat_area = ratatui::layout::Rect::new(0, 0, 20, 5);
        state.chat_scroll_row = 3;
        state.chat_plain_lines = vec![
            "zero".to_string(),
            "one".to_string(),
            "two".to_string(),
            "three".to_string(),
            "four".to_string(),
        ];

        let point = selection_point_for_mouse(&state, 2, 1).expect("point");
        assert_eq!(point.line, 4);
        assert_eq!(point.column, 2);
    }

    #[test]
    fn selection_range_maps_multiline_bounds() {
        let mut state = TerminalState::new(Settings::default(), None);
        state.chat_plain_lines = vec!["hello".to_string(), "world".to_string()];
        state.selection = Some(TextSelection {
            anchor: SelectionPoint { line: 0, column: 1 },
            focus: SelectionPoint { line: 1, column: 2 },
        });

        assert_eq!(selection_range_for_line(&state, 0), Some((1, 4)));
        assert_eq!(selection_range_for_line(&state, 1), Some((0, 2)));
    }

    #[test]
    fn double_click_seeds_word_selection() {
        let mut state = TerminalState::new(Settings::default(), None);
        state.chat_plain_lines = vec!["say hello-world".to_string()];

        let (anchor, focus, mode) =
            selection_seed_for_click(&state, SelectionPoint { line: 0, column: 5 }, 2);
        assert_eq!(mode, SelectionMode::Word);
        assert_eq!(anchor.column, 4);
        assert_eq!(focus.column, 14);
    }

    #[test]
    fn triple_click_seeds_full_line_selection() {
        let mut state = TerminalState::new(Settings::default(), None);
        state.chat_plain_lines = vec!["copy me".to_string()];

        let (anchor, focus, mode) =
            selection_seed_for_click(&state, SelectionPoint { line: 0, column: 2 }, 3);
        assert_eq!(mode, SelectionMode::Line);
        assert_eq!(anchor.column, 0);
        assert_eq!(focus.column, 6);
    }

    #[test]
    fn click_count_resets_after_timeout() {
        let point = SelectionPoint { line: 1, column: 2 };
        let previous = SelectionClickState {
            point,
            count: 2,
            at: Instant::now() - Duration::from_millis(500),
        };

        assert_eq!(next_click_count(Some(previous), point, Instant::now()), 1);
    }
}
