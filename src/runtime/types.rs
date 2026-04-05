use crate::api::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeRole {
    System,
    User,
    Assistant,
}

impl RuntimeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMessage {
    pub role: RuntimeRole,
    pub content: String,
}

impl RuntimeMessage {
    pub fn new(role: RuntimeRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(RuntimeRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(RuntimeRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(RuntimeRole::Assistant, content)
    }
}

impl From<RuntimeMessage> for ChatMessage {
    fn from(value: RuntimeMessage) -> Self {
        ChatMessage {
            role: value.role.as_str().to_string(),
            content: value.content,
            tool_calls: None,
        }
    }
}

impl From<&RuntimeMessage> for ChatMessage {
    fn from(value: &RuntimeMessage) -> Self {
        ChatMessage {
            role: value.role.as_str().to_string(),
            content: value.content.clone(),
            tool_calls: None,
        }
    }
}

impl From<ChatMessage> for RuntimeMessage {
    fn from(value: ChatMessage) -> Self {
        let role = match value.role.as_str() {
            "system" => RuntimeRole::System,
            "assistant" => RuntimeRole::Assistant,
            _ => RuntimeRole::User,
        };

        Self {
            role,
            content: value.content,
        }
    }
}

impl From<&ChatMessage> for RuntimeMessage {
    fn from(value: &ChatMessage) -> Self {
        let role = match value.role.as_str() {
            "system" => RuntimeRole::System,
            "assistant" => RuntimeRole::Assistant,
            _ => RuntimeRole::User,
        };

        Self {
            role,
            content: value.content.clone(),
        }
    }
}
