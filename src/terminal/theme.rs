use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

#[derive(Debug, Clone, Copy)]
pub struct TerminalTheme {
    pub brand: Color,
    pub accent: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub panel: Color,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            brand: Color::Rgb(222, 119, 55),
            accent: Color::Rgb(245, 198, 110),
            text: Color::Rgb(230, 228, 220),
            muted: Color::Rgb(145, 145, 140),
            border: Color::Rgb(82, 78, 72),
            panel: Color::Rgb(28, 29, 31),
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

    pub fn bordered_block(self) -> ratatui::widgets::Block<'static> {
        use ratatui::widgets::{Block, Borders};

        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.border))
            .style(Style::default().bg(self.panel))
    }

    pub fn welcome_lines(self, width: u16) -> Vec<Line<'static>> {
        let full = [
            Line::from(vec![
                Span::styled("Welcome to RustCode ", self.title_style()),
                Span::styled(
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                    self.muted_style(),
                ),
            ]),
            Line::from(Span::styled(
                "..........................................................",
                Style::default().fg(self.muted),
            )),
            Line::from(Span::styled(
                "      *                                        █████▓▓░     ",
                Style::default().fg(self.muted),
            )),
            Line::from(Span::styled(
                "            ░░░░░░                         ███▓░            ",
                Style::default().fg(self.muted),
            )),
            Line::from(vec![
                Span::styled("      ", Style::default()),
                Span::styled(
                    "█████████",
                    Style::default()
                        .fg(self.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "     *                                   ",
                    Style::default().fg(self.muted),
                ),
            ]),
            Line::from(vec![
                Span::styled("      ", Style::default()),
                Span::styled(
                    "██▄█████▄██",
                    Style::default().fg(self.brand).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "                        *                ",
                    Style::default().fg(self.muted),
                ),
            ]),
            Line::from(vec![
                Span::styled("      ", Style::default()),
                Span::styled(
                    "█████████",
                    Style::default()
                        .fg(self.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("                                       ", Style::default()),
            ]),
        ];

        if width >= 72 {
            full.to_vec()
        } else {
            vec![
                Line::from(vec![
                    Span::styled("Welcome to RustCode ", self.title_style()),
                    Span::styled(
                        format!("v{}", env!("CARGO_PKG_VERSION")),
                        self.muted_style(),
                    ),
                ]),
                Line::from(Span::styled(
                    "Multi-provider coding assistant with fallback-aware setup.",
                    self.muted_style(),
                )),
            ]
        }
    }
}
