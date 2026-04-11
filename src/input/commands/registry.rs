use super::{
    builtin::builtin_command_specs, markdown::discover_markdown_commands, spec::SlashCommandSpec,
};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SlashCommandRegistry {
    commands: Vec<SlashCommandSpec>,
}

impl SlashCommandRegistry {
    pub fn load(cwd: &Path) -> Self {
        let mut commands = builtin_command_specs();
        for command in discover_markdown_commands(cwd) {
            if commands
                .iter()
                .any(|existing| existing.matches(&command.name))
            {
                continue;
            }
            commands.push(command);
        }
        Self { commands }
    }

    pub fn all(&self) -> &[SlashCommandSpec] {
        &self.commands
    }

    pub fn find(&self, name: &str) -> Option<&SlashCommandSpec> {
        self.commands.iter().find(|command| command.matches(name))
    }
}
