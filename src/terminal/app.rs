use super::{
    state::{
        DisplayMessage, DisplayRole, FallbackField, OnboardingStep, PrimaryField, TerminalState,
        ViewMode,
    },
    theme::TerminalTheme,
};
use crate::{
    api::{ApiClient, ChatMessage},
    config::{ApiProtocol, ApiProvider, FallbackTarget, Settings},
    onboarding::OnboardingDraft,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
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
        execute!(stdout, EnterAlternateScreen)?;
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

        self.terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(if frame.size().width >= 72 { 9 } else { 5 }),
                    Constraint::Min(10),
                    Constraint::Length(4),
                ])
                .split(frame.size());

            let header = Paragraph::new(theme.welcome_lines(chunks[0].width))
                .block(theme.bordered_block().title(" RustCode "))
                .wrap(ratatui::widgets::Wrap { trim: false });
            frame.render_widget(header, chunks[0]);

            match state.view {
                ViewMode::Onboarding => render_onboarding(frame, chunks[1], theme, state),
                ViewMode::Chat => render_chat(frame, chunks[1], theme, state),
            }

            frame.render_widget(
                Paragraph::new(state.status.as_str())
                    .alignment(Alignment::Left)
                    .block(theme.bordered_block().title(" Status ")),
                chunks[2],
            );
        })?;

        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> anyhow::Result<()> {
        if let Event::Key(key) = event {
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
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
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
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))))
            .collect(),
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme.bordered_block().title(" Onboarding "))
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(6)])
        .split(area);

    let transcript = render_chat_lines(state, theme);
    frame.render_widget(
        Paragraph::new(transcript)
            .block(theme.bordered_block().title(" Conversation "))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(state.input.as_str())
            .block(theme.bordered_block().title(if state.thinking {
                " Input [thinking] "
            } else {
                " Input "
            }))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[1],
    );
}

fn render_chat_lines(state: &TerminalState, theme: TerminalTheme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in &state.messages {
        let (label, color) = match message.role {
            DisplayRole::User => ("You", theme.accent),
            DisplayRole::Assistant => ("FerrisCode", theme.brand),
            DisplayRole::System => ("System", theme.muted),
        };

        lines.push(Line::from(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));

        for content_line in message.content.lines() {
            lines.push(Line::from(Span::styled(
                content_line.to_string(),
                Style::default().fg(theme.text),
            )));
        }

        if message.content.is_empty() {
            lines.push(Line::from(""));
        }

        lines.push(Line::default());
    }

    lines
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
            let marker = if index == state.selected_fallback {
                "> "
            } else {
                "  "
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "{}{}",
                    marker,
                    OnboardingDraft::fallback_target_label(target)
                ),
                Style::default().fg(if index == state.selected_fallback {
                    theme.accent
                } else {
                    theme.text
                }),
            )));
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
    let prefix = if focused { "> " } else { "  " };
    Line::from(vec![
        Span::styled(
            prefix.to_string(),
            Style::default().fg(if focused { theme.accent } else { theme.muted }),
        ),
        Span::styled(
            format!("{label}: "),
            Style::default().fg(if focused { theme.accent } else { theme.muted }),
        ),
        Span::styled(value, Style::default().fg(theme.text)),
    ])
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
        None => "env or empty".to_string(),
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
