//! Shared onboarding data model used by CLI, TUI, and GUI flows.

use crate::config::{ApiProtocol, ApiProvider, FallbackTarget, Settings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingDraft {
    pub provider: ApiProvider,
    pub protocol: ApiProtocol,
    pub custom_provider_name: Option<String>,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub fallback_enabled: bool,
    pub fallback_chain: Vec<FallbackTarget>,
}

impl OnboardingDraft {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            provider: settings.api.provider(),
            protocol: settings.api.protocol(),
            custom_provider_name: settings.api.custom_provider_name.clone(),
            api_key: settings.api.api_key.clone(),
            base_url: settings.api.base_url.clone(),
            model: settings.model.clone(),
            fallback_enabled: settings.api.fallback.enabled,
            fallback_chain: settings.api.fallback.chain.clone(),
        }
    }

    pub fn provider_label(&self) -> String {
        match self.provider {
            ApiProvider::Custom => self
                .custom_provider_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "custom".to_string()),
            provider => provider.as_str().to_string(),
        }
    }

    pub fn prepare_for_provider_change(&mut self, provider: ApiProvider) {
        let previous_provider = self.provider;
        self.provider = provider;

        if previous_provider != provider {
            self.base_url = provider.default_base_url().to_string();
            self.model = provider.default_model().to_string();
            self.api_key = None;
        }

        if provider == ApiProvider::Custom {
            if self.custom_provider_name.is_none() {
                self.custom_provider_name = Some("custom".to_string());
            }
        } else {
            self.protocol = provider.default_protocol();
            self.custom_provider_name = None;
        }
    }

    pub fn set_protocol(&mut self, protocol: ApiProtocol) {
        self.protocol = protocol;
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = normalize_non_empty(model.into(), self.provider.default_model().to_string());
    }

    pub fn set_base_url(&mut self, base_url: impl Into<String>) {
        self.base_url = normalize_non_empty(
            base_url.into(),
            self.provider.default_base_url().to_string(),
        );
    }

    pub fn set_custom_provider_name(&mut self, value: impl Into<String>) {
        self.custom_provider_name = normalize_optional(value.into());
    }

    pub fn set_api_key(&mut self, value: Option<String>) {
        self.api_key = value.and_then(normalize_optional);
    }

    pub fn set_fallback_enabled(&mut self, enabled: bool) {
        self.fallback_enabled = enabled;
        if !enabled {
            self.fallback_chain.clear();
        }
    }

    pub fn add_fallback_target(&mut self, provider: ApiProvider) -> usize {
        let mut target = FallbackTarget {
            provider,
            protocol: None,
            custom_provider_name: None,
            api_key: None,
            base_url: None,
            model: provider.default_model().to_string(),
        };

        if provider == ApiProvider::Custom {
            target.protocol = Some(ApiProtocol::OpenAi);
            target.custom_provider_name = Some("custom".to_string());
            target.base_url = Some(provider.default_base_url().to_string());
        }

        self.fallback_enabled = true;
        self.fallback_chain.push(target);
        self.fallback_chain.len() - 1
    }

    pub fn fallback_target_label(target: &FallbackTarget) -> String {
        let provider_label = match target.provider {
            ApiProvider::Custom => target
                .custom_provider_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "custom".to_string()),
            provider => provider.as_str().to_string(),
        };

        format!("{}/{}", provider_label, target.model)
    }

    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Primary: {}/{}", self.provider_label(), self.model),
            format!("Base URL: {}", self.base_url),
            format!(
                "API key: {}",
                if self
                    .api_key
                    .as_ref()
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    "saved"
                } else {
                    "empty or env-managed"
                }
            ),
        ];

        if self.fallback_enabled && !self.fallback_chain.is_empty() {
            lines.push("Fallbacks:".to_string());
            lines.extend(
                self.fallback_chain
                    .iter()
                    .enumerate()
                    .map(|(index, target)| {
                        format!("  {}. {}", index + 1, Self::fallback_target_label(target))
                    }),
            );
        } else {
            lines.push("Fallbacks: disabled".to_string());
        }

        lines
    }

    pub fn apply_to_settings(&self, settings: &mut Settings) {
        settings.api.provider = self.provider;
        settings.api.protocol = if self.provider == ApiProvider::Custom {
            self.protocol
        } else {
            self.provider.default_protocol()
        };
        settings.api.custom_provider_name = self.custom_provider_name.clone();
        settings.api.api_key = self.api_key.clone();
        settings.api.base_url = self.base_url.clone();
        settings.model = self.model.clone();
        settings.api.fallback.enabled = self.fallback_enabled && !self.fallback_chain.is_empty();
        settings.api.fallback.chain = if settings.api.fallback.enabled {
            self.fallback_chain.clone()
        } else {
            Vec::new()
        };
    }
}

fn normalize_non_empty(value: String, fallback: String) -> String {
    if value.trim().is_empty() {
        fallback
    } else {
        value.trim().to_string()
    }
}

fn normalize_optional(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
