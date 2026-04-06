#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    Help,
    Clear,
    Compact { instructions: Option<String> },
    Permissions,
    Model { model: Option<String> },
    Status,
    Resume { session_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessedInput {
    Prompt(String),
    LocalCommand(LocalCommand),
}
