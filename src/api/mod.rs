//! API Module - Multi-provider API client with fallback support

use crate::{
    config::{ApiProtocol, ResolvedApiTarget, Settings},
    runtime::{ProgressSink, QueryProgressEvent},
    tools_runtime::ToolDefinition,
};
use chrono::Utc;
use futures::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
        let mut progress = |_: QueryProgressEvent| {};
        self.chat_with_tools_and_progress(messages, tools, &mut progress)
            .await
    }

    pub async fn chat_with_tools_and_progress(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<ChatResponse> {
        let targets = self.request_targets_for_tools(!tools.is_empty());
        let total_targets = targets.len();
        let mut failures = Vec::new();

        for (index, target) in targets.iter().enumerate() {
            progress.emit(QueryProgressEvent::ModelRequest {
                target: target.display_name(),
            });
            if index > 0 {
                eprintln!(
                    "Primary model failed, trying fallback {}/{}: {}",
                    index,
                    total_targets.saturating_sub(1),
                    target.display_name()
                );
            }

            match self.chat_once(target, messages, tools, progress).await {
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
        progress: &mut dyn ProgressSink,
    ) -> Result<ChatResponse, AttemptFailure> {
        let active_tools = if target.supports_tool_calling() {
            tools
        } else {
            &[]
        };

        match target.protocol {
            ApiProtocol::OpenAi => {
                return self
                    .chat_once_openai_streaming(target, messages, active_tools, progress)
                    .await;
            }
            ApiProtocol::Anthropic => {
                return self
                    .chat_once_anthropic_streaming(target, messages, active_tools, progress)
                    .await;
            }
            ApiProtocol::Responses => {
                return self.chat_once_responses(target, messages, progress).await;
            }
        }
    }

    async fn chat_once_responses(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        progress: &mut dyn ProgressSink,
    ) -> Result<ChatResponse, AttemptFailure> {
        let response = self
            .send_responses_request(target, messages)
            .await
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

        let parsed: ResponsesApiResponse =
            response.json().await.map_err(|error| AttemptFailure {
                message: format!("failed to parse Responses API response: {}", error),
                eligible_for_fallback: true,
            })?;
        let text = parsed.output_text();
        if !text.is_empty() {
            progress.emit(QueryProgressEvent::AssistantText(text.clone()));
        }
        Ok(parsed.into_chat_response(target.model.clone()))
    }

    async fn chat_once_openai_streaming(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        progress: &mut dyn ProgressSink,
    ) -> Result<ChatResponse, AttemptFailure> {
        let response = self.send_request(target, messages, tools, true).await?;
        let mut stream = response.bytes_stream();
        let mut pending = String::new();
        let mut content = String::new();
        let mut finish_reason = None;
        let mut response_id = String::new();
        let mut response_object = "chat.completion.chunk".to_string();
        let mut response_created = Utc::now().timestamp();
        let mut response_model = target.model.clone();
        let mut tool_calls = HashMap::<usize, StreamingToolCall>::new();
        let mut done = false;

        while !done {
            let Some(chunk) = stream.next().await else {
                break;
            };
            let chunk = chunk.map_err(|error| AttemptFailure {
                message: format!("failed to read streaming response: {}", error),
                eligible_for_fallback: true,
            })?;
            pending.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = pending.find('\n') {
                let raw_line = pending[..line_end].trim().to_string();
                pending.drain(..=line_end);
                if raw_line.is_empty() || !raw_line.starts_with("data: ") {
                    continue;
                }

                let payload = &raw_line[6..];
                if payload == "[DONE]" {
                    done = true;
                    break;
                }

                let parsed: StreamChunk =
                    serde_json::from_str(payload).map_err(|error| AttemptFailure {
                        message: format!("failed to parse OpenAI-style stream chunk: {}", error),
                        eligible_for_fallback: true,
                    })?;
                if response_id.is_empty() {
                    response_id = parsed.id.clone();
                    response_object = parsed.object.clone();
                    response_created = parsed.created;
                    response_model = parsed.model.clone();
                }

                for choice in parsed.choices {
                    if let Some(delta) = choice.delta.content {
                        content.push_str(&delta);
                        progress.emit(QueryProgressEvent::AssistantText(delta));
                    }
                    if let Some(delta_tool_calls) = choice.delta.tool_calls {
                        for delta_call in delta_tool_calls {
                            let entry = tool_calls
                                .entry(delta_call.index)
                                .or_insert_with(StreamingToolCall::default);
                            if let Some(id) = delta_call.id {
                                entry.id = id;
                            }
                            if let Some(function) = delta_call.function {
                                if let Some(name) = function.name {
                                    entry.name = name;
                                }
                                if let Some(arguments) = function.arguments {
                                    entry.arguments.push_str(&arguments);
                                }
                            }
                        }
                    }
                    if choice.finish_reason.is_some() {
                        finish_reason = choice.finish_reason;
                    }
                }
            }
        }

        let mut final_tool_calls = tool_calls
            .into_iter()
            .map(|(index, call)| finalize_streaming_tool_call(index, call))
            .collect::<Result<Vec<_>, _>>()?;
        final_tool_calls.sort_by_key(|call| call.0);
        let final_tool_calls = final_tool_calls
            .into_iter()
            .map(|(_, call)| call)
            .collect::<Vec<_>>();
        for tool_call in &final_tool_calls {
            progress.emit(QueryProgressEvent::ToolCall(tool_call.clone()));
        }

        let mut message = ChatMessage::assistant(content);
        if !final_tool_calls.is_empty() {
            message.tool_calls = Some(
                final_tool_calls
                    .iter()
                    .map(runtime_tool_call_to_api_value)
                    .collect(),
            );
        }

        Ok(ChatResponse {
            id: if response_id.is_empty() {
                format!("stream-{}", Utc::now().timestamp_millis())
            } else {
                response_id
            },
            object: response_object,
            created: response_created,
            model: response_model,
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason,
            }],
            usage: None,
        })
    }

    async fn chat_once_anthropic_streaming(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        progress: &mut dyn ProgressSink,
    ) -> Result<ChatResponse, AttemptFailure> {
        let response = self.send_request(target, messages, tools, true).await?;
        let mut stream = response.bytes_stream();
        let mut pending = String::new();
        let mut current_event = String::new();
        let mut content = String::new();
        let mut thinking = String::new();
        let mut finish_reason = None;
        let mut response_id = String::new();
        let mut response_model = target.model.clone();
        let mut usage = None;
        let mut blocks = Vec::<StreamingAnthropicBlock>::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| AttemptFailure {
                message: format!("failed to read streaming response: {}", error),
                eligible_for_fallback: true,
            })?;
            pending.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = pending.find('\n') {
                let raw_line = pending[..line_end].trim_end_matches('\r').to_string();
                pending.drain(..=line_end);

                if raw_line.is_empty() {
                    continue;
                }

                if let Some(event_name) = raw_line.strip_prefix("event: ") {
                    current_event = event_name.to_string();
                    continue;
                }

                let Some(payload) = raw_line.strip_prefix("data: ") else {
                    continue;
                };
                if payload == "[DONE]" {
                    continue;
                }

                let event: AnthropicStreamEnvelope =
                    serde_json::from_str(payload).map_err(|error| AttemptFailure {
                        message: format!("failed to parse Anthropic stream event: {}", error),
                        eligible_for_fallback: true,
                    })?;
                let event_type = if current_event.is_empty() {
                    event.event_type.clone()
                } else {
                    current_event.clone()
                };

                match event_type.as_str() {
                    "message_start" => {
                        if let Some(message) = event.message {
                            response_id = message.id;
                            response_model = message.model;
                            usage = message.usage.map(|usage| Usage {
                                prompt_tokens: usage.input_tokens,
                                completion_tokens: usage.output_tokens,
                                total_tokens: usage.input_tokens + usage.output_tokens,
                            });
                        }
                    }
                    "content_block_start" => {
                        let index = event.index.unwrap_or(blocks.len());
                        ensure_anthropic_block_len(&mut blocks, index);
                        blocks[index] = match event.content_block {
                            Some(AnthropicStreamContentBlock::Text { text, .. }) => {
                                StreamingAnthropicBlock::Text(text.unwrap_or_default())
                            }
                            Some(AnthropicStreamContentBlock::Thinking {
                                thinking: text, ..
                            }) => StreamingAnthropicBlock::Thinking(text.unwrap_or_default()),
                            Some(AnthropicStreamContentBlock::ToolUse {
                                id, name, input, ..
                            }) => StreamingAnthropicBlock::ToolUse {
                                id: id.unwrap_or_default(),
                                name: name.unwrap_or_default(),
                                input_json: input
                                    .map(|value| value.to_string())
                                    .unwrap_or_default(),
                            },
                            None => StreamingAnthropicBlock::Empty,
                        };
                    }
                    "content_block_delta" => {
                        let Some(index) = event.index else {
                            continue;
                        };
                        ensure_anthropic_block_len(&mut blocks, index);
                        if let Some(delta) = event.delta {
                            match delta {
                                AnthropicStreamDelta::TextDelta { text, .. } => {
                                    if let StreamingAnthropicBlock::Text(existing) =
                                        &mut blocks[index]
                                    {
                                        existing.push_str(&text);
                                    } else {
                                        blocks[index] = StreamingAnthropicBlock::Text(text.clone());
                                    }
                                    content.push_str(&text);
                                    progress.emit(QueryProgressEvent::AssistantText(text));
                                }
                                AnthropicStreamDelta::ThinkingDelta { thinking: text, .. } => {
                                    if let StreamingAnthropicBlock::Thinking(existing) =
                                        &mut blocks[index]
                                    {
                                        existing.push_str(&text);
                                    } else {
                                        blocks[index] =
                                            StreamingAnthropicBlock::Thinking(text.clone());
                                    }
                                    thinking.push_str(&text);
                                    progress.emit(QueryProgressEvent::ThinkingText(text));
                                }
                                AnthropicStreamDelta::InputJsonDelta { partial_json, .. } => {
                                    if let StreamingAnthropicBlock::ToolUse { input_json, .. } =
                                        &mut blocks[index]
                                    {
                                        input_json.push_str(&partial_json);
                                    }
                                }
                                AnthropicStreamDelta::Unknown => {}
                            }
                        }
                    }
                    "content_block_stop" => {}
                    "message_delta" => {
                        if let Some(delta) = event.message_delta {
                            finish_reason = delta.stop_reason.or(finish_reason);
                            if let Some(delta_usage) = delta.usage {
                                usage = Some(Usage {
                                    prompt_tokens: delta_usage.input_tokens,
                                    completion_tokens: delta_usage.output_tokens,
                                    total_tokens: delta_usage.input_tokens
                                        + delta_usage.output_tokens,
                                });
                            }
                        }
                    }
                    "message_stop" => break,
                    _ => {}
                }
            }
        }

        let final_tool_calls = blocks
            .into_iter()
            .filter_map(|block| match block {
                StreamingAnthropicBlock::ToolUse {
                    id,
                    name,
                    input_json,
                } => Some((id, name, input_json)),
                _ => None,
            })
            .map(|(id, name, input_json)| {
                let arguments =
                    serde_json::from_str(&input_json).unwrap_or_else(|_| serde_json::json!({}));
                Ok(crate::runtime::RuntimeToolCall {
                    id,
                    name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, AttemptFailure>>()?;

        for tool_call in &final_tool_calls {
            progress.emit(QueryProgressEvent::ToolCall(tool_call.clone()));
        }

        let mut message = ChatMessage::assistant(content);
        if !final_tool_calls.is_empty() {
            message.tool_calls = Some(
                final_tool_calls
                    .iter()
                    .map(runtime_tool_call_to_api_value)
                    .collect(),
            );
        }

        let _ = thinking;

        Ok(ChatResponse {
            id: if response_id.is_empty() {
                format!("anthropic-stream-{}", Utc::now().timestamp_millis())
            } else {
                response_id
            },
            object: "message".to_string(),
            created: Utc::now().timestamp(),
            model: response_model,
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason,
            }],
            usage,
        })
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
            ApiProtocol::Responses => self.send_responses_request(target, messages).await,
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
            response_format: self.settings.api.response_format.clone(),
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

    async fn send_responses_request(
        &self,
        target: &ResolvedApiTarget,
        messages: &[ChatMessage],
    ) -> anyhow::Result<reqwest::Response> {
        let request = ResponsesApiRequest {
            model: target.model.clone(),
            input: messages
                .iter()
                .filter(|message| message.role != "tool")
                .map(ResponsesInputMessage::from_chat_message)
                .collect(),
            max_output_tokens: self.settings.api.max_tokens,
            stream: false,
        };

        let url = build_api_url(&target.base_url, "/v1/responses");
        let mut builder = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json");

        if let Some(api_key) = target.api_key.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.header("Authorization", format!("Bearer {}", api_key));
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
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ResponsesApiRequest {
    model: String,
    input: Vec<ResponsesInputMessage>,
    max_output_tokens: usize,
    stream: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ResponsesInputMessage {
    role: String,
    content: String,
}

impl ResponsesInputMessage {
    fn from_chat_message(message: &ChatMessage) -> Self {
        Self {
            role: message.role.clone(),
            content: message.content.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ResponsesApiResponse {
    id: String,
    #[serde(default = "responses_object")]
    object: String,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<ResponsesOutputItem>,
    #[serde(default)]
    usage: Option<ResponsesUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResponsesOutputItem {
    #[serde(default)]
    content: Vec<ResponsesOutputContent>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResponsesOutputContent {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResponsesUsage {
    #[serde(default)]
    input_tokens: usize,
    #[serde(default)]
    output_tokens: usize,
    #[serde(default)]
    total_tokens: Option<usize>,
}

fn responses_object() -> String {
    "response".to_string()
}

impl ResponsesApiResponse {
    fn output_text(&self) -> String {
        self.output_text.clone().unwrap_or_else(|| {
            self.output
                .iter()
                .flat_map(|item| item.content.iter())
                .filter_map(|block| block.text.clone())
                .collect::<Vec<_>>()
                .join("")
        })
    }

    fn into_chat_response(self, model: String) -> ChatResponse {
        let text = self.output_text();
        ChatResponse {
            id: self.id,
            object: self.object,
            created: self.created_at.unwrap_or_else(|| Utc::now().timestamp()),
            model,
            choices: vec![Choice {
                index: 0,
                message: ChatMessage::assistant(text),
                finish_reason: Some("stop".to_string()),
            }],
            usage: self.usage.map(|usage| Usage {
                prompt_tokens: usage.input_tokens,
                completion_tokens: usage.output_tokens,
                total_tokens: usage
                    .total_tokens
                    .unwrap_or(usage.input_tokens + usage.output_tokens),
            }),
        }
    }
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

#[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamDeltaToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamDeltaToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<StreamDeltaFunction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamDeltaFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Default)]
struct StreamingToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicStreamEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    delta: Option<AnthropicStreamDelta>,
    #[serde(default)]
    content_block: Option<AnthropicStreamContentBlock>,
    #[serde(default)]
    message: Option<AnthropicStreamMessageStart>,
    #[serde(default)]
    message_delta: Option<AnthropicStreamMessageDelta>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicStreamMessageStart {
    id: String,
    model: String,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicStreamMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamContentBlock {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: Option<String>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        input: Option<serde_json::Value>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone)]
enum StreamingAnthropicBlock {
    Empty,
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

pub type AnthropicClient = ApiClient;

fn finalize_streaming_tool_call(
    index: usize,
    call: StreamingToolCall,
) -> Result<(usize, crate::runtime::RuntimeToolCall), AttemptFailure> {
    let arguments = if call.arguments.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&call.arguments).map_err(|error| AttemptFailure {
            message: format!(
                "failed to parse streamed tool call arguments for {}: {}",
                call.name, error
            ),
            eligible_for_fallback: true,
        })?
    };
    Ok((
        index,
        crate::runtime::RuntimeToolCall {
            id: if call.id.is_empty() {
                format!("call_{}", index)
            } else {
                call.id
            },
            name: call.name,
            arguments,
        },
    ))
}

fn ensure_anthropic_block_len(blocks: &mut Vec<StreamingAnthropicBlock>, index: usize) {
    if blocks.len() <= index {
        blocks.resize(index + 1, StreamingAnthropicBlock::Empty);
    }
}

fn runtime_tool_call_to_api_value(call: &crate::runtime::RuntimeToolCall) -> serde_json::Value {
    serde_json::json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string())
        }
    })
}

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

#[allow(dead_code)]
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
