#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    Help,
    Clear,
    Diff {
        full: bool,
    },
    Doctor,
    Init {
        force: bool,
        append: bool,
    },
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
    Mcp {
        action: McpSlashAction,
    },
    Plugin {
        action: PluginSlashAction,
    },
    Skills {
        action: SkillsSlashAction,
    },
    Plan {
        action: PlanSlashAction,
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
pub enum McpSlashAction {
    Help,
    List,
    Add {
        name: String,
        command: String,
        args: Vec<String>,
    },
    Remove {
        name: String,
    },
    Restart {
        name: String,
    },
    Start {
        name: String,
    },
    Stop {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSlashAction {
    Help,
    List,
    Search { query: String },
    Install { plugin: String },
    Remove { name: String },
    Enable { name: String },
    Disable { name: String },
    Update { target: PluginUpdateTarget },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginUpdateTarget {
    All,
    One(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsSlashAction {
    Help,
    List,
    Show { name: String },
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanSlashAction {
    Enter { prompt: Option<String> },
    Show,
    Open,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessedInput {
    Prompt(String),
    LocalCommand(LocalCommand),
    Error(String),
}
