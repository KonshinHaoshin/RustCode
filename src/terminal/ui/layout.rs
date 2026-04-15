use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChatScreenLayout {
    pub transcript: Rect,
    pub mascot: Rect,
    pub slash_menu: Rect,
    pub prompt: Rect,
    pub status: Rect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OnboardingScreenLayout {
    pub hero: Rect,
    pub body: Rect,
    pub status: Rect,
}

pub(crate) fn split_chat_screen(area: Rect, slash_menu_height: u16) -> ChatScreenLayout {
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(slash_menu_height),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    let top_area = vertical_chunks[0];
    let top_horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(40),
            Constraint::Length(if area.width > 80 { 12 } else { 0 }),
        ])
        .split(top_area);

    ChatScreenLayout {
        transcript: top_horizontal_chunks[0],
        mascot: top_horizontal_chunks[1],
        slash_menu: vertical_chunks[1],
        prompt: vertical_chunks[2],
        status: vertical_chunks[3],
    }
}

pub(crate) fn split_onboarding_screen(area: Rect) -> OnboardingScreenLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(area);

    OnboardingScreenLayout {
        hero: chunks[0],
        body: chunks[1],
        status: chunks[2],
    }
}
