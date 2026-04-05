use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub const GUTTER: &str = "⎿";
pub const BLACK_CIRCLE: &str = "●";
#[allow(dead_code)]
pub const DIVIDER_CHAR: char = '─';
pub const SPINNER_FRAMES: &[char] = &['·', '✢', '✳', '✶', '✻', '✽'];

#[derive(Debug, Clone, Copy)]
pub struct TerminalTheme {
    pub brand: Color,
    pub shimmer: Color,
    pub text: Color,
    pub muted: Color,
    pub subtle: Color,
    pub border: Color,
    #[allow(dead_code)]
    pub panel: Color,
    pub user_msg_bg: Color,
    pub success: Color,
    pub error: Color,
    #[allow(dead_code)]
    pub warning: Color,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            brand: Color::Rgb(215, 119, 87),
            shimmer: Color::Rgb(235, 159, 127),
            text: Color::Rgb(255, 255, 255),
            muted: Color::Rgb(153, 153, 153),
            subtle: Color::Rgb(80, 80, 80),
            border: Color::Rgb(136, 136, 136),
            panel: Color::Black,
            user_msg_bg: Color::Rgb(55, 55, 55),
            success: Color::Rgb(78, 186, 101),
            error: Color::Rgb(255, 107, 128),
            warning: Color::Rgb(255, 193, 7),
        }
    }
}

impl TerminalTheme {
    pub fn title_style(self) -> Style {
        Style::default().fg(self.brand).add_modifier(Modifier::BOLD)
    }

    pub fn muted_style(self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn prompt_block(self) -> ratatui::widgets::Block<'static> {
        use ratatui::widgets::{Block, Borders};

        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.border))
    }

    pub fn welcome_lines(self, _width: u16) -> Vec<Line<'static>> {
        let brand_style = Style::default().fg(self.brand);
        let subtle_style = Style::default().fg(self.subtle);

        vec![
            Line::from(Span::styled("  ▄█████▄  ", brand_style)),
            Line::from(vec![
                Span::styled(" █▀ ", brand_style),
                Span::styled("▄▄", subtle_style),
                Span::styled(" ▀█ ", brand_style),
            ]),
            Line::from(Span::styled("  ▀█████▀  ", brand_style)),
            Line::from(Span::styled("   ╹╹ ╹╹   ", brand_style)),
            Line::default(),
            Line::from(vec![
                Span::styled("Welcome to RustCode ", self.title_style()),
                Span::styled(
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                    self.muted_style(),
                ),
            ]),
            Line::from(Span::styled(
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "~".to_string()),
                self.muted_style(),
            )),
            Line::from(Span::styled(
                "─────────────────────────────────────────",
                Style::default().fg(self.subtle),
            )),
        ]
    }

    pub fn empty_chat_lines(self, width: u16) -> Vec<Line<'static>> {
        let mut lines = self.welcome_lines(width);
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Start a conversation or press Tab to configure providers.",
            self.muted_style(),
        )));
        lines
    }
}
