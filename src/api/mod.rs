//! API Module - Multi-provider API client with fallback support

use crate::{
    config::{ApiProtocol, ResolvedApiTarget, Settings},
    tools_runtime::ToolDefinition,
};
use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

pub struct ApiClient {
    settings: Settings,
    http_client: Client,
}

#[derive(Debug)]
struct AttemptFailure {
    message: String,
    eligible_for_fallback: bool,
}

impl ApiClient {
    pub fn new(settings: Settings) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(settings.api.timeout))
            .build()
            .unwrap_or_default();

        Self {
            settings,
            http_client,
        }
    }

    pub fn get_api_key(&self) -> Option<String> {
        self.settings.api.get_api_key()
    }

    pub fn get_base_url(&self) -> String {
        self.settings.api.get_base_url()
    }

    pub fn get_model(&self) -> String {
        self.settings.api.get_model_id(&self.settings.model)
    }

    fn request_targets(&self) -> Vec<ResolvedApiTarget> {
        let mut targets = Vec::new();
        targets.push(self.settings.api.active_target(&self.settings.model));
        targets.extend(self.settings.api.fallback_targets());

        let mut seen = HashSet::new();
        targets
            .into_iter()
            .filter(|target| {
                seen.insert(format!(
                    "{}|{}|{}|{}",
                    target.provider_label,
                    target.protocol.as_str(),
                    target.base_url,
                    target.model
                ))
            })
            .collect()
    }

    fn request_targets_for_tools(&self, requires_tool_calling: bool) -> Vec<ResolvedApiTarget> {
        let mut targets = self.request_targets();
        if requires_tool_calling {
            targets.sort_by_key(|target| !target.supports_tool_calling());
        }
        targets
    }

    pub async fn chat(&self, messages: &[ChatMessage]) -> anyhow::Result<ChatResponse> {
        self.chat_with_tools(messages, &[]).await
    }

    pub async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<ChatResponse> {
        let targets = self.request_targets_for_tools(!tools.is_empty());
        let total_targets = targets.len();
        let mut failures = Vec::new();

        for (index, target) in targets.iter().enumerate() {
            if index > 0 {
                eprintln!(
                    "Primary model failed, trying fallback {}/{}: {}",
                    index,
                    total_targets.saturating_sub(1),
                    target.display_name()
                );
            }

            match self.chat_once(target, messages, tools).await {
                Ok(response) => {
                    if index > 0 {
                        eprintln!("Fallback succeeded with {}", target.display_name());
                    }
                    return Ok(response);
                }
                Err(error) => {
                    failures.push(format!("{}: {}", target.display_name(), error.message));
                    if !error.eligible_for_fallback {
                        break;
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "All configured providers/models failed:\n{}",
            failures.join("\n")
        ))
    }

    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<reqwest::Response> {
        let target = self.settings.api.active_target(&self.settings.model);
        self.send_request(&target, &messages, &[], true)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))
    }

    async fn chat_once(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<ChatResponse, AttemptFailure> {
        let response = self
            .send_request(
                target,
                messages,
                if target.supports_tool_calling() {
                    tools
                } else {
                    &[]
                },
                false,
            )
            .await?;
        let status = response.status();

        match target.protocol {
            ApiProtocol::OpenAi => {
                let parsed: OpenAiChatResponse =
                    response.json().await.map_err(|error| AttemptFailure {
                        message: format!("failed to parse OpenAI-style response: {}", error),
                        eligible_for_fallback: true,
                    })?;
                Ok(parsed.into())
            }
            ApiProtocol::Anthropic => {
                let parsed: AnthropicResponse =
                    response.json().await.map_err(|error| AttemptFailure {
                        message: format!("failed to parse Anthropic-style response: {}", error),
                        eligible_for_fallback: true,
                    })?;
                Ok(ChatResponse::from_anthropic(parsed, status))
            }
        }
    }

    async fn send_request(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        stream: bool,
    ) -> Result<reqwest::Response, AttemptFailure> {
        let response = match target.protocol {
            ApiProtocol::OpenAi => {
                self.send_openai_request(target, messages, tools, stream)
                    .await
            }
            ApiProtocol::Anthropic => {
                self.send_anthropic_request(target, messages, tools, stream)
                    .await
            }
        }
        .map_err(|error| AttemptFailure {
            message: error.to_string(),
            eligible_for_fallback: true,
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AttemptFailure {
                message: format!("API error ({}): {}", status, body),
                eligible_for_fallback: Self::is_fallback_status(status),
            });
        }

        Ok(response)
    }

    async fn send_openai_request(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        stream: bool,
    ) -> anyhow::Result<reqwest::Response> {
        let request = OpenAiChatRequest {
            model: target.model.clone(),
            messages: messages.to_vec(),
            max_tokens: self.settings.api.max_tokens,
            stream,
            temperature: 0.7,
            tools: (!tools.is_empty()).then(|| {
                tools
                    .iter()
                    .map(|tool| OpenAiToolDefinition::from(tool.clone()))
                    .collect()
            }),
            tool_choice: (!tools.is_empty()).then(|| "auto".to_string()),
        };

        let url = build_api_url(&target.base_url, "/v1/chat/completions");
        let mut builder = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json");

        if let Some(api_key) = target.api_key.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.header("Authorization", format!("Bearer {}", api_key));
        }

        Ok(builder.json(&request).send().await?)
    }

    async fn send_anthropic_request(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        stream: bool,
    ) -> anyhow::Result<reqwest::Response> {
        let (system, anthropic_messages) = Self::to_anthropic_messages(messages);
        let request = AnthropicRequest {
            model: target.model.clone(),
            max_tokens: self.settings.api.max_tokens,
            messages: anthropic_messages,
            system,
            stream,
            tools: (!tools.is_empty()).then(|| {
                tools
                    .iter()
                    .map(|tool| AnthropicToolDefinition::from(tool.clone()))
                    .collect()
            }),
        };

        let url = build_api_url(&target.base_url, "/v1/messages");
        let mut builder = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .header("anthropic-version", "2023-06-01");

        if let Some(api_key) = target.api_key.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.header("x-api-key", api_key);
        }

        if !self.settings.api.beta_headers.is_empty() {
            builder = builder.header("anthropic-beta", self.settings.api.beta_headers.join(","));
        }

        Ok(builder.json(&request).send().await?)
    }

    fn to_anthropic_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_messages = Vec::new();
        let mut anthropic_messages = Vec::new();

        for message in messages {
            match message.role.as_str() {
                "system" => system_messages.push(message.content.clone()),
                "assistant" => anthropic_messages
                    .push(AnthropicMessage::from_chat_message("assistant", message)),
                "tool" => anthropic_messages.push(AnthropicMessage::tool_result(message)),
                _ => anthropic_messages.push(AnthropicMessage::from_chat_message("user", message)),
            }
        }

        let system = if system_messages.is_empty() {
            None
        } else {
            Some(system_messages.join("\n\n"))
        };

        (system, anthropic_messages)
    }

    fn is_fallback_status(status: StatusCode) -> bool {
        matches!(
            status,
            StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
                | StatusCode::NOT_FOUND
                | StatusCode::REQUEST_TIMEOUT
                | StatusCode::CONFLICT
                | StatusCode::TOO_MANY_REQUESTS
        ) || status.is_server_error()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: usize,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    function: OpenAiFunctionDefinition,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiFunctionDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl From<ToolDefinition> for OpenAiToolDefinition {
    fn from(value: ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            function: OpenAiFunctionDefinition {
                name: value.name,
                description: value.description,
                parameters: value.input_schema,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

impl From<OpenAiChatResponse> for ChatResponse {
    fn from(value: OpenAiChatResponse) -> Self {
        Self {
            id: value.id,
            object: value.object,
            created: value.created,
            model: value.model,
            choices: value.choices,
            usage: value.usage,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolDefinition>>,
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

impl AnthropicMessage {
    fn from_chat_message(role: &str, message: &ChatMessage) -> Self {
        let mut content = Vec::new();
        if !message.content.is_empty() {
            content.push(AnthropicContentBlock::text(&message.content));
        }
        if let Some(tool_calls) = &message.tool_calls {
            content.extend(
                tool_calls
                    .iter()
                    .filter_map(anthropic_tool_use_block_from_api_value),
            );
        }
        if content.is_empty() {
            content.push(AnthropicContentBlock::text(""));
        }

        Self {
            role: role.to_string(),
            content,
        }
    }

    fn tool_result(message: &ChatMessage) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![AnthropicContentBlock::tool_result(
                message.tool_call_id.as_deref().unwrap_or_default(),
                &message.content,
            )],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Vec<AnthropicToolResultContentBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

impl AnthropicContentBlock {
    fn text(text: &str) -> Self {
        Self {
            block_type: "text".to_string(),
            text: Some(text.to_string()),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            is_error: None,
        }
    }

    fn tool_use(id: String, name: String, input: serde_json::Value) -> Self {
        Self {
            block_type: "tool_use".to_string(),
            text: None,
            id: Some(id),
            name: Some(name),
            input: Some(input),
            tool_use_id: None,
            content: None,
            is_error: None,
        }
    }

    fn tool_result(tool_use_id: &str, text: &str) -> Self {
        Self {
            block_type: "tool_result".to_string(),
            text: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: Some(tool_use_id.to_string()),
            content: Some(vec![AnthropicToolResultContentBlock::text(text)]),
            is_error: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicToolResultContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

impl AnthropicToolResultContentBlock {
    fn text(text: &str) -> Self {
        Self {
            block_type: "text".to_string(),
            text: text.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicToolDefinition {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

impl From<ToolDefinition> for AnthropicToolDefinition {
    fn from(value: ToolDefinition) -> Self {
        Self {
            name: value.name,
            description: value.description,
            input_schema: value.input_schema,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicResponse {
    id: String,
    #[serde(default = "anthropic_object")]
    object: String,
    model: String,
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicUsage {
    input_tokens: usize,
    output_tokens: usize,
}

fn anthropic_object() -> String {
    "message".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

impl ChatResponse {
    fn from_anthropic(response: AnthropicResponse, _status: StatusCode) -> Self {
        let content = response
            .content
            .iter()
            .filter_map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("");
        let tool_calls = response
            .content
            .iter()
            .filter_map(anthropic_tool_use_block_to_api_value)
            .collect::<Vec<_>>();
        let mut message = ChatMessage::assistant(content);
        if !tool_calls.is_empty() {
            message.tool_calls = Some(tool_calls);
        }

        Self {
            id: response.id,
            object: response.object,
            created: Utc::now().timestamp(),
            model: response.model,
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason: response.stop_reason,
            }],
            usage: response.usage.map(|usage| Usage {
                prompt_tokens: usage.input_tokens,
                completion_tokens: usage.output_tokens,
                total_tokens: usage.input_tokens + usage.output_tokens,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: i32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamChoice {
    pub index: i32,
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Delta {
    pub role: Option<String>,
    pub content: Option<String>,
}

pub type AnthropicClient = ApiClient;

fn anthropic_tool_use_block_from_api_value(
    value: &serde_json::Value,
) -> Option<AnthropicContentBlock> {
    let id = value.get("id")?.as_str()?.to_string();
    let function = value.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let raw_arguments = function.get("arguments")?;
    let input = match raw_arguments {
        serde_json::Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| {
            serde_json::json!({
                "raw": text
            })
        }),
        other => other.clone(),
    };

    Some(AnthropicContentBlock::tool_use(id, name, input))
}

fn anthropic_tool_use_block_to_api_value(
    block: &AnthropicContentBlock,
) -> Option<serde_json::Value> {
    if block.block_type != "tool_use" {
        return None;
    }

    Some(serde_json::json!({
        "id": block.id.as_ref()?,
        "type": "function",
        "function": {
            "name": block.name.as_ref()?,
            "arguments": serde_json::to_string(block.input.as_ref()?).unwrap_or_else(|_| "{}".to_string())
        }
    }))
}

fn build_api_url(base_url: &str, endpoint_path: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');

    if trimmed.ends_with(endpoint_path) {
        return trimmed.to_string();
    }

    match endpoint_path {
        "/v1/chat/completions" => {
            if trimmed.ends_with("/chat/completions") {
                return trimmed.to_string();
            }
            if trimmed.ends_with("/v1") {
                return format!("{}/chat/completions", trimmed);
            }
        }
        "/v1/messages" => {
            if trimmed.ends_with("/messages") {
                return trimmed.to_string();
            }
            if trimmed.ends_with("/v1") {
                return format!("{}/messages", trimmed);
            }
        }
        _ => {}
    }

    format!("{}{}", trimmed, endpoint_path)
}

#[cfg(test)]
mod tests {
    use super::build_api_url;

    #[test]
    fn openai_url_uses_root_base_url() {
        assert_eq!(
            build_api_url("https://example.com", "/v1/chat/completions"),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_does_not_duplicate_v1() {
        assert_eq!(
            build_api_url("https://example.com/v1", "/v1/chat/completions"),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_accepts_full_endpoint() {
        assert_eq!(
            build_api_url(
                "https://example.com/v1/chat/completions",
                "/v1/chat/completions"
            ),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn anthropic_url_does_not_duplicate_v1() {
        assert_eq!(
            build_api_url("https://example.com/v1", "/v1/messages"),
            "https://example.com/v1/messages"
        );
    }
}
