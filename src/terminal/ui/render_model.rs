use crate::terminal::state::{DisplayMessage, DisplayRole};

#[derive(Clone)]
pub(crate) enum RenderBlock {
    Message(DisplayMessage),
    GroupedToolUse {
        tool_name: String,
        count: usize,
        detail: Vec<DisplayMessage>,
    },
    CollapsedReadSearch {
        summary: String,
        detail: Vec<DisplayMessage>,
    },
}

#[derive(Default)]
struct ToolRunSummary {
    read_count: usize,
    search_count: usize,
    list_count: usize,
    bash_count: usize,
    other_name: Option<String>,
    other_count: usize,
}

enum ToolRunRender {
    GroupedToolUse { tool_name: String, count: usize },
    CollapsedReadSearch { summary: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolClass {
    Read,
    Search,
    List,
    Bash,
    Other,
}

pub(crate) fn build_render_blocks(messages: &[DisplayMessage]) -> Vec<RenderBlock> {
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < messages.len() {
        let message = &messages[index];
        if message.role != DisplayRole::Tool {
            blocks.push(RenderBlock::Message(message.clone()));
            index += 1;
            continue;
        }

        let mut run_end = index;
        while run_end < messages.len() && messages[run_end].role == DisplayRole::Tool {
            run_end += 1;
        }

        let tool_run = &messages[index..run_end];
        match summarize_tool_run(tool_run) {
            Some(ToolRunRender::CollapsedReadSearch { summary }) => {
                blocks.push(RenderBlock::CollapsedReadSearch {
                    summary,
                    detail: tool_run.to_vec(),
                });
            }
            Some(ToolRunRender::GroupedToolUse { tool_name, count }) => {
                blocks.push(RenderBlock::GroupedToolUse {
                    tool_name,
                    count,
                    detail: tool_run.to_vec(),
                });
            }
            None => {
                for item in tool_run {
                    blocks.push(RenderBlock::Message(item.clone()));
                }
            }
        }
        index = run_end;
    }

    blocks
}

pub(crate) fn compressed_thinking_label(content: &str, preview_chars: usize) -> String {
    let normalized = content.replace('\r', "").replace('\n', " ");
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let char_count = compact.chars().count();

    if preview_chars == 0 || compact.is_empty() {
        return format!("∴ Thinking hidden ({} chars)", char_count);
    }

    let preview = compact.chars().take(preview_chars).collect::<String>();
    let suffix = if char_count > preview_chars {
        "..."
    } else {
        ""
    };
    format!("∴ Thinking {}{}", preview, suffix)
}

fn summarize_tool_run(messages: &[DisplayMessage]) -> Option<ToolRunRender> {
    if messages.len() < 2 {
        return None;
    }

    let mut summary = ToolRunSummary::default();

    for message in messages {
        let tool_name = parse_tool_name(&message.content)?;
        match classify_tool_name(&tool_name) {
            ToolClass::Read => summary.read_count += 1,
            ToolClass::Search => summary.search_count += 1,
            ToolClass::List => summary.list_count += 1,
            ToolClass::Bash => summary.bash_count += 1,
            ToolClass::Other => {
                summary.other_count += 1;
                match &summary.other_name {
                    Some(existing) if existing != &tool_name => return None,
                    Some(_) => {}
                    None => summary.other_name = Some(tool_name),
                }
            }
        }
    }

    if summary.read_count + summary.search_count + summary.list_count + summary.bash_count > 0 {
        let mut parts = Vec::new();
        if summary.read_count > 0 {
            parts.push(format!("Read {} file(s)", summary.read_count));
        }
        if summary.search_count > 0 {
            parts.push(format!("Searched {} time(s)", summary.search_count));
        }
        if summary.list_count > 0 {
            parts.push(format!("Listed {} time(s)", summary.list_count));
        }
        if summary.bash_count > 0 {
            parts.push(format!("Ran {} command(s)", summary.bash_count));
        }
        return Some(ToolRunRender::CollapsedReadSearch {
            summary: parts.join(", "),
        });
    }

    if summary.other_count >= 2 {
        return Some(ToolRunRender::GroupedToolUse {
            tool_name: summary.other_name.unwrap_or_else(|| "tool".to_string()),
            count: summary.other_count,
        });
    }

    None
}

fn classify_tool_name(name: &str) -> ToolClass {
    let lower = name.to_ascii_lowercase();
    if lower.contains("read") || lower.contains("file_read") {
        ToolClass::Read
    } else if lower.contains("search") || lower.contains("grep") || lower.contains("glob") {
        ToolClass::Search
    } else if lower.contains("list") {
        ToolClass::List
    } else if lower.contains("execute_command") || lower.contains("bash") {
        ToolClass::Bash
    } else {
        ToolClass::Other
    }
}

fn parse_tool_name(content: &str) -> Option<String> {
    let (_, remainder) = content.split_once(": ")?;
    let name = remainder
        .lines()
        .next()
        .unwrap_or(remainder)
        .split_whitespace()
        .next()?;
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_thinking_defaults_to_hidden_label() {
        assert_eq!(
            compressed_thinking_label("long hidden reasoning", 0),
            "∴ Thinking hidden (21 chars)"
        );
    }

    #[test]
    fn build_render_blocks_groups_same_non_collapsible_tool() {
        let messages = vec![
            DisplayMessage {
                role: DisplayRole::Tool,
                content: "Tool request: custom_tool {}".to_string(),
                message_id: None,
                parent_id: None,
                entry_type: None,
            },
            DisplayMessage {
                role: DisplayRole::Tool,
                content: "Tool result: custom_tool\nok".to_string(),
                message_id: None,
                parent_id: None,
                entry_type: None,
            },
        ];

        match build_render_blocks(&messages).as_slice() {
            [RenderBlock::GroupedToolUse {
                tool_name, count, ..
            }] => {
                assert_eq!(tool_name, "custom_tool");
                assert_eq!(*count, 2);
            }
            other => panic!("unexpected blocks: {}", other.len()),
        }
    }
}
