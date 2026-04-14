pub mod builtin;
pub mod init;
pub mod local;
pub mod markdown;
pub mod plan;
pub mod registry;
pub mod spec;

use crate::session::SessionPlan;
use crate::{compact::CompactSettings, config::Settings};
use registry::SlashCommandRegistry;
use spec::{SlashCommandKind, SlashCommandSource};

pub fn format_help_text() -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let registry = SlashCommandRegistry::load(&cwd);
    let mut lines = vec!["Available slash commands:".to_string()];
    for command in registry.all() {
        let source = match command.source {
            SlashCommandSource::Builtin => "builtin",
            SlashCommandSource::Project => "project",
            SlashCommandSource::User => "user",
            SlashCommandSource::ClaudeCompat => "claude-compat",
            SlashCommandSource::VendoredPrompt => "prompt",
        };
        let kind = match &command.kind {
            SlashCommandKind::Native => "native",
            SlashCommandKind::Prompt => "prompt",
            SlashCommandKind::FileBacked { .. } => "markdown",
        };
        let hint = command
            .argument_hint
            .as_deref()
            .map(|value| format!(" {}", value))
            .unwrap_or_default();
        lines.push(format!(
            "/{}{}  {} [{}:{}]",
            command.name, hint, command.description, source, kind
        ));
    }
    lines.join("\n")
}

pub fn format_status_text(
    settings: &Settings,
    session_id: Option<&str>,
    message_count: usize,
    pending_approval: bool,
    usage_total_tokens: Option<usize>,
    plan_mode: bool,
    active_plan: Option<&SessionPlan>,
) -> String {
    format!(
        "Provider: {}\nModel: {}\nProtocol: {}\nFallback: {} ({})\nSession: {}\nMessages: {}\nPending approval: {}\nPlan mode: {}\nCurrent plan: {}\nCompact: enabled={} auto={} reactive={} micro={} turns={} tokens={} reserve={} current_tokens={}",
        settings.api.provider_label(),
        settings.model,
        settings.api.protocol().as_str(),
        if settings.api.fallback.enabled { "on" } else { "off" },
        settings.api.fallback.chain.len(),
        session_id.unwrap_or("none"),
        message_count,
        if pending_approval { "yes" } else { "no" },
        if plan_mode { "on" } else { "off" },
        if active_plan.is_some() { "available" } else { "none" },
        settings.compact.enabled,
        settings.compact.auto_compact,
        settings.compact.reactive_compact,
        settings.compact.enable_microcompact,
        settings.compact.max_turns_before_compact,
        settings.compact.max_tokens_before_compact,
        settings.compact.reserved_completion_budget,
        usage_total_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    )
}

pub fn format_compact_status(settings: &CompactSettings) -> String {
    format!(
        "Compact settings: enabled={} auto={} reactive={} micro={} turns={} tokens={} preserve_recent_turns={} reserve={}",
        settings.enabled,
        settings.auto_compact,
        settings.reactive_compact,
        settings.enable_microcompact,
        settings.max_turns_before_compact,
        settings.max_tokens_before_compact,
        settings.preserve_recent_turns,
        settings.reserved_completion_budget
    )
}
