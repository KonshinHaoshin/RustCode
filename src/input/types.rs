#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    Help,
    Clear,
    Branch {
        message_id: Option<String>,
    },
    Compact {
        instructions: Option<String>,
    },
    Permissions,
    Model {
        model: Option<String>,
    },
    Rewind {
        message_id: Option<String>,
        files_only: bool,
    },
    Status,
    Resume {
        session_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessedInput {
    Prompt(String),
    LocalCommand(LocalCommand),
}
