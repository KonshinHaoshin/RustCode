use crate::runtime::RuntimeMessage;

pub fn build_compact_messages(
    history: &[RuntimeMessage],
    instructions: Option<&str>,
) -> (String, String) {
    let system = [
        "You are compacting a coding-assistant transcript.",
        "Produce a concise structured summary for continuing work later.",
        "Focus on goals, constraints, completed work, pending work, important facts, and blockers.",
        "Do not include filler or conversational framing.",
    ]
    .join(" ");

    let mut user = String::from(
        "Summarize the following conversation for future continuation.\n\nUse sections:\n- Goal\n- Constraints\n- Completed\n- Pending\n- Important Context\n- Risks\n\nTranscript:\n",
    );
    user.push_str(&render_history(history));

    if let Some(instructions) = instructions
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        user.push_str("\n\nAdditional instructions:\n");
        user.push_str(instructions);
    }

    (system, user)
}

fn render_history(history: &[RuntimeMessage]) -> String {
    let mut rendered = String::new();

    for message in history {
        rendered.push_str(message.role.as_str());
        rendered.push_str(": ");
        if !message.content.trim().is_empty() {
            rendered.push_str(message.content.trim());
        }

        if !message.tool_calls.is_empty() {
            for tool_call in &message.tool_calls {
                rendered.push_str("\nassistant_tool_call: ");
                rendered.push_str(&tool_call.name);
                rendered.push(' ');
                rendered.push_str(&tool_call.arguments.to_string());
            }
        }

        if let Some(tool_result) = &message.tool_result {
            rendered.push_str("\ntool_result: ");
            rendered.push_str(&tool_result.name);
            rendered.push_str(" => ");
            rendered.push_str(tool_result.content.trim());
        }

        rendered.push_str("\n\n");
    }

    rendered
}
