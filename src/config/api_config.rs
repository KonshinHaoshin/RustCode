//! API Configuration

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProvider {
    DeepSeek,
    OpenAI,
    Anthropic,
    XAI,
    Gemini,
    DashScope,
    OpenRouter,
    Ollama,
    Custom,
}

impl ApiProvider {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "deepseek" => Some(Self::DeepSeek),
            "openai" => Some(Self::OpenAI),
            "anthropic" | "claude" => Some(Self::Anthropic),
            "xai" | "grok" => Some(Self::XAI),
            "gemini" | "google" => Some(Self::Gemini),
            "dashscope" | "qwen" => Some(Self::DashScope),
            "openrouter" => Some(Self::OpenRouter),
            "ollama" => Some(Self::Ollama),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::XAI => "xai",
            Self::Gemini => "gemini",
            Self::DashScope => "dashscope",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }

    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::DeepSeek => "https://api.deepseek.com",
            Self::OpenAI => "https://api.openai.com",
            Self::Anthropic => "https://api.anthropic.com",
            Self::XAI => "https://api.x.ai",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai",
            Self::DashScope => "https://dashscope.aliyuncs.com/compatible-mode",
            Self::OpenRouter => "https://openrouter.ai/api",
            Self::Ollama => "http://127.0.0.1:11434",
            Self::Custom => "https://api.example.com",
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek-chat",
            Self::OpenAI => "gpt-4.1-mini",
            Self::Anthropic => "claude-3-5-sonnet-20241022",
            Self::XAI => "grok-4-1-fast-non-reasoning",
            Self::Gemini => "gemini-2.5-pro",
            Self::DashScope => "qwen-plus",
            Self::OpenRouter => "openai/gpt-4.1-mini",
            Self::Ollama => "llama3.1",
            Self::Custom => "custom-model",
        }
    }

    pub fn default_protocol(&self) -> ApiProtocol {
        match self {
            Self::Anthropic => ApiProtocol::Anthropic,
            Self::XAI => ApiProtocol::Responses,
            _ => ApiProtocol::OpenAi,
        }
    }
}

impl Default for ApiProvider {
    fn default() -> Self {
        Self::DeepSeek
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProtocol {
    OpenAi,
    Anthropic,
    Responses,
}

impl ApiProtocol {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" | "open_ai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "responses" | "response" => Some(Self::Responses),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Responses => "responses",
        }
    }

    pub fn supports_tool_calling(&self) -> bool {
        matches!(self, Self::OpenAi | Self::Anthropic | Self::Responses)
    }
}

impl Default for ApiProtocol {
    fn default() -> Self {
        Self::OpenAi
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FallbackTarget {
    pub provider: ApiProvider,
    pub protocol: Option<ApiProtocol>,
    pub custom_provider_name: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedApiTarget {
    pub provider: ApiProvider,
    pub protocol: ApiProtocol,
    pub provider_label: String,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
}

impl ResolvedApiTarget {
    pub fn display_name(&self) -> String {
        format!("{}/{}", self.provider_label, self.model)
    }

    pub fn supports_tool_calling(&self) -> bool {
        self.protocol.supports_tool_calling()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FallbackConfig {
    pub enabled: bool,
    pub chain: Vec<FallbackTarget>,
}

/// API configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    /// Selected API provider preset
    pub provider: ApiProvider,
    /// Selected protocol for requests. For preset providers this follows the provider default.
    pub protocol: ApiProtocol,
    /// Optional label for a custom provider
    pub custom_provider_name: Option<String>,
    /// API key (can be set via environment variable)
    pub api_key: Option<String>,
    /// Base URL for API requests
    pub base_url: String,
    /// Maximum tokens per request
    pub max_tokens: usize,
    /// Request timeout in seconds
    pub timeout: u64,
    /// Enable streaming responses
    pub streaming: bool,
    /// Beta headers to include
    pub beta_headers: Vec<String>,
    /// Optional OpenAI-compatible response_format object.
    pub response_format: Option<serde_json::Value>,
    /// Fallback request chain
    pub fallback: FallbackConfig,
}

impl Default for ApiConfig {
    fn default() -> Self {
        let provider = ApiProvider::default();
        let protocol = provider.default_protocol();

        Self {
            provider,
            protocol,
            custom_provider_name: None,
            api_key: None,
            base_url: provider.default_base_url().to_string(),
            max_tokens: 4096,
            timeout: 120,
            streaming: true,
            beta_headers: Vec::new(),
            response_format: None,
            fallback: FallbackConfig::default(),
        }
    }
}

impl ApiConfig {
    fn sanitize_optional(value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn parse_fallback_csv(value: &str) -> anyhow::Result<Vec<FallbackTarget>> {
        let mut chain = Vec::new();

        for item in value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let mut parts = item.splitn(2, ':');
            let provider = parts
                .next()
                .and_then(ApiProvider::parse)
                .ok_or_else(|| anyhow::anyhow!("Unsupported fallback provider entry: {}", item))?;
            let model = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing model for fallback entry: {}", item))?;

            chain.push(FallbackTarget {
                provider,
                model: model.trim().to_string(),
                ..Default::default()
            });
        }

        Ok(chain)
    }

    pub fn provider(&self) -> ApiProvider {
        self.provider
    }

    pub fn protocol(&self) -> ApiProtocol {
        if self.provider() == ApiProvider::Custom {
            self.protocol
        } else {
            self.provider().default_protocol()
        }
    }

    pub fn provider_label(&self) -> String {
        match self.provider() {
            ApiProvider::Custom => self
                .custom_provider_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "custom".to_string()),
            provider => provider.as_str().to_string(),
        }
    }

    pub fn default_model(&self) -> &'static str {
        self.provider().default_model()
    }

    pub fn set_provider(&mut self, value: &str) -> anyhow::Result<()> {
        let provider = ApiProvider::parse(value)
            .ok_or_else(|| anyhow::anyhow!("Unsupported API provider: {}", value))?;
        self.provider = provider;
        self.protocol = provider.default_protocol();
        self.base_url = provider.default_base_url().to_string();
        Ok(())
    }

    pub fn set_protocol(&mut self, value: &str) -> anyhow::Result<()> {
        let protocol = ApiProtocol::parse(value)
            .ok_or_else(|| anyhow::anyhow!("Unsupported API protocol: {}", value))?;
        self.protocol = protocol;
        Ok(())
    }

    pub fn set_fallback_chain_from_str(&mut self, value: &str) -> anyhow::Result<()> {
        let trimmed = value.trim();
        self.fallback.chain = if trimmed.is_empty() {
            Vec::new()
        } else if trimmed.starts_with('[') {
            serde_json::from_str(trimmed)?
        } else {
            Self::parse_fallback_csv(trimmed)?
        };
        Ok(())
    }

    /// Get the configured API key.
    pub fn get_api_key(&self) -> Option<String> {
        Self::sanitize_optional(self.api_key.clone())
    }

    /// Get the configured base URL.
    pub fn get_base_url(&self) -> String {
        let base_url = self.base_url.trim();
        if base_url.is_empty() {
            self.provider().default_base_url().to_string()
        } else {
            base_url.to_string()
        }
    }

    /// Get the model ID for the given model name
    pub fn get_model_id(&self, model: &str) -> String {
        match model.trim() {
            "" => self.default_model().to_string(),
            "chat" if self.provider() == ApiProvider::DeepSeek => "deepseek-chat".to_string(),
            "reasoner" if self.provider() == ApiProvider::DeepSeek => {
                "deepseek-reasoner".to_string()
            }
            "mini" if self.provider() == ApiProvider::OpenAI => "gpt-4.1-mini".to_string(),
            "opus" => "claude-3-opus-20240229".to_string(),
            "sonnet" => "claude-3-5-sonnet-20241022".to_string(),
            "haiku" => "claude-3-5-haiku-20241022".to_string(),
            value => value.to_string(),
        }
    }

    pub fn active_target(&self, configured_model: &str) -> ResolvedApiTarget {
        let provider = self.provider();
        let protocol = self.protocol();
        let provider_label = self.provider_label();

        ResolvedApiTarget {
            provider,
            protocol,
            provider_label,
            api_key: self.get_api_key(),
            base_url: self.get_base_url(),
            model: self.get_model_id(configured_model),
        }
    }

    pub fn fallback_targets(&self) -> Vec<ResolvedApiTarget> {
        if !self.fallback.enabled {
            return Vec::new();
        }

        self.fallback
            .chain
            .iter()
            .map(|target| {
                let provider = target.provider;
                let protocol = target.protocol.unwrap_or_else(|| {
                    if provider == ApiProvider::Custom {
                        self.protocol()
                    } else {
                        provider.default_protocol()
                    }
                });
                let provider_label = match provider {
                    ApiProvider::Custom => target
                        .custom_provider_name
                        .clone()
                        .or_else(|| self.custom_provider_name.clone())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "custom".to_string()),
                    _ => provider.as_str().to_string(),
                };

                ResolvedApiTarget {
                    provider,
                    protocol,
                    provider_label,
                    api_key: Self::sanitize_optional(target.api_key.clone())
                        .or_else(|| self.get_api_key()),
                    base_url: target
                        .base_url
                        .clone()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| provider.default_base_url().to_string()),
                    model: self.get_model_id(&target.model),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiConfig, ApiProtocol, ApiProvider};

    #[test]
    fn default_api_config_uses_builtin_defaults() {
        let config = ApiConfig::default();

        assert_eq!(config.provider(), ApiProvider::DeepSeek);
        assert_eq!(config.protocol(), ApiProtocol::OpenAi);
        assert_eq!(config.get_base_url(), "https://api.deepseek.com");
        assert_eq!(config.get_api_key(), None);
    }

    #[test]
    fn blank_configured_api_key_is_treated_as_missing() {
        let config = ApiConfig {
            api_key: Some("   ".to_string()),
            ..ApiConfig::default()
        };

        assert_eq!(config.get_api_key(), None);
    }

    #[test]
    fn fallback_target_uses_its_own_key_before_primary_key() {
        let mut config = ApiConfig {
            api_key: Some("primary-key".to_string()),
            ..ApiConfig::default()
        };
        config.fallback.enabled = true;
        config.fallback.chain.push(super::FallbackTarget {
            provider: ApiProvider::Custom,
            protocol: Some(ApiProtocol::OpenAi),
            custom_provider_name: Some("backup".to_string()),
            api_key: Some("fallback-key".to_string()),
            base_url: Some("https://example.com".to_string()),
            model: "backup-model".to_string(),
        });

        let fallback = config.fallback_targets();

        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].api_key.as_deref(), Some("fallback-key"));
    }

    #[test]
    fn xai_defaults_to_responses_protocol() {
        assert_eq!(ApiProvider::XAI.default_protocol(), ApiProtocol::Responses);
    }
}
