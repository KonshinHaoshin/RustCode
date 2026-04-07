use crate::input::types::LocalCommand;

pub fn parse_slash_command(input: &str) -> Option<LocalCommand> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let command = parts.next()?.trim().to_ascii_lowercase();
    let args = parts.next().map(str::trim).unwrap_or_default();

    match command.as_str() {
        "help" => Some(LocalCommand::Help),
        "clear" => Some(LocalCommand::Clear),
        "branch" | "fork" => Some(LocalCommand::Branch {
            message_id: (!args.is_empty()).then(|| args.to_string()),
        }),
        "compact" => Some(LocalCommand::Compact {
            instructions: (!args.is_empty()).then(|| args.to_string()),
        }),
        "permissions" => Some(LocalCommand::Permissions),
        "model" => Some(LocalCommand::Model {
            model: (!args.is_empty()).then(|| args.to_string()),
        }),
        "rewind" => Some(LocalCommand::Rewind {
            message_id: (!args.is_empty()).then(|| args.to_string()),
            files_only: false,
        }),
        "rewind-files" => Some(LocalCommand::Rewind {
            message_id: (!args.is_empty()).then(|| args.to_string()),
            files_only: true,
        }),
        "status" => Some(LocalCommand::Status),
        "resume" => Some(LocalCommand::Resume {
            session_id: (!args.is_empty()).then(|| args.to_string()),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_slash_command;
    use crate::input::LocalCommand;

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
    fn parses_resume_and_model_arguments() {
        assert_eq!(
            parse_slash_command("/resume abc123"),
            Some(LocalCommand::Resume {
                session_id: Some("abc123".to_string()),
            })
        );
        assert_eq!(
            parse_slash_command("/model gpt-4o"),
            Some(LocalCommand::Model {
                model: Some("gpt-4o".to_string()),
            })
        );
        assert_eq!(
            parse_slash_command("/rewind-files msg-1"),
            Some(LocalCommand::Rewind {
                message_id: Some("msg-1".to_string()),
                files_only: true,
            })
        );
        assert_eq!(
            parse_slash_command("/rewind"),
            Some(LocalCommand::Rewind {
                message_id: None,
                files_only: false,
            })
        );
    }
}
