use super::{
    markdown::ChatRenderLine,
    state::{
        format_arguments_preview, format_tool_body, ChatWorkerOutcome, ChatWorkerResult,
        ChatWorkerUpdate, DisplayMessage, DisplayRole, FallbackField,
        MessageSelectorConfirmationState, MessageSelectorItem, MessageSelectorMode,
        MessageSelectorState, OnboardingStep, PendingChatRequest, PermissionSection,
        PermissionsViewState, PrimaryField, ResumePickerState, SelectionClickState, SelectionMode,
        SelectionPoint, TaskProgressItem, TerminalState, TranscriptViewMode, ViewMode,
    },
    theme::{TerminalTheme, BLACK_CIRCLE},
    ui::{
        layout::{split_chat_screen, split_onboarding_screen},
        messages::{push_wrapped_line, render_render_block},
        render_model::build_render_blocks as build_ui_render_blocks,
    },
};
use crate::{
    agents_runtime::{
        resume_agent_task_after_approval, run_agent_with_parent_history, AgentTaskStore,
    },
    compact::is_compact_summary_content,
    compact::CompactService,
    config::{
        add_project_local_permission_rule, load_project_local_settings,
        remove_project_local_permission_rule, ApiProtocol, ApiProvider, FallbackTarget,
        ProjectPermissionRuleKind, Settings,
    },
    file_history::FileHistoryStore,
    input::commands::{
        init::{run_init, InitMode},
        local::{
            run_diff_command, run_doctor_command, run_mcp_command, run_plugin_command,
            run_skills_command,
        },
        plan::{plan_help_text, render_session_plan},
        registry::SlashCommandRegistry,
        spec::SlashCommandSpec,
    },
    input::{
        format_help_text, format_status_text, InputProcessor, LocalCommand, PlanSlashAction,
        ProcessedInput,
    },
    onboarding::OnboardingDraft,
    permissions::events::{PermissionEvent, PermissionEventStore},
    runtime::{
        ApprovalAction, QueryEngine, QueryProgressEvent, QueryTurnResult, RuntimeMessage,
        TurnStatus,
    },
    services::AgentsService,
    session::{SessionInfo, SessionKind, SessionQuery},
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
    layout::Alignment,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Terminal,
};
use std::{
    fs,
    io::{stdout, Stdout, Write},
    process::{Command, Stdio},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MAX_SLASH_MENU_ITEMS: usize = 8;

pub struct TerminalApp {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    state: TerminalState,
    theme: TerminalTheme,
}

const MAX_VISIBLE_TRANSCRIPT_MESSAGES: usize = 80;
const MAX_TRANSCRIPT_MODE_MESSAGES: usize = 240;
const THINKING_PREVIEW_CHARS: usize = 0;

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
            state_changed |= self.poll_task_notifications();
            state_changed |= self.refresh_active_task_progress();
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
                        let selection_text = self.state.selection_text().unwrap_or_default();
                        match copy_text_to_clipboard(selection_text.clone()) {
                            Ok(()) => {
                                self.state.mark_selection_copied(&selection_text);
                                self.state.status =
                                    self.state.last_copy_status.clone().unwrap_or_else(|| {
                                        "Selection copied to clipboard.".to_string()
                                    });
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
                    let old_offset = self.state.scroll_offset;
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            self.state.scroll_offset = self.state.scroll_offset.saturating_sub(3);
                            if self.state.scroll_offset == 0 {
                                self.state.chat_auto_follow = true;
                            }
                            return Ok(self.state.scroll_offset != old_offset);
                        }
                        MouseEventKind::ScrollUp => {
                            self.state.scroll_offset = self.state.scroll_offset.saturating_add(3);
                            self.state.chat_auto_follow = false;
                            return Ok(self.state.scroll_offset != old_offset);
                        }
                        _ => {}
                    }

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
                                        let selection_text =
                                            self.state.selection_text().unwrap_or_default();
                                        match copy_text_to_clipboard(selection_text.clone()) {
                                            Ok(()) => {
                                                self.state.mark_selection_copied(&selection_text);
                                                self.state.status = self
                                                    .state
                                                    .last_copy_status
                                                    .clone()
                                                    .unwrap_or_else(|| {
                                                        "Selection copied to clipboard.".to_string()
                                                    });
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
                }
                Ok(false)
            }
            Event::Paste(text) => {
                if self.state.view != ViewMode::Chat {
                    return Ok(false);
                }
                if text.chars().count() > 2048 || text.lines().count() > 8 {
                    let token = self.state.register_paste(text);
                    self.state.input.push_str(&token);
                    self.state.status = "Large paste collapsed in input box.".to_string();
                } else {
                    self.state.input.push_str(&text);
                }
                self.state.refresh_slash_menu();
                Ok(true)
            }
            Event::Resize(_, _) => Ok(true),
            _ => Ok(false),
        }
    }

    fn current_slash_menu_window_size(&self, command_count: usize) -> usize {
        let total_height = self.terminal.size().map(|rect| rect.height).unwrap_or(24);
        slash_menu_window_size(total_height, command_count)
    }

    fn handle_chat_key(&mut self, key: KeyEvent) -> bool {
        if self.state.resume_picker.is_some() {
            return self.handle_resume_picker_key(key);
        }

        if self.state.message_selector.is_some() {
            return self.handle_message_selector_key(key);
        }

        if self.state.permissions_view.is_some() {
            return self.handle_permissions_view_key(key);
        }

        if self.state.pending_approval.is_some() {
            return self.handle_approval_key(key);
        }

        match key.code {
            KeyCode::Up if self.state.slash_menu_visible => {
                let commands = matching_slash_commands(&self.state.input);
                let window_size = self.current_slash_menu_window_size(commands.len());
                if !commands.is_empty() {
                    self.state
                        .clamp_slash_menu_selection(commands.len(), window_size);
                    self.state.slash_menu_selected =
                        self.state.slash_menu_selected.saturating_sub(1);
                    self.state
                        .clamp_slash_menu_selection(commands.len(), window_size);
                    return true;
                }
                false
            }
            KeyCode::Down if self.state.slash_menu_visible => {
                let commands = matching_slash_commands(&self.state.input);
                let window_size = self.current_slash_menu_window_size(commands.len());
                if !commands.is_empty() {
                    self.state
                        .clamp_slash_menu_selection(commands.len(), window_size);
                }
                if !commands.is_empty() && self.state.slash_menu_selected + 1 < commands.len() {
                    self.state.slash_menu_selected += 1;
                    self.state
                        .clamp_slash_menu_selection(commands.len(), window_size);
                    return true;
                }
                !commands.is_empty()
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.transcript_mode = match self.state.transcript_mode {
                    TranscriptViewMode::Main => TranscriptViewMode::Transcript,
                    TranscriptViewMode::Transcript => TranscriptViewMode::Main,
                };
                self.state.scroll_offset = 0;
                self.state.status = match self.state.transcript_mode {
                    TranscriptViewMode::Main => "Returned to main chat view.".to_string(),
                    TranscriptViewMode::Transcript => "Opened transcript view.".to_string(),
                };
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.verbose_transcript = !self.state.verbose_transcript;
                self.state.status = if self.state.verbose_transcript {
                    "Verbose transcript enabled.".to_string()
                } else {
                    "Verbose transcript disabled.".to_string()
                };
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Char('v')
                if self.state.transcript_mode == TranscriptViewMode::Transcript
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.verbose_transcript = !self.state.verbose_transcript;
                self.state.status = if self.state.verbose_transcript {
                    "Verbose transcript enabled.".to_string()
                } else {
                    "Verbose transcript disabled.".to_string()
                };
                self.state.mark_chat_render_dirty();
                true
            }
            KeyCode::Up if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let old_offset = self.state.scroll_offset;
                self.state.scroll_offset = self.state.scroll_offset.saturating_add(1);
                self.state.chat_auto_follow = false;
                self.state.scroll_offset != old_offset
            }
            KeyCode::Down if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let old_offset = self.state.scroll_offset;
                self.state.scroll_offset = self.state.scroll_offset.saturating_sub(1);
                if self.state.scroll_offset == 0 {
                    self.state.chat_auto_follow = true;
                }
                self.state.scroll_offset != old_offset
            }
            KeyCode::PageUp if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let old_offset = self.state.scroll_offset;
                self.state.scroll_offset = self.state.scroll_offset.saturating_add(12);
                self.state.chat_auto_follow = false;
                self.state.scroll_offset != old_offset
            }
            KeyCode::PageDown if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let old_offset = self.state.scroll_offset;
                self.state.scroll_offset = self.state.scroll_offset.saturating_sub(12);
                if self.state.scroll_offset == 0 {
                    self.state.chat_auto_follow = true;
                }
                self.state.scroll_offset != old_offset
            }
            KeyCode::Home if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                self.state.scroll_offset = usize::MAX / 4;
                self.state.chat_auto_follow = false;
                true
            }
            KeyCode::End if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let changed = self.state.scroll_offset != 0;
                self.state.scroll_offset = 0;
                self.state.chat_auto_follow = true;
                changed
            }
            KeyCode::Char('k') if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let old_offset = self.state.scroll_offset;
                self.state.scroll_offset = self.state.scroll_offset.saturating_add(1);
                self.state.chat_auto_follow = false;
                self.state.scroll_offset != old_offset
            }
            KeyCode::Char('j') if self.state.transcript_mode == TranscriptViewMode::Transcript => {
                let old_offset = self.state.scroll_offset;
                self.state.scroll_offset = self.state.scroll_offset.saturating_sub(1);
                if self.state.scroll_offset == 0 {
                    self.state.chat_auto_follow = true;
                }
                self.state.scroll_offset != old_offset
            }
            KeyCode::Enter => {
                if self.state.slash_menu_visible {
                    let commands = matching_slash_commands(&self.state.input);
                    if self.state.apply_selected_slash_command(&commands) {
                        return true;
                    }
                }
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.state.input.push('\n');
                    self.state.refresh_slash_menu();
                    true
                } else {
                    self.submit_prompt()
                }
            }
            KeyCode::Backspace => {
                let had_input = !self.state.input.is_empty();
                self.state.input.pop();
                self.state.refresh_slash_menu();
                let commands = matching_slash_commands(&self.state.input);
                self.state.clamp_slash_menu_selection(
                    commands.len(),
                    self.current_slash_menu_window_size(commands.len()),
                );
                had_input
            }
            KeyCode::Tab => {
                let commands = matching_slash_commands(&self.state.input);
                if self.state.apply_selected_slash_command(&commands) {
                    return true;
                }
                self.state.view = ViewMode::Onboarding;
                self.state.onboarding_step = OnboardingStep::Summary;
                self.state.status = "Opened configuration summary.".to_string();
                true
            }
            KeyCode::Esc => {
                if self.state.has_selection() {
                    self.state.clear_selection();
                    self.state.mark_chat_render_dirty();
                    self.state.status = "Selection cleared.".to_string();
                    return true;
                }
                if self.state.transcript_mode == TranscriptViewMode::Transcript {
                    self.state.transcript_mode = TranscriptViewMode::Main;
                    self.state.scroll_offset = 0;
                    self.state.status = "Returned to main chat view.".to_string();
                    self.state.mark_chat_render_dirty();
                    return true;
                }
                let had_input = !self.state.input.is_empty();
                self.state.input.clear();
                self.state.refresh_slash_menu();
                self.state.clamp_slash_menu_selection(0, 1);
                had_input
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.input.push(ch);
                self.state.refresh_slash_menu();
                let commands = matching_slash_commands(&self.state.input);
                self.state.clamp_slash_menu_selection(
                    commands.len(),
                    self.current_slash_menu_window_size(commands.len()),
                );
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

    fn handle_message_selector_key(&mut self, key: KeyEvent) -> bool {
        if self.state.message_selector.is_none() {
            return false;
        }

        let confirmation_open = self
            .state
            .message_selector
            .as_ref()
            .and_then(|selector| selector.confirmation.as_ref())
            .is_some();

        if confirmation_open {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let message_id = self
                        .state
                        .message_selector
                        .as_ref()
                        .and_then(|selector| selector.confirmation.as_ref())
                        .map(|confirmation| confirmation.message_id.clone());
                    if let Some(message_id) = message_id {
                        self.state.message_selector = None;
                        return self.rewind_current_session(&message_id, false);
                    }
                    false
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    if let Some(selector) = self.state.message_selector.as_mut() {
                        selector.confirmation = None;
                    }
                    self.state.status = "Cancelled rewind confirmation.".to_string();
                    self.state.mark_chat_render_dirty();
                    true
                }
                _ => false,
            }
        } else {
            match key.code {
                KeyCode::Up => {
                    if let Some(selector) = self.state.message_selector.as_mut() {
                        selector.selected = selector.selected.saturating_sub(1);
                    }
                    self.state.mark_chat_render_dirty();
                    true
                }
                KeyCode::Down => {
                    if let Some(selector) = self.state.message_selector.as_mut() {
                        if selector.selected + 1 < selector.items.len() {
                            selector.selected += 1;
                        }
                    }
                    self.state.mark_chat_render_dirty();
                    true
                }
                KeyCode::Enter => self.submit_message_selector_selection(),
                KeyCode::Esc => {
                    self.state.message_selector = None;
                    self.state.status = "Closed message picker.".to_string();
                    self.state.mark_chat_render_dirty();
                    true
                }
                _ => false,
            }
        }
    }

    fn open_message_selector(&mut self, mode: MessageSelectorMode) -> bool {
        if self.state.thinking || self.state.pending_approval.is_some() {
            self.state.status = match mode {
                MessageSelectorMode::Branch => "Cannot branch while a turn is active.".to_string(),
                MessageSelectorMode::Rewind { .. } => {
                    "Cannot rewind while a turn or approval is active.".to_string()
                }
            };
            return true;
        }
        let Some(session) = self.state.active_session.clone() else {
            self.state.status = match mode {
                MessageSelectorMode::Branch => "No active session available to branch.".to_string(),
                MessageSelectorMode::Rewind { .. } => {
                    "No active session available to rewind.".to_string()
                }
            };
            return true;
        };
        let items = self.build_user_message_selector_items(&session);
        if items.is_empty() {
            self.state.status = "No user messages available in this session.".to_string();
            return true;
        }
        self.state.message_selector = Some(MessageSelectorState {
            mode,
            items,
            selected: 0,
            confirmation: None,
        });
        self.state.status = match mode {
            MessageSelectorMode::Branch => "Select a user message to branch from.".to_string(),
            MessageSelectorMode::Rewind { files_only: true } => {
                "Select a user message to rewind tracked files to.".to_string()
            }
            MessageSelectorMode::Rewind { files_only: false } => {
                "Select a user message to rewind conversation to.".to_string()
            }
        };
        self.state.mark_chat_render_dirty();
        true
    }

    fn submit_message_selector_selection(&mut self) -> bool {
        let Some(selector) = self.state.message_selector.clone() else {
            return false;
        };
        let Some(item) = selector.items.get(selector.selected).cloned() else {
            return false;
        };

        match selector.mode {
            MessageSelectorMode::Branch => {
                self.state.message_selector = None;
                self.branch_current_session(Some(item.message_id))
            }
            MessageSelectorMode::Rewind { files_only: true } => {
                self.state.message_selector = None;
                self.rewind_current_session(&item.message_id, true)
            }
            MessageSelectorMode::Rewind { files_only: false } => {
                if item.has_file_changes {
                    let (changed_files, warning) = self
                        .state
                        .active_session
                        .as_ref()
                        .map(|session| {
                            FileHistoryStore::for_project(Some(std::path::Path::new(
                                &self.state.settings.working_dir,
                            )))
                            .ok()
                            .and_then(|store| {
                                store
                                    .file_history_get_change_descriptors(session, &item.message_id)
                                    .ok()
                                    .map(|descriptors| {
                                        let warning = descriptors.iter().any(|descriptor| descriptor.truncated).then(|| {
                                            "Command snapshot tracking was truncated; some changed files may be missing from this preview.".to_string()
                                        });
                                        let files = descriptors
                                            .into_iter()
                                            .map(|descriptor| {
                                                format!(
                                                    "[{}] {}",
                                                    match descriptor.origin {
                                                        crate::file_history::FileHistoryOrigin::FileWrite => "write",
                                                        crate::file_history::FileHistoryOrigin::FileEdit => "edit",
                                                        crate::file_history::FileHistoryOrigin::ExecuteCommandSnapshot => "cmd",
                                                    },
                                                    descriptor.path
                                                )
                                            })
                                            .collect::<Vec<_>>();
                                        (files, warning)
                                    })
                            })
                        })
                        .unwrap_or_default()
                        .unwrap_or_default();
                    if let Some(selector) = self.state.message_selector.as_mut() {
                        selector.confirmation = Some(MessageSelectorConfirmationState {
                            message_id: item.message_id,
                            preview: item.preview,
                            changed_files,
                            warning,
                        });
                    }
                    self.state.status =
                        "Confirm rewind: this will also restore tracked files.".to_string();
                    self.state.mark_chat_render_dirty();
                    true
                } else {
                    self.state.message_selector = None;
                    self.rewind_current_session(&item.message_id, false)
                }
            }
        }
    }

    fn build_user_message_selector_items(
        &self,
        session: &crate::session::Session,
    ) -> Vec<MessageSelectorItem> {
        let history_store = FileHistoryStore::for_project(Some(std::path::Path::new(
            &self.state.settings.working_dir,
        )))
        .ok();
        session
            .messages
            .iter()
            .rev()
            .filter(|message| message.role.eq_ignore_ascii_case("user"))
            .map(|message| MessageSelectorItem {
                message_id: message.id.clone(),
                preview: format!(
                    "{}  {}",
                    short_message_id(&message.id),
                    summarize_selector_preview(&message.content)
                ),
                has_file_changes: history_store
                    .as_ref()
                    .is_some_and(|store| store.file_history_has_any_changes(session, &message.id)),
            })
            .collect()
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
        let display_prompt = self.state.input.trim().to_string();
        if display_prompt.is_empty() {
            return false;
        }
        let prompt = self.state.resolve_input_for_submission().trim().to_string();

        match InputProcessor::new().process(&prompt) {
            ProcessedInput::LocalCommand(command) => {
                self.state.input.clear();
                self.execute_local_command(command)
            }
            ProcessedInput::Error(message) => {
                self.state.input.clear();
                self.push_system_message(message.clone());
                self.state.status = message;
                true
            }
            ProcessedInput::Prompt(prompt) => {
                if self.state.thinking || self.state.pending_approval.is_some() {
                    return false;
                }
                self.submit_runtime_prompt(display_prompt, prompt);
                true
            }
        }
    }

    fn submit_runtime_prompt(&mut self, display_prompt: String, prompt: String) {
        if let Err(error) = self.ensure_active_session() {
            self.push_system_message(format!("Failed to initialize session: {}", error));
            self.state.status = "Session initialization failed.".to_string();
            return;
        }
        let message = DisplayMessage::transient(DisplayRole::User, display_prompt);
        self.state.messages.push(message);
        self.state.input.clear();
        self.state.scroll_offset = 0;
        self.state.chat_auto_follow = true;
        self.state.thinking = true;
        self.state.spinner_tick = 0;
        self.state.last_tick = std::time::Instant::now();
        self.state.status = if self.state.plan_mode {
            "Planning with built-in Plan Agent".to_string()
        } else {
            format!(
                "Querying {}/{}",
                self.state.settings.api.provider_label(),
                self.state.settings.model
            )
        };
        self.state.mark_chat_render_dirty();

        let user_message = RuntimeMessage::user(prompt);
        let base_history = Arc::clone(&self.state.conversation_history);
        let receiver = if self.state.plan_mode {
            spawn_plan_request(
                self.state.settings.clone(),
                Arc::clone(&base_history),
                user_message.content.clone(),
                self.state.active_session_id.clone(),
            )
        } else {
            spawn_chat_request(
                self.state.settings.clone(),
                Arc::clone(&base_history),
                user_message.clone(),
                self.state.active_session_id.clone(),
            )
        };
        self.state.pending_response = Some(PendingChatRequest {
            receiver,
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
            LocalCommand::Diff { full } => self.handle_sync_local_text(
                run_diff_command(Some(&self.state.settings.working_dir), full),
                "Displayed workspace diff.",
                "Diff failed.",
            ),
            LocalCommand::Doctor => self.handle_sync_local_text(
                run_doctor_command(&self.state.settings),
                "Displayed doctor report.",
                "Doctor failed.",
            ),
            LocalCommand::Init { force, append } => {
                let mode = if append {
                    InitMode::Append
                } else if force {
                    InitMode::Force
                } else {
                    InitMode::Create
                };
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                match run_init(&cwd, mode) {
                    Ok(outcome) => {
                        self.push_system_message(outcome.message.clone());
                        self.state.status = outcome.message;
                    }
                    Err(error) => {
                        let message = format!("/init failed: {}", error);
                        self.push_system_message(message.clone());
                        self.state.status = message;
                    }
                }
                true
            }
            LocalCommand::Branch { message_id } => match message_id {
                Some(message_id) => self.branch_current_session(Some(message_id)),
                None => self.open_message_selector(MessageSelectorMode::Branch),
            },
            LocalCommand::Compact { instructions } => self.compact_current_history(instructions),
            LocalCommand::Mcp { action } => self.handle_async_local_text(
                run_mcp_command(&action),
                "Handled MCP command.",
                "MCP command failed.",
            ),
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
            LocalCommand::Plan { action } => self.handle_plan_command(action),
            LocalCommand::Plugin { action } => self.handle_async_local_text(
                run_plugin_command(&action),
                "Handled plugin command.",
                "Plugin command failed.",
            ),
            LocalCommand::Rewind {
                message_id,
                files_only,
            } => match message_id {
                Some(message_id) => self.rewind_current_session(&message_id, files_only),
                None => self.open_message_selector(MessageSelectorMode::Rewind { files_only }),
            },
            LocalCommand::Status => {
                self.push_system_message(format_status_text(
                    &self.state.settings,
                    self.state.active_session_id.as_deref(),
                    self.state.conversation_history.len(),
                    self.state.pending_approval.is_some(),
                    self.state.last_usage_total,
                    self.state.plan_mode,
                    self.state.active_plan.as_ref(),
                ));
                self.state.status = "Displayed runtime status.".to_string();
                true
            }
            LocalCommand::Skills { action } => self.handle_sync_local_text(
                run_skills_command(&action),
                "Handled skills command.",
                "Skills command failed.",
            ),
            LocalCommand::Resume { session_id } => match session_id {
                Some(session_id) => self.resume_session_by_query(&session_id),
                None => self.open_resume_picker(),
            },
        }
    }

    fn ensure_active_session(&mut self) -> anyhow::Result<()> {
        if self.state.active_session.is_none() && self.state.settings.session.persist_transcript {
            let session = self.state.session_manager.create(Some("tui-session"))?;
            self.state.active_session_id = Some(session.id.clone());
            self.state.active_session = Some(session);
        }
        Ok(())
    }

    fn branch_current_session(&mut self, message_id: Option<String>) -> bool {
        if self.state.thinking || self.state.pending_approval.is_some() {
            self.state.status = "Cannot branch while a turn is active.".to_string();
            return true;
        }
        if self.state.conversation_history.is_empty() {
            self.state.status = "No conversation available to branch.".to_string();
            return true;
        }
        if let Err(error) = self.ensure_active_session() {
            self.push_system_message(format!("Failed to initialize session: {}", error));
            self.state.status = "Session initialization failed.".to_string();
            return true;
        }
        if let Err(error) = self.state.persist_current_session() {
            self.push_system_message(format!("Session save failed: {}", error));
            self.state.status = "Session save failed.".to_string();
            return true;
        }
        let Some(source_session) = self.state.active_session.clone() else {
            self.state.status = "No active session available to branch.".to_string();
            return true;
        };
        let resolved_message_id = message_id
            .as_deref()
            .map(|value| resolve_user_message_id(&source_session, value))
            .transpose();
        let resolved_message_id = match resolved_message_id {
            Ok(value) => value,
            Err(error) => {
                self.push_system_message(format!("Branch failed: {}", error));
                self.state.status = "Branch failed.".to_string();
                return true;
            }
        };
        match self.state.session_manager.create_fork_session(
            &source_session,
            resolved_message_id.as_deref(),
            None,
        ) {
            Ok(forked) => {
                self.state.restore_session(forked.clone());
                self.push_system_message(format!(
                    "Forked session {} from {}{}.",
                    forked.id,
                    source_session.id,
                    resolved_message_id
                        .as_deref()
                        .map(|id| format!(" at {}", id))
                        .unwrap_or_default()
                ));
                self.state.status = "Switched to forked session.".to_string();
                true
            }
            Err(error) => {
                self.push_system_message(format!("Branch failed: {}", error));
                self.state.status = "Branch failed.".to_string();
                true
            }
        }
    }

    fn rewind_current_session(&mut self, message_id: &str, files_only: bool) -> bool {
        if self.state.thinking || self.state.pending_approval.is_some() {
            self.state.status = "Cannot rewind while a turn or approval is active.".to_string();
            return true;
        }
        let Some(mut session) = self.state.active_session.clone() else {
            self.state.status = "No active session available to rewind.".to_string();
            return true;
        };
        let resolved_message_id = match resolve_user_message_id(&session, message_id) {
            Ok(value) => value,
            Err(error) => {
                self.push_system_message(format!("Conversation rewind failed: {}", error));
                self.state.status = "Conversation rewind failed.".to_string();
                return true;
            }
        };

        let file_rewind = if files_only {
            Some(
                FileHistoryStore::for_project(Some(std::path::Path::new(
                    &self.state.settings.working_dir,
                )))
                .and_then(|store| store.rewind_session_to_message(&session, &resolved_message_id)),
            )
        } else {
            None
        };
        if let Some(result) = file_rewind {
            match result {
                Ok(rewind) => {
                    self.push_system_message(format_file_rewind_summary(&rewind));
                }
                Err(error) => {
                    self.push_system_message(format!("File rewind failed: {}", error));
                    self.state.status = "File rewind failed.".to_string();
                    return true;
                }
            }
        }
        if files_only {
            self.state.status = "Files rewound.".to_string();
            return true;
        }

        let file_rewind = match FileHistoryStore::for_project(Some(std::path::Path::new(
            &self.state.settings.working_dir,
        )))
        .and_then(|store| store.rewind_session_to_message(&session, &resolved_message_id))
        {
            Ok(rewind) => rewind,
            Err(error) => {
                self.push_system_message(format!("File rewind failed: {}", error));
                self.state.status = "File rewind failed.".to_string();
                return true;
            }
        };

        match self
            .state
            .session_manager
            .rewind_session_to_message(&mut session, &resolved_message_id)
        {
            Ok(restored_input) => {
                self.state.restore_session(session);
                self.state.input = restored_input;
                self.push_system_message(format_file_rewind_summary(&file_rewind));
                self.push_system_message(format!(
                    "Rewound conversation to {}.",
                    resolved_message_id
                ));
                self.state.status = "Conversation rewound.".to_string();
                true
            }
            Err(error) => {
                self.push_system_message(format!("Conversation rewind failed: {}", error));
                self.state.status = "Conversation rewind failed.".to_string();
                true
            }
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
        self.state
            .messages
            .push(DisplayMessage::transient(DisplayRole::System, content));
        self.state.mark_chat_render_dirty();
    }

    fn handle_sync_local_text(
        &mut self,
        result: anyhow::Result<String>,
        success_status: &str,
        error_status: &str,
    ) -> bool {
        match result {
            Ok(message) => {
                self.push_system_message(message);
                self.state.status = success_status.to_string();
            }
            Err(error) => {
                self.push_system_message(error.to_string());
                self.state.status = error_status.to_string();
            }
        }
        true
    }

    fn handle_async_local_text<F>(
        &mut self,
        future: F,
        success_status: &str,
        error_status: &str,
    ) -> bool
    where
        F: std::future::Future<Output = anyhow::Result<String>>,
    {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                self.push_system_message(format!("Failed to initialize local runtime: {}", error));
                self.state.status = error_status.to_string();
                return true;
            }
        };
        self.handle_sync_local_text(runtime.block_on(future), success_status, error_status)
    }

    fn handle_plan_command(&mut self, action: PlanSlashAction) -> bool {
        match action {
            PlanSlashAction::Enter { prompt } => {
                if self.state.plan_mode {
                    self.show_active_plan();
                } else {
                    self.state.plan_mode = true;
                    self.push_system_message("Plan mode enabled.".to_string());
                    self.persist_plan_state("Plan mode enabled.");
                    if let Some(prompt) = prompt {
                        self.submit_runtime_prompt(prompt.clone(), prompt);
                    }
                }
                true
            }
            PlanSlashAction::Show => {
                self.show_active_plan();
                true
            }
            PlanSlashAction::Open => {
                self.push_system_message(
                    "Opening the current plan in an external editor is not implemented."
                        .to_string(),
                );
                self.state.status = "Plan open is not implemented.".to_string();
                true
            }
            PlanSlashAction::Exit => {
                self.state.plan_mode = false;
                self.push_system_message("Plan mode disabled.".to_string());
                self.persist_plan_state("Plan mode disabled.");
                true
            }
        }
    }

    fn show_active_plan(&mut self) {
        let content = match self.state.active_plan.as_ref() {
            Some(plan) => format!(
                "Plan mode: {}\n\n{}",
                if self.state.plan_mode { "on" } else { "off" },
                render_session_plan(plan)
            ),
            None => format!(
                "Plan mode: {}\n\nNo current session plan.\n\n{}",
                if self.state.plan_mode { "on" } else { "off" },
                plan_help_text()
            ),
        };
        self.push_system_message(content);
        self.state.status = "Displayed current session plan.".to_string();
    }

    fn persist_plan_state(&mut self, status: &str) {
        if let Err(error) = self.ensure_active_session() {
            self.push_system_message(format!("Failed to initialize session: {}", error));
            self.state.status = "Session initialization failed.".to_string();
            return;
        }
        if let Err(error) = self.state.persist_current_session() {
            self.push_system_message(format!("Session save failed: {}", error));
            self.state.status = "Session save failed.".to_string();
            return;
        }
        self.state.status = status.to_string();
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
                        query: None,
                    });
                    self.state.permissions_view = None;
                    self.state.status = "Select a session to resume.".to_string();
                }
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Failed to list sessions: {}", error);
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Failed to list sessions: {}", error),
                ));
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
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Session {} not found.", session_id),
                ));
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Session restore failed: {}", error);
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Session restore failed: {}", error),
                ));
                self.state.mark_chat_render_dirty();
                true
            }
        }
    }

    fn resume_session_by_query(&mut self, query: &str) -> bool {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.open_resume_picker();
        }

        let parsed_query = parse_resume_query(trimmed);

        match self.state.session_manager.search(parsed_query) {
            Ok(mut sessions) => {
                if let Some(active_id) = &self.state.active_session_id {
                    sessions.retain(|session| session.id != *active_id);
                }
                if sessions.is_empty() {
                    self.state.status =
                        format!("No sessions matched '{}' .", trimmed).replace("' .", "'.");
                    self.push_system_message(format!("No sessions matched '{}'.", trimmed));
                    return true;
                }
                if sessions.len() == 1 {
                    let session_id = sessions[0].id.clone();
                    return self.resume_session_by_id(&session_id);
                }

                self.state.resume_picker = Some(ResumePickerState {
                    sessions,
                    selected: 0,
                    query: Some(trimmed.to_string()),
                });
                self.state.permissions_view = None;
                self.state.status = format!("Multiple sessions matched '{}'.", trimmed);
                self.state.mark_chat_render_dirty();
                true
            }
            Err(error) => {
                self.state.status = format!("Session search failed: {}", error);
                self.push_system_message(format!("Session search failed: {}", error));
                true
            }
        }
    }

    fn open_permissions_view(&mut self) -> bool {
        let global_settings = match Settings::load_global() {
            Ok(settings) => settings.permissions,
            Err(error) => {
                self.state.status = format!("Failed to load global permissions: {}", error);
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Failed to load global permissions: {}", error),
                ));
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
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Failed to load local permissions: {}", error),
                ));
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
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Failed to load permission events: {}", error),
                ));
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
        let mut pending = pending;
        let mut changed = false;

        loop {
            match pending.receiver.try_recv() {
                Ok(ChatWorkerUpdate::Progress(event)) => {
                    self.apply_progress_event(event);
                    changed = true;
                }
                Ok(ChatWorkerUpdate::Finished(ChatWorkerResult { outcome })) => {
                    self.state.thinking = false;
                    match outcome {
                        Ok(ChatWorkerOutcome::Turn(turn)) => self.apply_turn_result(turn),
                        Ok(ChatWorkerOutcome::TaskResume { message }) => {
                            self.push_system_message(message);
                            self.state.status = "Child task resumed.".to_string();
                        }
                        Err(error) => {
                            let had_user_message = pending.user_message.is_some();
                            let mut history = (*pending.base_history).clone();
                            if let Some(user_message) = pending.user_message.take() {
                                history.push(user_message);
                            }
                            if had_user_message {
                                self.state.replace_history(history);
                            }
                            self.state.messages.push(DisplayMessage::transient(
                                DisplayRole::System,
                                format!("Request failed: {}", error),
                            ));
                            self.state.status = "Request failed.".to_string();
                        }
                    }
                    return true;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.state.pending_response = Some(pending);
                    return changed;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let mut restored_history = (*pending.base_history).clone();
                    if let Some(user_message) = pending.user_message.take() {
                        restored_history.push(user_message);
                    }
                    self.state.replace_history(restored_history);
                    self.state.thinking = false;
                    self.state.status = "Request worker disconnected.".to_string();
                    self.state.messages.push(DisplayMessage::transient(
                        DisplayRole::System,
                        "Request worker disconnected.".to_string(),
                    ));
                    self.state.mark_chat_render_dirty();
                    return true;
                }
            }
        }
    }

    fn apply_progress_event(&mut self, event: QueryProgressEvent) {
        match event {
            QueryProgressEvent::ModelRequest { target } => {
                self.state.status = format!("Querying {}", target);
            }
            QueryProgressEvent::ThinkingText(chunk) => {
                self.state.append_streaming_thinking_text(&chunk);
                self.state.status = "Streaming reasoning...".to_string();
            }
            QueryProgressEvent::AssistantText(chunk) => {
                self.state.append_streaming_assistant_text(&chunk);
                self.state.live_thinking_message = None;
            }
            QueryProgressEvent::ToolCall(tool_call) => {
                self.state.live_assistant_message = None;
                self.state.live_thinking_message = None;
                let content = format!(
                    "Preparing tool: {}{}",
                    tool_call.name,
                    bounded_argument_preview(&tool_call.arguments, 4)
                );
                if let Some(index) = self.state.live_tool_message {
                    if let Some(message) = self.state.messages.get_mut(index) {
                        message.content = content;
                    }
                } else {
                    self.state
                        .messages
                        .push(DisplayMessage::transient(DisplayRole::Tool, content));
                    self.state.live_tool_message =
                        Some(self.state.messages.len().saturating_sub(1));
                }
                self.state.status = format!("Running tool {}.", tool_call.name);
                self.state.mark_chat_render_dirty();
            }
            QueryProgressEvent::ToolResult(result) => {
                self.state.live_assistant_message = None;
                self.state.live_thinking_message = None;
                let label = if result.is_error {
                    format!("Tool error: {}", result.name)
                } else {
                    format!("Tool result: {}", result.name)
                };
                let content = format!("{}{}", label, format_tool_body(&result.content));
                if let Some(index) = self.state.live_tool_message.take() {
                    if let Some(message) = self.state.messages.get_mut(index) {
                        message.role = DisplayRole::Tool;
                        message.content = content;
                    } else {
                        self.state
                            .messages
                            .push(DisplayMessage::transient(DisplayRole::Tool, content));
                    }
                } else {
                    self.state
                        .messages
                        .push(DisplayMessage::transient(DisplayRole::Tool, content));
                }
                self.state.status = format!("Completed tool {}.", result.name);
                self.state.mark_chat_render_dirty();
            }
            QueryProgressEvent::AwaitingApproval(pending) => {
                self.state.live_assistant_message = None;
                self.state.live_thinking_message = None;
                self.state.live_tool_message = None;
                self.state.status =
                    format!("Tool approval required for {}.", pending.tool_call.name);
            }
        }
    }

    fn apply_turn_result(&mut self, turn: QueryTurnResult) {
        if self.state.plan_mode {
            if let Some(text) = turn.assistant_text() {
                if !text.trim().is_empty() {
                    self.state.active_plan = Some(text.to_string());
                }
            }
        }
        self.state.last_usage_total = turn.usage.as_ref().map(|usage| usage.total_tokens);
        let should_follow = self.state.chat_auto_follow || self.state.scroll_offset == 0;
        self.state.replace_history(turn.history);
        if should_follow {
            self.state.scroll_offset = 0;
            self.state.chat_auto_follow = true;
        }

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
            self.state.messages.push(DisplayMessage::transient(
                DisplayRole::System,
                format!("Session save failed: {}", error),
            ));
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
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Failed to update local permissions: {}", error),
                ));
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
                self.state.messages.push(DisplayMessage::transient(
                    DisplayRole::System,
                    format!("Failed to save local permissions: {}", error),
                ));
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

        let pending_origin = view_model.origin.clone();
        self.state.thinking = true;
        self.state.status = "Resuming after approval…".to_string();
        self.state.set_pending_approval(None);
        self.state.spinner_tick = 0;
        self.state.last_tick = std::time::Instant::now();
        if let Err(error) = self.state.persist_current_session() {
            self.state.messages.push(DisplayMessage::transient(
                DisplayRole::System,
                format!("Session save failed: {}", error),
            ));
        }
        let base_history = Arc::clone(&self.state.conversation_history);
        let receiver = match pending_origin {
            super::state::PendingApprovalOrigin::ChildTask { task_id, .. } => {
                spawn_task_approval_request(self.state.settings.clone(), task_id, action)
            }
            super::state::PendingApprovalOrigin::FreshTurn
            | super::state::PendingApprovalOrigin::RestoredSession => spawn_approval_request(
                self.state.settings.clone(),
                Arc::clone(&base_history),
                action,
                self.state.active_session_id.clone(),
            ),
        };
        self.state.pending_response = Some(PendingChatRequest {
            receiver,
            base_history,
            user_message: None,
        });
        true
    }

    fn poll_task_notifications(&mut self) -> bool {
        let Some(session_id) = self.state.active_session_id.as_deref() else {
            return false;
        };
        let Ok(store) = AgentTaskStore::for_project(Some(&self.state.settings.working_dir)) else {
            return false;
        };
        let Ok(notifications) = store.drain_notifications(session_id) else {
            return false;
        };
        if notifications.is_empty() {
            return false;
        }

        for notification in notifications {
            let content = match notification.status {
                crate::agents_runtime::AgentTaskStatus::Completed => format!(
                    "Task completed: #{} {}{}",
                    notification.id,
                    notification.subject,
                    notification
                        .result_summary
                        .as_deref()
                        .map(|summary| format!("\n{}", summary))
                        .unwrap_or_default()
                ),
                crate::agents_runtime::AgentTaskStatus::Failed => format!(
                    "Task failed: #{} {}{}",
                    notification.id,
                    notification.subject,
                    notification
                        .error
                        .as_deref()
                        .map(|error| format!("\n{}", error))
                        .unwrap_or_default()
                ),
                _ => continue,
            };
            self.push_system_message(content);
        }
        self.state.mark_chat_render_dirty();
        true
    }

    fn refresh_active_task_progress(&mut self) -> bool {
        let Some(session_id) = self.state.active_session_id.as_deref() else {
            if self.state.active_tasks.is_empty() {
                return false;
            }
            self.state.set_active_tasks(Vec::new());
            return true;
        };
        let Ok(store) = AgentTaskStore::for_project(Some(&self.state.settings.working_dir)) else {
            return false;
        };
        let Ok(tasks) = store.list_for_parent(Some(session_id)) else {
            return false;
        };

        if self.state.pending_approval.is_none() && !self.state.thinking {
            if let Some(task) = tasks.iter().find(|task| {
                matches!(
                    task.status,
                    crate::agents_runtime::AgentTaskStatus::AwaitingApproval
                ) && task.pending_approval.is_some()
            }) {
                if let (Some(pending), Some(child_session_id)) = (
                    task.pending_approval.as_ref(),
                    task.child_session_id.as_ref(),
                ) {
                    self.state.set_pending_approval_with_origin(
                        Some(crate::runtime::PendingApproval {
                            tool_call: pending.tool_call.clone(),
                            reason: pending.reason.clone(),
                        }),
                        super::state::PendingApprovalOrigin::ChildTask {
                            task_id: task.id.clone(),
                            child_session_id: child_session_id.clone(),
                            subject: task.subject.clone(),
                        },
                    );
                    self.state.status = format!(
                        "Child task #{} requires approval for {}.",
                        task.id, pending.tool_call.name
                    );
                }
            }
        }

        let mut active_tasks = tasks
            .into_iter()
            .filter(|task| {
                matches!(
                    task.status,
                    crate::agents_runtime::AgentTaskStatus::Pending
                        | crate::agents_runtime::AgentTaskStatus::Running
                        | crate::agents_runtime::AgentTaskStatus::AwaitingApproval
                )
            })
            .map(|task| TaskProgressItem {
                id: task.id,
                subject: task.subject,
                agent_type: task.agent_type,
                status: task.status,
            })
            .collect::<Vec<_>>();
        active_tasks.sort_by_key(|task| match task.status {
            crate::agents_runtime::AgentTaskStatus::AwaitingApproval => 0,
            crate::agents_runtime::AgentTaskStatus::Running => 1,
            crate::agents_runtime::AgentTaskStatus::Pending => 2,
            _ => 3,
        });

        let unchanged = self.state.active_tasks.len() == active_tasks.len()
            && self
                .state
                .active_tasks
                .iter()
                .zip(active_tasks.iter())
                .all(|(left, right)| {
                    left.id == right.id
                        && left.subject == right.subject
                        && left.agent_type == right.agent_type
                        && left.status == right.status
                });
        if unchanged {
            return false;
        }

        self.state.set_active_tasks(active_tasks);
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
    let layout = split_onboarding_screen(frame.size());

    frame.render_widget(
        Paragraph::new(theme.welcome_lines(layout.hero.width, &state.working_dir))
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false }),
        layout.hero,
    );
    render_onboarding(frame, layout.body, theme, state);
    render_status_line(frame, layout.status, theme, state);
}

fn draw_chat_view(frame: &mut ratatui::Frame<'_>, theme: TerminalTheme, state: &mut TerminalState) {
    let slash_commands = matching_slash_commands(&state.input);
    let slash_menu_height = slash_menu_height(state, slash_commands.len(), frame.size().height);
    let layout = split_chat_screen(frame.size(), slash_menu_height);
    render_chat(frame, layout.transcript, theme, state);
    render_slash_menu(frame, layout.slash_menu, theme, state, &slash_commands);
    render_prompt(frame, layout.prompt, theme, state);
    render_status_line(frame, layout.status, theme, state);
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
    state.scroll_offset =
        clamp_scroll_offset(total_lines as usize, visible as usize, state.scroll_offset);
    let scroll_up = state.scroll_offset as u16;
    let scroll_row = max_scroll.saturating_sub(scroll_up);
    state.chat_area = area;
    state.chat_scroll_row = scroll_row as usize;

    let paragraph = std::mem::take(&mut state.chat_render_cache).scroll((scroll_row, 0));
    frame.render_widget(&paragraph, area);
    state.chat_render_cache = paragraph.scroll((0, 0));
}

fn clamp_scroll_offset(total_lines: usize, visible_lines: usize, requested: usize) -> usize {
    total_lines.saturating_sub(visible_lines).min(requested)
}

fn ensure_chat_cache(state: &mut TerminalState, theme: TerminalTheme, width: u16) {
    if state.chat_render_dirty || state.chat_render_width != width {
        if state.messages.is_empty()
            && state.pending_approval.is_none()
            && state.resume_picker.is_none()
            && state.message_selector.is_none()
            && state.permissions_view.is_none()
            && !state.has_selection()
        {
            let lines = theme.empty_chat_lines(width, &state.working_dir);
            state.chat_plain_lines = lines.iter().map(line_to_plain_text).collect();
            let paragraph = Paragraph::new(lines)
                .alignment(Alignment::Left)
                .wrap(ratatui::widgets::Wrap { trim: false });
            state.chat_render_line_count = paragraph.line_count(width) as u16;
            state.chat_render_cache = paragraph;
            state.chat_render_width = width;
            state.chat_render_dirty = false;
            return;
        }

        let rendered_lines = render_chat_lines(state, theme, width);
        state.chat_plain_lines = rendered_lines
            .iter()
            .map(|line| line.plain_text.clone())
            .collect();
        let lines = rendered_lines
            .iter()
            .enumerate()
            .map(|(index, line)| line.to_line(selection_range_for_line(state, index), theme))
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false });
        state.chat_render_line_count = rendered_lines.len() as u16;
        state.chat_render_cache = paragraph;
        state.chat_render_width = width;
        state.chat_render_dirty = false;
    }
}

fn render_chat_lines(
    state: &TerminalState,
    theme: TerminalTheme,
    width: u16,
) -> Vec<ChatRenderLine> {
    if state.messages.is_empty()
        && state.pending_approval.is_none()
        && state.resume_picker.is_none()
        && state.message_selector.is_none()
        && state.permissions_view.is_none()
    {
        return theme
            .empty_chat_lines(width, &state.working_dir)
            .into_iter()
            .map(|line| ChatRenderLine {
                plain_text: line_to_plain_text(&line),
                spans: line
                    .spans
                    .iter()
                    .map(|span| (span.content.to_string(), span.style))
                    .collect(),
            })
            .collect();
    }

    let mut lines = Vec::new();
    let compact_boundary = state.messages.iter().rposition(|message| {
        matches!(message.role, DisplayRole::System) && is_compact_summary_content(&message.content)
    });
    let boundary_start = compact_boundary.unwrap_or(0);
    let mode_cap = match state.transcript_mode {
        TranscriptViewMode::Main => MAX_VISIBLE_TRANSCRIPT_MESSAGES,
        TranscriptViewMode::Transcript => MAX_TRANSCRIPT_MODE_MESSAGES,
    };
    let boundary_hidden = match state.transcript_mode {
        TranscriptViewMode::Main => compact_boundary.unwrap_or(0),
        TranscriptViewMode::Transcript => 0,
    };
    let visible_start_base = match state.transcript_mode {
        TranscriptViewMode::Main => boundary_start,
        TranscriptViewMode::Transcript => 0,
    };
    let tail_hidden = state
        .messages
        .len()
        .saturating_sub(visible_start_base)
        .saturating_sub(mode_cap);
    let visible_start = visible_start_base + tail_hidden;
    let hidden_count = boundary_hidden + tail_hidden;

    if hidden_count > 0 {
        push_wrapped_line(
            &mut lines,
            format!(
                "{} {} earlier messages hidden. Ctrl+O toggles transcript view.",
                BLACK_CIRCLE, hidden_count
            ),
            Style::default().fg(theme.muted),
            width,
        );
    }

    let visible_messages = &state.messages[visible_start..];
    let last_thinking_index = visible_messages
        .iter()
        .rposition(|message| matches!(message.role, DisplayRole::Thinking));
    let render_blocks = build_ui_render_blocks(visible_messages);

    for block in render_blocks {
        render_render_block(
            &mut lines,
            visible_messages,
            last_thinking_index,
            state.transcript_mode,
            state.verbose_transcript,
            theme,
            width,
            block,
            THINKING_PREVIEW_CHARS,
        );
        lines.push(ChatRenderLine::empty());
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
        match &approval.origin {
            super::state::PendingApprovalOrigin::RestoredSession => {
                push_wrapped_line(
                    &mut lines,
                    "Restored from previous session.".to_string(),
                    theme.muted_style(),
                    width,
                );
            }
            super::state::PendingApprovalOrigin::ChildTask {
                task_id, subject, ..
            } => {
                push_wrapped_line(
                    &mut lines,
                    format!("Child task #{task_id}: {subject}"),
                    theme.muted_style(),
                    width,
                );
            }
            super::state::PendingApprovalOrigin::FreshTurn => {}
        }
        push_wrapped_line(
            &mut lines,
            format!(
                "Tool: {}{}",
                approval.pending.tool_call.name,
                approval
                    .risk_label
                    .as_deref()
                    .map(|risk| format!(" · {}", risk))
                    .unwrap_or_default()
            ),
            Style::default().fg(theme.text),
            width,
        );
        if let Some(summary) = approval.tool_summary.as_deref() {
            push_wrapped_line(
                &mut lines,
                format!("Summary: {}", summary),
                theme.muted_style(),
                width,
            );
        }
        push_wrapped_line(
            &mut lines,
            format!("Reason: {}", approval.pending.reason),
            Style::default().fg(theme.text),
            width,
        );
        let preview_lines = approval.arguments_preview.lines().collect::<Vec<_>>();
        for preview_line in preview_lines.iter().take(8) {
            push_wrapped_line(
                &mut lines,
                format!("Args: {}", preview_line),
                Style::default().fg(theme.text),
                width,
            );
        }
        if preview_lines.len() > 8 {
            push_wrapped_line(
                &mut lines,
                format!("... {} more argument line(s)", preview_lines.len() - 8),
                theme.muted_style(),
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
        lines.push(ChatRenderLine::empty());
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
        if let Some(query) = picker.query.as_deref() {
            push_wrapped_line(
                &mut lines,
                format!("Filtered by: {}", query),
                theme.muted_style(),
                width,
            );
        }
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
        lines.push(ChatRenderLine::empty());
    }

    if let Some(selector) = &state.message_selector {
        push_wrapped_line(
            &mut lines,
            match selector.mode {
                MessageSelectorMode::Branch => "Branch from message".to_string(),
                MessageSelectorMode::Rewind { files_only: true } => {
                    "Rewind tracked files".to_string()
                }
                MessageSelectorMode::Rewind { files_only: false } => {
                    "Rewind conversation".to_string()
                }
            },
            Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD),
            width,
        );
        if let Some(confirmation) = &selector.confirmation {
            push_wrapped_line(
                &mut lines,
                format!("Target: {}", confirmation.preview),
                Style::default().fg(theme.text),
                width,
            );
            push_wrapped_line(
                &mut lines,
                "This will restore tracked files changed after that user turn.".to_string(),
                theme.muted_style(),
                width,
            );
            if confirmation.changed_files.is_empty() {
                push_wrapped_line(
                    &mut lines,
                    "No tracked files listed.".to_string(),
                    theme.muted_style(),
                    width,
                );
            } else {
                for file in confirmation.changed_files.iter().take(8) {
                    push_wrapped_line(
                        &mut lines,
                        format!("- {}", file),
                        theme.muted_style(),
                        width,
                    );
                }
                if confirmation.changed_files.len() > 8 {
                    push_wrapped_line(
                        &mut lines,
                        format!(
                            "... and {} more file(s)",
                            confirmation.changed_files.len() - 8
                        ),
                        theme.muted_style(),
                        width,
                    );
                }
            }
            if let Some(warning) = &confirmation.warning {
                push_wrapped_line(
                    &mut lines,
                    warning.clone(),
                    Style::default().fg(theme.error),
                    width,
                );
            }
            push_wrapped_line(
                &mut lines,
                "Press Enter to confirm, or Esc to go back.".to_string(),
                Style::default().fg(theme.text),
                width,
            );
        } else {
            push_wrapped_line(
                &mut lines,
                "Use Up/Down and Enter to select a user turn. Esc closes.".to_string(),
                theme.muted_style(),
                width,
            );
            for (index, item) in selector.items.iter().enumerate() {
                let marker = if index == selector.selected {
                    BLACK_CIRCLE
                } else {
                    " "
                };
                let mut text = format!("{} {}", marker, item.preview);
                if item.has_file_changes {
                    text.push_str("  [files]");
                }
                push_wrapped_line(
                    &mut lines,
                    text,
                    Style::default().fg(if index == selector.selected {
                        theme.text
                    } else {
                        theme.muted
                    }),
                    width,
                );
            }
        }
        lines.push(ChatRenderLine::empty());
    }

    if let Some(view) = &state.permissions_view {
        lines.extend(
            render_permissions_view_lines(view, theme)
                .into_iter()
                .map(|line| ChatRenderLine {
                    plain_text: line_to_plain_text(&line),
                    spans: line
                        .spans
                        .iter()
                        .map(|span| (span.content.to_string(), span.style))
                        .collect(),
                }),
        );
    }

    lines
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
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let temp_path = std::env::temp_dir().join(format!(
            "rustcode-clipboard-{}-{}.txt",
            std::process::id(),
            timestamp
        ));
        fs::write(&temp_path, text.as_bytes())?;

        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "param([string]$Path) Get-Content -Raw -Encoding UTF8 -LiteralPath $Path | Set-Clipboard",
                temp_path.to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;

        let _ = fs::remove_file(&temp_path);
        if !status.success() {
            return Err(anyhow::anyhow!(
                "powershell Set-Clipboard exited with status {}",
                status
            ));
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
    } else if state
        .message_selector
        .as_ref()
        .is_some_and(|selector| selector.confirmation.is_some())
    {
        vec![
            Line::from(Span::styled(
                "Rewind confirmation open. Enter confirms; Esc cancels.",
                theme.muted_style(),
            )),
            Line::from(Span::styled(
                "Chat input is paused until the rewind decision is handled.",
                theme.muted_style(),
            )),
        ]
    } else if state.message_selector.is_some() {
        vec![
            Line::from(Span::styled(
                "Message picker open. Select a user turn with Up/Down and Enter.",
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
        let copied = state
            .last_copy_status
            .as_deref()
            .unwrap_or("Selection active.");
        vec![
            Line::from(Span::styled(copied.to_string(), theme.muted_style())),
            Line::from(Span::styled(
                "Esc clears selection. Double-click selects word; triple-click selects line.",
                theme.muted_style(),
            )),
        ]
    } else if state.transcript_mode == TranscriptViewMode::Transcript && state.input.is_empty() {
        vec![
            Line::from(Span::styled(
                "Transcript mode. Up/Down, j/k, PgUp/PgDn scroll. Esc returns.",
                theme.muted_style(),
            )),
            Line::from(Span::styled(
                "v or Ctrl+V toggles verbose transcript details.",
                theme.muted_style(),
            )),
        ]
    } else if state.input.is_empty() {
        vec![
            Line::from(Span::styled("What do you want to do?", theme.muted_style())),
            Line::from(Span::styled(
                "Ctrl+O toggles transcript view. Ctrl+V toggles verbose transcript.",
                theme.muted_style(),
            )),
        ]
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

    if !state.active_tasks.is_empty() && state.pending_approval.is_none() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format_task_progress_summary(&state.active_tasks),
            theme.muted_style(),
        )));
        for task in state.active_tasks.iter().take(2) {
            lines.push(Line::from(Span::styled(
                format!(
                    "#{} [{}] {} ({})",
                    task.id,
                    task.agent_type,
                    task.subject,
                    format_task_progress_status(task.status)
                ),
                theme.muted_style(),
            )));
        }
        if state.active_tasks.len() > 2 {
            lines.push(Line::from(Span::styled(
                format!(
                    "... and {} more running task(s)",
                    state.active_tasks.len() - 2
                ),
                theme.muted_style(),
            )));
        }
    }

    if state.thinking {
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(
                state.spinner_char().to_string(),
                Style::default().fg(theme.brand),
            ),
            Span::styled(" thinking...", theme.muted_style()),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme.prompt_block())
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn render_slash_menu(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    theme: TerminalTheme,
    state: &TerminalState,
    commands: &[SlashCommandSpec],
) {
    if area.height == 0 || !state.slash_menu_visible || commands.is_empty() {
        return;
    }

    let mut lines = vec![Line::from(Span::styled(
        "Slash commands",
        Style::default()
            .fg(theme.brand)
            .add_modifier(Modifier::BOLD),
    ))];
    let visible_count = slash_menu_visible_items_for_area(area.height, commands.len());
    let start = state
        .slash_menu_scroll_offset
        .min(commands.len().saturating_sub(1));
    for (index, command) in commands.iter().skip(start).take(visible_count).enumerate() {
        let actual_index = start + index;
        let selected = actual_index == state.slash_menu_selected;
        let prefix = if selected { "> " } else { "  " };
        let line = format!(
            "{}{}  {}",
            prefix,
            format_command_label(command),
            command.description
        );
        let style = if selected {
            Style::default().fg(theme.panel).bg(theme.brand)
        } else {
            Style::default().fg(theme.text)
        };
        lines.push(Line::from(Span::styled(
            truncate_to_width(&line, area.width.saturating_sub(2) as usize),
            style,
        )));
    }
    if commands.len() > visible_count {
        let end = (start + visible_count).min(commands.len());
        lines.push(Line::from(Span::styled(
            format!("Showing {}-{} of {}", start + 1, end, commands.len()),
            theme.muted_style(),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme.prompt_block().title(" Commands "))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn slash_menu_height(state: &TerminalState, command_count: usize, total_height: u16) -> u16 {
    if !state.slash_menu_visible || command_count == 0 {
        return 0;
    }
    let items = slash_menu_window_size(total_height, command_count) as u16;
    let footer = if command_count > items as usize { 1 } else { 0 };
    1 + items + footer + 2
}

fn slash_menu_window_size(total_height: u16, command_count: usize) -> usize {
    if command_count == 0 {
        return 0;
    }
    let max_allowed = total_height.saturating_sub(10).min(18);
    let inner_height = max_allowed.saturating_sub(2);
    let reserved_lines = 2u16;
    let visible = inner_height.saturating_sub(reserved_lines);
    visible
        .max(1)
        .min(MAX_SLASH_MENU_ITEMS as u16)
        .min(command_count as u16) as usize
}

fn slash_menu_visible_items_for_area(area_height: u16, command_count: usize) -> usize {
    if command_count == 0 || area_height <= 2 {
        return 0;
    }

    let inner_height = area_height.saturating_sub(2);
    let mut visible = inner_height.saturating_sub(1);
    visible = visible.max(1);

    let mut visible = visible
        .min(MAX_SLASH_MENU_ITEMS as u16)
        .min(command_count as u16) as usize;
    if command_count > visible {
        let with_footer = inner_height.saturating_sub(2);
        visible = with_footer
            .max(1)
            .min(MAX_SLASH_MENU_ITEMS as u16)
            .min(command_count as u16) as usize;
    }

    visible.max(1).min(command_count)
}

fn matching_slash_commands(input: &str) -> Vec<SlashCommandSpec> {
    let trimmed = input.trim_start();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Vec::new();
    };
    if rest.contains(char::is_whitespace) && !rest.ends_with(' ') {
        return Vec::new();
    }
    let query = rest
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    SlashCommandRegistry::load(&cwd)
        .all()
        .iter()
        .filter(|command| query.is_empty() || command.name.starts_with(&query))
        .cloned()
        .collect()
}

fn format_command_label(command: &SlashCommandSpec) -> String {
    match &command.argument_hint {
        Some(hint) => format!("/{} {}", command.name, hint),
        None => format!("/{}", command.name),
    }
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut output = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width + 1 > max_width {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output.push('…');
    output
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

fn bounded_argument_preview(arguments: &serde_json::Value, max_lines: usize) -> String {
    let preview = format_arguments_preview(arguments);
    let lines = preview.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let mut bounded = lines
        .iter()
        .take(max_lines)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > max_lines {
        bounded.push_str(&format!(
            "\n... {} more argument line(s)",
            lines.len() - max_lines
        ));
    }
    format!("\nArgs: {}", bounded)
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

    spans.push(Span::styled("  ∙  ", theme.muted_style()));
    spans.push(Span::styled(
        match state.transcript_mode {
            TranscriptViewMode::Main => "main",
            TranscriptViewMode::Transcript => {
                if state.verbose_transcript {
                    "transcript+verbose"
                } else {
                    "transcript"
                }
            }
        },
        theme.muted_style(),
    ));

    if !state.active_tasks.is_empty() {
        spans.push(Span::styled("  ∙  ", theme.muted_style()));
        spans.push(Span::styled(
            format_task_progress_summary(&state.active_tasks),
            theme.muted_style(),
        ));
    }

    if !state.thinking && !state.status.is_empty() {
        spans.push(Span::styled("  ∙  ", theme.muted_style()));
        spans.push(Span::styled(state.status.clone(), theme.muted_style()));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

fn format_task_progress_summary(tasks: &[TaskProgressItem]) -> String {
    let running = tasks
        .iter()
        .filter(|task| matches!(task.status, crate::agents_runtime::AgentTaskStatus::Running))
        .count();
    let awaiting_approval = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                crate::agents_runtime::AgentTaskStatus::AwaitingApproval
            )
        })
        .count();
    let pending = tasks
        .len()
        .saturating_sub(running)
        .saturating_sub(awaiting_approval);
    match (running, pending, awaiting_approval) {
        (r, 0, 0) => format!("{r} subagent(s) running"),
        (0, p, 0) => format!("{p} subagent(s) queued"),
        (0, 0, a) => format!("{a} awaiting approval"),
        (r, p, 0) => format!("{r} running, {p} queued"),
        (r, 0, a) => format!("{r} running, {a} awaiting approval"),
        (0, p, a) => format!("{p} queued, {a} awaiting approval"),
        (r, p, a) => format!("{r} running, {p} queued, {a} awaiting approval"),
    }
}

fn format_task_progress_status(status: crate::agents_runtime::AgentTaskStatus) -> &'static str {
    match status {
        crate::agents_runtime::AgentTaskStatus::Pending => "queued",
        crate::agents_runtime::AgentTaskStatus::Running => "running",
        crate::agents_runtime::AgentTaskStatus::AwaitingApproval => "awaiting approval",
        crate::agents_runtime::AgentTaskStatus::Completed => "completed",
        crate::agents_runtime::AgentTaskStatus::Failed => "failed",
        crate::agents_runtime::AgentTaskStatus::Cancelled => "cancelled",
    }
}

fn format_file_rewind_summary(result: &crate::file_history::FileRewindResult) -> String {
    match (
        result.restored_files.is_empty(),
        result.deleted_files.is_empty(),
    ) {
        (true, true) => "No tracked file changes needed to be rewound.".to_string(),
        (false, true) => format!("Rewound files: {}", result.restored_files.join(", ")),
        (true, false) => format!(
            "Deleted files created after target: {}",
            result.deleted_files.join(", ")
        ),
        (false, false) => format!(
            "Rewound files: {}. Deleted new files: {}",
            result.restored_files.join(", "),
            result.deleted_files.join(", ")
        ),
    }
}

fn resolve_user_message_id(session: &crate::session::Session, raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    if matches!(trimmed, "last-user" | "latest-user") {
        return session
            .messages
            .iter()
            .rev()
            .find(|message| message.role.eq_ignore_ascii_case("user"))
            .map(|message| message.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No user messages available in this session"));
    }
    let mut exact_match = None;
    let mut prefix_matches = Vec::new();
    for message in session
        .messages
        .iter()
        .filter(|message| message.role.eq_ignore_ascii_case("user"))
    {
        if message.id == trimmed {
            exact_match = Some(message.id.clone());
            break;
        }
        if message.id.starts_with(trimmed) {
            prefix_matches.push(message.id.clone());
        }
    }
    if let Some(exact_match) = exact_match {
        return Ok(exact_match);
    }
    match prefix_matches.len() {
        1 => Ok(prefix_matches.remove(0)),
        0 => Err(anyhow::anyhow!(
            "No user message found for id or prefix: {}",
            trimmed
        )),
        _ => Err(anyhow::anyhow!("Message prefix is ambiguous: {}", trimmed)),
    }
}

fn render_resume_session_line(
    session: &SessionInfo,
    selected: bool,
    theme: TerminalTheme,
) -> Line<'static> {
    let marker = if selected { BLACK_CIRCLE } else { " " };
    let status = format!("{:?}", session.status).to_ascii_lowercase();
    let kind = match session.session_kind {
        SessionKind::Primary => "primary",
        SessionKind::Forked => "forked",
        SessionKind::ChildAgent => "child",
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", marker),
            Style::default().fg(if selected { theme.brand } else { theme.muted }),
        ),
        Span::styled(session.name.clone(), Style::default().fg(theme.text)),
        Span::styled("  ", theme.muted_style()),
        Span::styled(session.id.clone(), theme.muted_style()),
        Span::styled("  ", theme.muted_style()),
        Span::styled(kind, Style::default().fg(theme.subtle)),
        Span::styled("  ", theme.muted_style()),
        Span::styled(status, Style::default().fg(theme.subtle)),
        Span::styled(
            session
                .latest_user_summary
                .as_deref()
                .map(|summary| format!("  {}", summary))
                .unwrap_or_default(),
            theme.muted_style(),
        ),
        Span::styled(
            session
                .forked_from_session_id
                .as_deref()
                .map(|id| format!("  from {}", short_message_id(id)))
                .unwrap_or_default(),
            theme.muted_style(),
        ),
        Span::styled(
            session
                .spawned_by_task_id
                .as_deref()
                .map(|id| format!("  task {}", short_task_id(id)))
                .unwrap_or_default(),
            theme.muted_style(),
        ),
    ])
}

fn summarize_selector_preview(content: &str) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview = normalized.chars().take(80).collect::<String>();
    if normalized.chars().count() <= 80 {
        normalized
    } else {
        format!("{}...", preview)
    }
}

fn short_message_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn short_task_id(id: &str) -> String {
    id.chars().take(8).collect()
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
        ApiProtocol::Anthropic => ApiProtocol::Responses,
        ApiProtocol::Responses => ApiProtocol::OpenAi,
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
    session_id: Option<String>,
) -> mpsc::Receiver<ChatWorkerUpdate> {
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
                let sender = sender.clone();
                let mut progress = move |event| {
                    let _ = sender.send(ChatWorkerUpdate::Progress(event));
                };
                engine
                    .submit_message_with_context_and_progress(
                        &base_history,
                        user_message,
                        session_id.clone(),
                        &mut progress,
                    )
                    .await
            });

            match result {
                Ok(turn) => Ok(ChatWorkerResult {
                    outcome: Ok(ChatWorkerOutcome::Turn(turn)),
                }),
                Err(error) => {
                    let _ = fallback_user_message;
                    Err(error)
                }
            }
        })()
        .unwrap_or_else(|error| {
            let _ = original_user_message;
            ChatWorkerResult {
                outcome: Err(error),
            }
        });
        let _ = sender.send(ChatWorkerUpdate::Finished(payload));
    });
    receiver
}

fn spawn_plan_request(
    settings: Settings,
    base_history: Arc<Vec<RuntimeMessage>>,
    prompt: String,
    session_id: Option<String>,
) -> mpsc::Receiver<ChatWorkerUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let plan_prompt = prompt.clone();
        let payload = (|| -> anyhow::Result<ChatWorkerResult> {
            let Some(agent) = AgentsService::builtin_definition_by_name("plan") else {
                return Err(anyhow::anyhow!("Plan agent is not available"));
            };

            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(run_agent_with_parent_history(
                settings.clone(),
                Some(settings.working_dir.clone()),
                agent,
                &base_history,
                plan_prompt.clone(),
                session_id.clone(),
            ))?;

            let mut history = (*base_history).clone();
            history.push(RuntimeMessage::user(plan_prompt));
            history.push(RuntimeMessage::assistant(result.clone()));

            Ok(ChatWorkerResult {
                outcome: Ok(ChatWorkerOutcome::Turn(QueryTurnResult {
                    history,
                    assistant_message: Some(RuntimeMessage::assistant(result)),
                    usage: None,
                    model: "plan".to_string(),
                    finish_reason: None,
                    tool_call_count: 0,
                    status: TurnStatus::Completed,
                    pending_approval: None,
                    was_compacted: false,
                    compaction_summary: None,
                })),
            })
        })()
        .unwrap_or_else(|error| ChatWorkerResult {
            outcome: Err(error),
        });
        let _ = sender.send(ChatWorkerUpdate::Finished(payload));
    });
    receiver
}

fn spawn_approval_request(
    settings: Settings,
    base_history: Arc<Vec<RuntimeMessage>>,
    action: ApprovalAction,
    session_id: Option<String>,
) -> mpsc::Receiver<ChatWorkerUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let payload = (|| -> anyhow::Result<ChatWorkerResult> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(async {
                let engine = QueryEngine::new(settings);
                let sender = sender.clone();
                let mut progress = move |event| {
                    let _ = sender.send(ChatWorkerUpdate::Progress(event));
                };
                engine
                    .resume_after_approval_with_context_and_progress(
                        &base_history,
                        action,
                        session_id.clone(),
                        &mut progress,
                    )
                    .await
            });

            match result {
                Ok(turn) => Ok(ChatWorkerResult {
                    outcome: Ok(ChatWorkerOutcome::Turn(turn)),
                }),
                Err(error) => Err(error),
            }
        })()
        .unwrap_or_else(|error| ChatWorkerResult {
            outcome: Err(error),
        });
        let _ = sender.send(ChatWorkerUpdate::Finished(payload));
    });
    receiver
}

fn spawn_task_approval_request(
    settings: Settings,
    task_id: String,
    action: ApprovalAction,
) -> mpsc::Receiver<ChatWorkerUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let task_id_for_worker = task_id.clone();
        let payload = (|| -> anyhow::Result<ChatWorkerResult> {
            resume_agent_task_after_approval(
                settings.clone(),
                Some(settings.working_dir.clone()),
                task_id.clone(),
                action,
            )?;
            Ok(ChatWorkerResult {
                outcome: Ok(ChatWorkerOutcome::TaskResume {
                    message: format!("Child task resumed after approval: #{}", task_id_for_worker),
                }),
            })
        })()
        .unwrap_or_else(|error| ChatWorkerResult {
            outcome: Err(error),
        });
        let _ = sender.send(ChatWorkerUpdate::Finished(payload));
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

fn parse_resume_query(input: &str) -> SessionQuery {
    let mut kind = None;
    let mut text_tokens = Vec::new();

    for token in input.split_whitespace() {
        if let Some(value) = token.strip_prefix("kind:") {
            kind = SessionKind::parse_filter(value).or(kind);
            continue;
        }

        text_tokens.push(token);
    }

    let text = (!text_tokens.is_empty()).then(|| text_tokens.join(" "));

    SessionQuery {
        text,
        kind,
        limit: Some(20),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::state::TextSelection;

    #[test]
    fn wrap_plain_line_splits_at_width() {
        assert_eq!(
            crate::terminal::markdown::wrap_plain_line("abcdef", 2),
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

    #[test]
    fn task_progress_summary_prefers_running_count() {
        let tasks = vec![
            TaskProgressItem {
                id: "1".to_string(),
                subject: "inspect".to_string(),
                agent_type: "explore".to_string(),
                status: crate::agents_runtime::AgentTaskStatus::Running,
            },
            TaskProgressItem {
                id: "2".to_string(),
                subject: "verify".to_string(),
                agent_type: "verification".to_string(),
                status: crate::agents_runtime::AgentTaskStatus::Pending,
            },
        ];

        assert_eq!(format_task_progress_summary(&tasks), "1 running, 1 queued");
    }

    #[test]
    fn task_progress_status_formats_pending_as_queued() {
        assert_eq!(
            format_task_progress_status(crate::agents_runtime::AgentTaskStatus::Pending),
            "queued"
        );
    }

    #[test]
    fn clamp_scroll_offset_caps_large_home_offset() {
        assert_eq!(clamp_scroll_offset(120, 20, usize::MAX / 4), 100);
        assert_eq!(clamp_scroll_offset(10, 20, 8), 0);
    }

    #[test]
    fn task_progress_summary_prioritizes_awaiting_approval() {
        let tasks = vec![
            TaskProgressItem {
                id: "1".to_string(),
                subject: "inspect".to_string(),
                agent_type: "explore".to_string(),
                status: crate::agents_runtime::AgentTaskStatus::AwaitingApproval,
            },
            TaskProgressItem {
                id: "2".to_string(),
                subject: "verify".to_string(),
                agent_type: "verification".to_string(),
                status: crate::agents_runtime::AgentTaskStatus::Running,
            },
        ];

        assert_eq!(
            format_task_progress_summary(&tasks),
            "1 running, 1 awaiting approval"
        );
    }

    #[test]
    fn parse_resume_query_extracts_kind_filter() {
        let query = parse_resume_query("kind:branch android build");

        assert_eq!(query.kind, Some(SessionKind::Forked));
        assert_eq!(query.text.as_deref(), Some("android build"));
        assert_eq!(query.limit, Some(20));
    }

    #[test]
    fn slash_menu_visible_items_uses_actual_area_height() {
        assert_eq!(slash_menu_visible_items_for_area(18, 21), 8);
    }

    #[test]
    fn truncate_to_width_keeps_single_line_with_ellipsis() {
        assert_eq!(
            truncate_to_width("/plan [open|<description>] enable plan mode", 20),
            "/plan [open|<descri…"
        );
    }
}
