#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommandSource {
    Builtin,
    Project,
    User,
    ClaudeCompat,
    VendoredPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommandKind {
    Native,
    Prompt,
    FileBacked { template: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub argument_hint: Option<String>,
    pub source: SlashCommandSource,
    pub kind: SlashCommandKind,
}

impl SlashCommandSpec {
    pub fn matches(&self, candidate: &str) -> bool {
        self.name.eq_ignore_ascii_case(candidate)
            || self
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(candidate))
    }
}
