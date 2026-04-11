use super::spec::{SlashCommandKind, SlashCommandSource, SlashCommandSpec};

pub fn builtin_command_specs() -> Vec<SlashCommandSpec> {
    vec![
        native("help", "Show available commands", None, &["h"]),
        native(
            "clear",
            "Clear the active conversation and start a new session",
            None,
            &[],
        ),
        prompt(
            "init",
            "Analyze this codebase and create or update rustcode.md",
            None,
        ),
        native(
            "branch",
            "Fork the current session, optionally from a user message id",
            Some("[id]"),
            &["fork"],
        ),
        native(
            "compact",
            "Compact history into a summary and keep recent context",
            Some("[text]"),
            &[],
        ),
        native("permissions", "Inspect permission rules", None, &[]),
        native(
            "model",
            "Show or change the active model for this session",
            Some("[name]"),
            &[],
        ),
        native(
            "rewind",
            "Rewind conversation and files to a user message id (or last-user)",
            Some("<id>"),
            &[],
        ),
        native(
            "rewind-files",
            "Rewind only files to a user message id (or last-user)",
            Some("<id>"),
            &[],
        ),
        native("status", "Show current runtime status", None, &[]),
        native(
            "resume",
            "Resume the latest or a matching session",
            Some("[query]"),
            &[],
        ),
        prompt(
            "review",
            "Review the current change or target for correctness and risk",
            Some("[scope]"),
        ),
        prompt(
            "explain",
            "Explain a file, symbol, or code path",
            Some("<file-or-topic>"),
        ),
        prompt(
            "fix",
            "Propose and implement a fix for a problem",
            Some("<issue>"),
        ),
        prompt(
            "test",
            "Design or suggest tests for the current workspace or target",
            Some("[scope]"),
        ),
    ]
}

fn native(
    name: &str,
    description: &str,
    argument_hint: Option<&str>,
    aliases: &[&str],
) -> SlashCommandSpec {
    SlashCommandSpec {
        name: name.to_string(),
        aliases: aliases.iter().map(|value| (*value).to_string()).collect(),
        description: description.to_string(),
        argument_hint: argument_hint.map(str::to_string),
        source: SlashCommandSource::Builtin,
        kind: SlashCommandKind::Native,
    }
}

fn prompt(name: &str, description: &str, argument_hint: Option<&str>) -> SlashCommandSpec {
    SlashCommandSpec {
        name: name.to_string(),
        aliases: Vec::new(),
        description: description.to_string(),
        argument_hint: argument_hint.map(str::to_string),
        source: SlashCommandSource::VendoredPrompt,
        kind: SlashCommandKind::Prompt,
    }
}
