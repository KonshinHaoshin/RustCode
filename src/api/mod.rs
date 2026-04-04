//! API Module - Multi-provider API client with fallback support

use crate::config::{ApiProtocol, ResolvedApiTarget, Settings};
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

    pub async fn chat(&self, messages: Vec<ChatMessage>) -> anyhow::Result<ChatResponse> {
        let targets = self.request_targets();
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

            match self.chat_once(target, &messages).await {
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
        self.send_request(&target, &messages, true)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))
    }

    async fn chat_once(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
    ) -> Result<ChatResponse, AttemptFailure> {
        let response = self.send_request(target, messages, false).await?;
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
        stream: bool,
    ) -> Result<reqwest::Response, AttemptFailure> {
        let response = match target.protocol {
            ApiProtocol::OpenAi => self.send_openai_request(target, messages, stream).await,
            ApiProtocol::Anthropic => self.send_anthropic_request(target, messages, stream).await,
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
        stream: bool,
    ) -> anyhow::Result<reqwest::Response> {
        let request = OpenAiChatRequest {
            model: target.model.clone(),
            messages: messages.to_vec(),
            max_tokens: self.settings.api.max_tokens,
            stream,
            temperature: 0.7,
        };

        let url = format!(
            "{}/v1/chat/completions",
            target.base_url.trim_end_matches('/')
        );
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
        stream: bool,
    ) -> anyhow::Result<reqwest::Response> {
        let (system, anthropic_messages) = Self::to_anthropic_messages(messages);
        let request = AnthropicRequest {
            model: target.model.clone(),
            max_tokens: self.settings.api.max_tokens,
            messages: anthropic_messages,
            system,
            stream,
        };

        let url = format!("{}/v1/messages", target.base_url.trim_end_matches('/'));
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
                    .push(AnthropicMessage::from_text("assistant", &message.content)),
                _ => anthropic_messages.push(AnthropicMessage::from_text("user", &message.content)),
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
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_calls: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            tool_calls: None,
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
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

impl AnthropicMessage {
    fn from_text(role: &str, text: &str) -> Self {
        Self {
            role: role.to_string(),
            content: vec![AnthropicContentBlock::text(text)],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

impl AnthropicContentBlock {
    fn text(text: &str) -> Self {
        Self {
            block_type: "text".to_string(),
            text: Some(text.to_string()),
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

        Self {
            id: response.id,
            object: response.object,
            created: Utc::now().timestamp(),
            model: response.model,
            choices: vec![Choice {
                index: 0,
                message: ChatMessage::assistant(content),
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
