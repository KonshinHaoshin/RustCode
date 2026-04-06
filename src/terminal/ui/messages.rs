use crate::{
    compact::is_compact_summary_content,
    terminal::{
        markdown::{render_assistant_markdown, wrap_plain_line, ChatRenderLine},
        state::{DisplayMessage, DisplayRole, TranscriptViewMode},
        theme::{TerminalTheme, BLACK_CIRCLE, GUTTER},
        ui::render_model::{compressed_thinking_label, RenderBlock},
    },
};
use ratatui::style::{Modifier, Style};

pub(crate) fn render_render_block(
    lines: &mut Vec<ChatRenderLine>,
    visible_messages: &[DisplayMessage],
    last_thinking_index: Option<usize>,
    transcript_mode: TranscriptViewMode,
    verbose_transcript: bool,
    theme: TerminalTheme,
    width: u16,
    block: RenderBlock,
    thinking_preview_chars: usize,
) {
    match block {
        RenderBlock::Message(message) => {
            render_single_message(
                lines,
                visible_messages,
                last_thinking_index,
                transcript_mode,
                theme,
                width,
                &message,
                thinking_preview_chars,
            );
        }
        RenderBlock::GroupedToolUse {
            tool_name,
            count,
            detail,
        } => {
            render_grouped_tool_use_summary(lines, theme, width, &tool_name, count);
            if transcript_mode == TranscriptViewMode::Transcript && verbose_transcript {
                render_tool_group_detail(lines, theme, width, detail);
            }
        }
        RenderBlock::CollapsedReadSearch { summary, detail } => {
            render_collapsed_read_search_summary(lines, theme, width, &summary);
            if transcript_mode == TranscriptViewMode::Transcript && verbose_transcript {
                render_tool_group_detail(lines, theme, width, detail);
            }
        }
    }
}

fn render_single_message(
    lines: &mut Vec<ChatRenderLine>,
    visible_messages: &[DisplayMessage],
    last_thinking_index: Option<usize>,
    transcript_mode: TranscriptViewMode,
    theme: TerminalTheme,
    width: u16,
    message: &DisplayMessage,
    thinking_preview_chars: usize,
) {
    match message.role {
        DisplayRole::User => render_user_message(lines, theme, width, message),
        DisplayRole::Assistant => render_assistant_message(lines, theme, width, message),
        DisplayRole::Thinking => {
            let visible_idx = visible_messages
                .iter()
                .position(|candidate| {
                    candidate.role == message.role && candidate.content == message.content
                })
                .unwrap_or_default();
            if Some(visible_idx) != last_thinking_index {
                return;
            }
            if transcript_mode == TranscriptViewMode::Transcript {
                render_full_thinking(lines, theme, width, message);
            } else {
                push_wrapped_line(
                    lines,
                    compressed_thinking_label(&message.content, thinking_preview_chars),
                    Style::default().fg(theme.muted),
                    width,
                );
            }
        }
        DisplayRole::System => {
            if is_compact_summary_content(&message.content) {
                push_wrapped_line(
                    lines,
                    format!("{} Earlier conversation compacted.", BLACK_CIRCLE),
                    Style::default().fg(theme.muted),
                    width,
                );
            } else {
                for content_line in message.content.lines() {
                    push_wrapped_line(
                        lines,
                        format!("{} {}", BLACK_CIRCLE, content_line),
                        Style::default().fg(theme.error),
                        width,
                    );
                }
            }
        }
        DisplayRole::Tool => render_tool_message(lines, theme, width, message),
    }
}

fn render_grouped_tool_use_summary(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    tool_name: &str,
    count: usize,
) {
    push_wrapped_line(
        lines,
        format!("{} {} x{}", GUTTER, tool_name, count),
        Style::default().fg(theme.muted),
        width,
    );
}

fn render_collapsed_read_search_summary(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    summary: &str,
) {
    push_wrapped_line(
        lines,
        format!("{} {}", GUTTER, summary),
        Style::default().fg(theme.muted),
        width,
    );
}

fn render_tool_group_detail(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    detail: Vec<DisplayMessage>,
) {
    for message in detail {
        render_tool_message(lines, theme, width, &message);
    }
}

fn render_user_message(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    message: &DisplayMessage,
) {
    for content_line in message.content.lines() {
        push_wrapped_line(
            lines,
            format!(" {}", content_line),
            Style::default().fg(theme.text).bg(theme.user_msg_bg),
            width,
        );
    }
    if message.content.is_empty() {
        push_wrapped_line(
            lines,
            " ".to_string(),
            Style::default().bg(theme.user_msg_bg),
            width,
        );
    }
}

fn render_assistant_message(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    message: &DisplayMessage,
) {
    lines.extend(render_assistant_markdown(&message.content, theme, width));
}

fn render_full_thinking(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    message: &DisplayMessage,
) {
    push_wrapped_line(
        lines,
        "∴ Thinking".to_string(),
        Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::ITALIC),
        width,
    );
    for content_line in message.content.lines() {
        push_wrapped_line(
            lines,
            format!("  {}", content_line),
            Style::default().fg(theme.muted),
            width,
        );
    }
}

fn render_tool_message(
    lines: &mut Vec<ChatRenderLine>,
    theme: TerminalTheme,
    width: u16,
    message: &DisplayMessage,
) {
    for content_line in message.content.lines() {
        push_wrapped_line(
            lines,
            format!("{} {}", GUTTER, content_line),
            Style::default().fg(theme.muted),
            width,
        );
    }
}

pub(crate) fn push_wrapped_line(
    lines: &mut Vec<ChatRenderLine>,
    text: String,
    style: Style,
    width: u16,
) {
    for wrapped in wrap_plain_line(&text, width.max(1) as usize) {
        lines.push(ChatRenderLine::plain(wrapped, style));
    }
}
