use crate::runtime::types::RuntimeMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct QueryTurnResult {
    pub history: Vec<RuntimeMessage>,
    pub assistant_message: Option<RuntimeMessage>,
    pub usage: Option<RuntimeUsage>,
    pub model: String,
    pub finish_reason: Option<String>,
}

impl QueryTurnResult {
    pub fn assistant_text(&self) -> Option<&str> {
        self.assistant_message
            .as_ref()
            .map(|message| message.content.as_str())
    }
}
