use crate::{compact::CompactSettings, config::Settings};

pub fn format_help_text() -> String {
    [
        "Available slash commands:",
        "/help           Show available commands",
        "/clear          Clear the active conversation and start a new session",
        "/compact [text] Compact history into a summary and keep recent context",
        "/permissions    Inspect permission rules",
        "/model [name]   Show or change the active model for this session",
        "/status         Show current runtime status",
        "/resume [id]    Resume the latest or specified session",
    ]
    .join("\n")
}

pub fn format_status_text(
    settings: &Settings,
    session_id: Option<&str>,
    message_count: usize,
    pending_approval: bool,
    usage_total_tokens: Option<usize>,
) -> String {
    format!(
        "Provider: {}\nModel: {}\nProtocol: {}\nFallback: {} ({})\nSession: {}\nMessages: {}\nPending approval: {}\nCompact: enabled={} auto={} turns={} tokens={} current_tokens={}",
        settings.api.provider_label(),
        settings.model,
        settings.api.protocol().as_str(),
        if settings.api.fallback.enabled { "on" } else { "off" },
        settings.api.fallback.chain.len(),
        session_id.unwrap_or("none"),
        message_count,
        if pending_approval { "yes" } else { "no" },
        settings.compact.enabled,
        settings.compact.auto_compact,
        settings.compact.max_turns_before_compact,
        settings.compact.max_tokens_before_compact,
        usage_total_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    )
}

pub fn format_compact_status(settings: &CompactSettings) -> String {
    format!(
        "Compact settings: enabled={} auto={} turns={} tokens={} preserve_recent_turns={}",
        settings.enabled,
        settings.auto_compact,
        settings.max_turns_before_compact,
        settings.max_tokens_before_compact,
        settings.preserve_recent_turns
    )
}
