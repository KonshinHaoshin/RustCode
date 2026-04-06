use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChatScreenLayout {
    pub transcript: Rect,
    pub prompt: Rect,
    pub status: Rect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OnboardingScreenLayout {
    pub hero: Rect,
    pub body: Rect,
    pub status: Rect,
}

pub(crate) fn split_chat_screen(area: Rect) -> ChatScreenLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    ChatScreenLayout {
        transcript: chunks[0],
        prompt: chunks[1],
        status: chunks[2],
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
