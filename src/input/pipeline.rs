use crate::input::{slash::process_slash_input, ProcessedInput};

#[derive(Default)]
pub struct InputProcessor;

impl InputProcessor {
    pub fn new() -> Self {
        Self
    }

    pub fn process(&self, input: &str) -> ProcessedInput {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        process_slash_input(input, &cwd)
            .unwrap_or_else(|| ProcessedInput::Prompt(input.trim().to_string()))
    }
}
