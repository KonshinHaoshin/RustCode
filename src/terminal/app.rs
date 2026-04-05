use super::{
    state::{
        DisplayMessage, DisplayRole, FallbackField, OnboardingStep, PrimaryField, TerminalState,
        ViewMode,
    },
    theme::{TerminalTheme, BLACK_CIRCLE, GUTTER},
};
use crate::{
    api::{ApiClient, ChatMessage},
    config::{ApiProtocol, ApiProvider, FallbackTarget, Settings},
    onboarding::OnboardingDraft,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind},
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
    sync::mpsc::{self, Receiver},
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
        loop {
            self.poll_pending_response();
            self.state.tick_spinner();
            self.state.clear_exit_confirmation_if_stale();

            if let Some(prompt) = self.state.consume_initial_prompt() {
                self.state.input = prompt;
                self.submit_prompt();
            }

            self.draw()?;

            if self.state.should_quit {
                break;
            }

            if event::poll(Duration::from_millis(80))? {
                self.handle_event(event::read()?)?;
            }
        }

        Ok(())
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        let theme = self.theme;
        let state = &self.state;

        self.terminal.draw(|frame| match state.view {
            ViewMode::Onboarding => draw_onboarding_view(frame, theme, state),
            ViewMode::Chat => draw_chat_view(frame, theme, state),
        })?;

        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return Ok(());
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    self.state.request_exit();
                    return Ok(());
                }
                match self.state.view {
                    ViewMode::Chat => self.handle_chat_key(key),
                    ViewMode::Onboarding => self.handle_onboarding_key(key)?,
                }
            }
            Event::Mouse(mouse) => {
                if self.state.view == ViewMode::Chat {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            self.state.scroll_offset =
                                self.state.scroll_offset.saturating_add(3);
                        }
                        MouseEventKind::ScrollUp => {
                            self.state.scroll_offset =
                                self.state.scroll_offset.saturating_sub(3);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_chat_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.state.input.push('\n');
                } else {
                    self.submit_prompt();
                }
            }
            KeyCode::Backspace => {
                self.state.input.pop();
            }
            KeyCode::Tab => {
                self.state.view = ViewMode::Onboarding;
                self.state.onboarding_step = OnboardingStep::Summary;
                self.state.status = "Opened configuration summary.".to_string();
            }
            KeyCode::Esc => {
                self.state.input.clear();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.input.push(ch);
            }
            _ => {}
        }
    }

    fn handle_onboarding_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
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

        Ok(())
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

    fn submit_prompt(&mut self) {
        if self.state.thinking {
            return;
        }
        let prompt = self.state.input.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        self.state.messages.push(DisplayMessage {
            role: DisplayRole::User,
            content: prompt.clone(),
        });
        self.state
            .conversation_history
            .push(ChatMessage::user(prompt));
        self.state.input.clear();
        self.state.pending_response = Some(spawn_chat_request(
            self.state.settings.clone(),
            self.state.conversation_history.clone(),
        ));
        self.state.thinking = true;
        self.state.status = format!(
            "Querying {}/{}",
            self.state.settings.api.provider_label(),
            self.state.settings.model
        );
    }

    fn poll_pending_response(&mut self) {
        let Some(receiver) = self.state.pending_response.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(content)) => {
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::Assistant,
                    content: content.clone(),
                });
                self.state
                    .conversation_history
                    .push(ChatMessage::assistant(content));
                self.state.thinking = false;
                self.state.status = "Response received.".to_string();
                self.state.scroll_offset = 0; // snap to bottom on new message
            }
            Ok(Err(error)) => {
                self.state.messages.push(DisplayMessage {
                    role: DisplayRole::System,
                    content: format!("Request failed: {}", error),
                });
                self.state.thinking = false;
                self.state.status = "Request failed.".to_string();
            }
            Err(mpsc::TryRecvError::Empty) => self.state.pending_response = Some(receiver),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.state.thinking = false;
                self.state.status = "Request worker disconnected.".to_string();
            }
        }
    }
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
        Paragraph::new(theme.welcome_lines(chunks[0].width))
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[0],
    );
    render_onboarding(frame, chunks[1], theme, state);
    render_status_line(frame, chunks[2], theme, state);
}

fn draw_chat_view(frame: &mut ratatui::Frame<'_>, theme: TerminalTheme, state: &TerminalState) {
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
    state: &TerminalState,
) {
    let transcript = render_chat_lines(state, theme, area.width);
    let total_lines = transcript.len() as u16;
    let visible = area.height;

    // scroll_offset == 0 means "pinned to bottom"
    // positive offset scrolls up (shows older content)
    let max_scroll = total_lines.saturating_sub(visible);
    let scroll_up = state.scroll_offset as u16;
    let scroll_row = max_scroll.saturating_sub(scroll_up);

    frame.render_widget(
        Paragraph::new(transcript)
            .alignment(Alignment::Left)
            .scroll((scroll_row, 0))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn render_chat_lines(
    state: &TerminalState,
    theme: TerminalTheme,
    width: u16,
) -> Vec<Line<'static>> {
    if state.messages.is_empty() {
        return theme.empty_chat_lines(width);
    }

    let mut lines = Vec::new();

    for message in &state.messages {
        match message.role {
            DisplayRole::User => {
                for content_line in message.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!(" {}", content_line),
                        Style::default()
                            .fg(theme.text)
                            .bg(theme.user_msg_bg),
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
                        Span::styled(
                            format!("{} ", GUTTER),
                            Style::default().fg(theme.subtle),
                        ),
                        Span::styled(
                            content_line.to_string(),
                            Style::default().fg(theme.text),
                        ),
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
                        Span::styled(
                            content_line.to_string(),
                            Style::default().fg(theme.error),
                        ),
                    ]));
                }
            }
        }
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
    let mut lines: Vec<Line<'static>> = if state.input.is_empty() {
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
        Span::styled(
            format!("{} ", BLACK_CIRCLE),
            Style::default().fg(dot_color),
        ),
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
            Span::styled(
                format!("{label}: "),
                Style::default().fg(theme.brand),
            ),
            Span::styled(value, Style::default().fg(theme.text)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{label}: "),
                Style::default().fg(theme.muted),
            ),
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
    messages: Vec<ChatMessage>,
) -> Receiver<anyhow::Result<String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result: anyhow::Result<String> = (|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let client = ApiClient::new(settings);
                let response = client.chat(messages).await?;
                Ok(response
                    .choices
                    .first()
                    .map(|choice| choice.message.content.clone())
                    .unwrap_or_default())
            })
        })();
        let _ = sender.send(result);
    });
    receiver
}
