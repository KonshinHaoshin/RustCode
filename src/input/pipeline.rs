use crate::input::{slash::parse_slash_command, ProcessedInput};

#[derive(Default)]
pub struct InputProcessor;

impl InputProcessor {
    pub fn new() -> Self {
        Self
    }

    pub fn process(&self, input: &str) -> ProcessedInput {
        parse_slash_command(input)
            .map(ProcessedInput::LocalCommand)
            .unwrap_or_else(|| ProcessedInput::Prompt(input.trim().to_string()))
    }
}
