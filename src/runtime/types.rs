use crate::api::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeRole {
    System,
    User,
    Assistant,
    Tool,
}

impl RuntimeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMessage {
    pub role: RuntimeRole,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RuntimeToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<RuntimeToolResult>,
}

impl RuntimeMessage {
    pub fn new(role: RuntimeRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_result: None,
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

    pub fn assistant_with_tool_calls(tool_calls: Vec<RuntimeToolCall>) -> Self {
        Self {
            role: RuntimeRole::Assistant,
            content: String::new(),
            tool_calls,
            tool_result: None,
        }
    }

    pub fn tool_result(result: RuntimeToolResult) -> Self {
        Self {
            role: RuntimeRole::Tool,
            content: result.content.clone(),
            tool_calls: Vec::new(),
            tool_result: Some(result),
        }
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

impl From<RuntimeMessage> for ChatMessage {
    fn from(value: RuntimeMessage) -> Self {
        let tool_calls = if value.tool_calls.is_empty() {
            None
        } else {
            Some(
                value
                    .tool_calls
                    .iter()
                    .map(runtime_tool_call_to_api_value)
                    .collect(),
            )
        };

        let (content, tool_call_id, name) = match value.tool_result {
            Some(result) => (result.content, Some(result.tool_call_id), Some(result.name)),
            None => (value.content, None, None),
        };

        ChatMessage {
            role: value.role.as_str().to_string(),
            content,
            tool_calls,
            tool_call_id,
            name,
        }
    }
}

impl From<&RuntimeMessage> for ChatMessage {
    fn from(value: &RuntimeMessage) -> Self {
        let tool_calls = if value.tool_calls.is_empty() {
            None
        } else {
            Some(
                value
                    .tool_calls
                    .iter()
                    .map(runtime_tool_call_to_api_value)
                    .collect(),
            )
        };

        let (content, tool_call_id, name) = match &value.tool_result {
            Some(result) => (
                result.content.clone(),
                Some(result.tool_call_id.clone()),
                Some(result.name.clone()),
            ),
            None => (value.content.clone(), None, None),
        };

        ChatMessage {
            role: value.role.as_str().to_string(),
            content,
            tool_calls,
            tool_call_id,
            name,
        }
    }
}

impl From<ChatMessage> for RuntimeMessage {
    fn from(value: ChatMessage) -> Self {
        let role = match value.role.as_str() {
            "system" => RuntimeRole::System,
            "assistant" => RuntimeRole::Assistant,
            "tool" => RuntimeRole::Tool,
            _ => RuntimeRole::User,
        };

        let tool_calls = value
            .tool_calls
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(api_value_to_runtime_tool_call)
            .collect::<Vec<_>>();
        let tool_result = if role == RuntimeRole::Tool {
            value.tool_call_id.map(|tool_call_id| RuntimeToolResult {
                tool_call_id,
                name: value.name.unwrap_or_else(|| "tool".to_string()),
                content: value.content.clone(),
                is_error: false,
            })
        } else {
            None
        };

        Self {
            role,
            content: value.content,
            tool_calls,
            tool_result,
        }
    }
}

impl From<&ChatMessage> for RuntimeMessage {
    fn from(value: &ChatMessage) -> Self {
        let role = match value.role.as_str() {
            "system" => RuntimeRole::System,
            "assistant" => RuntimeRole::Assistant,
            "tool" => RuntimeRole::Tool,
            _ => RuntimeRole::User,
        };

        let tool_calls = value
            .tool_calls
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(api_value_to_runtime_tool_call)
            .collect::<Vec<_>>();
        let tool_result = if role == RuntimeRole::Tool {
            value
                .tool_call_id
                .clone()
                .map(|tool_call_id| RuntimeToolResult {
                    tool_call_id,
                    name: value.name.clone().unwrap_or_else(|| "tool".to_string()),
                    content: value.content.clone(),
                    is_error: false,
                })
        } else {
            None
        };

        Self {
            role,
            content: value.content.clone(),
            tool_calls,
            tool_result,
        }
    }
}

fn runtime_tool_call_to_api_value(call: &RuntimeToolCall) -> serde_json::Value {
    serde_json::json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string())
        }
    })
}

fn api_value_to_runtime_tool_call(value: &serde_json::Value) -> Option<RuntimeToolCall> {
    let id = value.get("id")?.as_str()?.to_string();
    let function = value.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let raw_arguments = function.get("arguments")?;
    let arguments = match raw_arguments {
        serde_json::Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| {
            serde_json::json!({
                "raw": text
            })
        }),
        other => other.clone(),
    };

    Some(RuntimeToolCall {
        id,
        name,
        arguments,
    })
}
