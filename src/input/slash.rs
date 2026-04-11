use crate::input::{
    commands::{registry::SlashCommandRegistry, spec::SlashCommandKind},
    types::{LocalCommand, ProcessedInput},
};
use std::path::Path;

pub fn process_slash_input(input: &str, cwd: &Path) -> Option<ProcessedInput> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let command_name = parts.next()?.trim().to_ascii_lowercase();
    let args = parts.next().map(str::trim).unwrap_or_default();
    let registry = SlashCommandRegistry::load(cwd);
    let Some(command) = registry.find(&command_name) else {
        return Some(ProcessedInput::Error(format!(
            "Unknown slash command: /{}",
            command_name
        )));
    };

    match &command.kind {
        SlashCommandKind::Native => Some(ProcessedInput::LocalCommand(parse_native_command(
            &command.name,
            args,
        ))),
        SlashCommandKind::Prompt => Some(ProcessedInput::Prompt(format_prompt_command(
            &command.name,
            args,
        ))),
        SlashCommandKind::FileBacked { template } => Some(ProcessedInput::Prompt(
            render_markdown_command(template, args, cwd),
        )),
    }
}

pub fn parse_slash_command(input: &str) -> Option<LocalCommand> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match process_slash_input(input, &cwd) {
        Some(ProcessedInput::LocalCommand(command)) => Some(command),
        _ => None,
    }
}

fn parse_native_command(command: &str, args: &str) -> LocalCommand {
    match command {
        "help" => LocalCommand::Help,
        "clear" => LocalCommand::Clear,
        "init" => LocalCommand::Init {
            force: args.split_whitespace().any(|arg| arg == "--force"),
            append: args.split_whitespace().any(|arg| arg == "--append"),
        },
        "branch" => LocalCommand::Branch {
            message_id: (!args.is_empty()).then(|| args.to_string()),
        },
        "compact" => LocalCommand::Compact {
            instructions: (!args.is_empty()).then(|| args.to_string()),
        },
        "permissions" => LocalCommand::Permissions,
        "model" => LocalCommand::Model {
            model: (!args.is_empty()).then(|| args.to_string()),
        },
        "rewind" => LocalCommand::Rewind {
            message_id: (!args.is_empty()).then(|| args.to_string()),
            files_only: false,
        },
        "rewind-files" => LocalCommand::Rewind {
            message_id: (!args.is_empty()).then(|| args.to_string()),
            files_only: true,
        },
        "status" => LocalCommand::Status,
        "resume" => LocalCommand::Resume {
            session_id: (!args.is_empty()).then(|| args.to_string()),
        },
        _ => LocalCommand::Help,
    }
}

fn format_prompt_command(command: &str, args: &str) -> String {
    match command {
        "review" => format!(
            "Review this codebase or change with a code-review mindset. Prioritize correctness, regressions, security risks, and missing tests. Scope: {}",
            empty_as_default(args, "current workspace or diff")
        ),
        "explain" => format!(
            "Explain the following target clearly and concisely, including important architecture and behavior: {}",
            empty_as_default(args, "the current context")
        ),
        "fix" => format!(
            "Investigate and fix this issue. Gather evidence before changing code, make the smallest correct change, and verify it: {}",
            empty_as_default(args, "the current issue")
        ),
        "test" => format!(
            "Create or improve tests for this target. Prefer focused tests that verify behavior and regressions: {}",
            empty_as_default(args, "the current change")
        ),
        _ => args.to_string(),
    }
}

fn render_markdown_command(template: &str, args: &str, cwd: &Path) -> String {
    template
        .replace("$ARGUMENTS", args)
        .replace("$CWD", &cwd.display().to_string())
        .replace(
            "$RUSTCODE_MD",
            &cwd.join("rustcode.md").display().to_string(),
        )
}

fn empty_as_default<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.trim().is_empty() {
        default
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_slash_command, process_slash_input};
    use crate::input::{LocalCommand, ProcessedInput};

    #[test]
    fn parses_basic_slash_commands() {
        assert_eq!(parse_slash_command("/help"), Some(LocalCommand::Help));
        assert_eq!(parse_slash_command("/status"), Some(LocalCommand::Status));
        assert_eq!(
            parse_slash_command("/branch"),
            Some(LocalCommand::Branch { message_id: None })
        );
        assert_eq!(
            parse_slash_command("/compact keep risks"),
            Some(LocalCommand::Compact {
                instructions: Some("keep risks".to_string()),
            })
        );
    }

    #[test]
    fn parses_init_flags() {
        assert_eq!(
            parse_slash_command("/init --force"),
            Some(LocalCommand::Init {
                force: true,
                append: false,
            })
        );
    }

    #[test]
    fn unknown_slash_command_is_error() {
        let temp = tempfile::tempdir().unwrap();
        assert!(matches!(
            process_slash_input("/missing", temp.path()),
            Some(ProcessedInput::Error(_))
        ));
    }
}
