//! Configuration Module

pub mod api_config;
pub mod mcp_config;

pub use api_config::{
    ApiConfig, ApiProtocol, ApiProvider, FallbackConfig, FallbackTarget, ResolvedApiTarget,
};
pub use mcp_config::{McpConfig, McpServerStatus};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// API configuration
    pub api: ApiConfig,
    /// MCP server configurations
    pub mcp_servers: Vec<McpConfig>,
    /// Model selection
    pub model: String,
    /// Enable verbose logging
    pub verbose: bool,
    /// Working directory
    pub working_dir: PathBuf,
    /// Memory settings
    pub memory: MemorySettings,
    /// Voice settings
    pub voice: VoiceSettings,
    /// Plugin settings
    pub plugins: PluginSettings,
    /// First-run onboarding state
    pub onboarding: OnboardingSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySettings {
    /// Enable memory persistence
    pub enabled: bool,
    /// Memory file path
    pub path: PathBuf,
    /// Auto-consolidation interval (hours)
    pub consolidation_interval: u64,
    /// Maximum memories to keep
    pub max_memories: usize,
}

impl Default for MemorySettings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        Self {
            enabled: true,
            path: home.join(".rustcode").join("memory.json"),
            consolidation_interval: 24,
            max_memories: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    /// Enable voice input
    pub enabled: bool,
    /// Push-to-talk mode
    pub push_to_talk: bool,
    /// Silence detection threshold
    pub silence_threshold: f32,
    /// Sample rate
    pub sample_rate: u32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            push_to_talk: false,
            silence_threshold: 0.01,
            sample_rate: 16000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginSettings {
    /// Enable plugin system
    pub enabled: bool,
    /// Plugin directory
    pub plugin_dir: PathBuf,
    /// Auto-update plugins
    pub auto_update: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OnboardingSettings {
    /// Whether the user has completed the first-run onboarding flow
    pub has_completed_onboarding: bool,
    /// Version that last completed onboarding
    pub last_onboarding_version: String,
}

impl Default for PluginSettings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        Self {
            enabled: true,
            plugin_dir: home.join(".rustcode").join("plugins"),
            auto_update: true,
        }
    }
}

impl Default for OnboardingSettings {
    fn default() -> Self {
        Self {
            has_completed_onboarding: false,
            last_onboarding_version: String::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        let api = ApiConfig::default();
        let model = api.default_model().to_string();

        Self {
            api,
            mcp_servers: Vec::new(),
            model,
            verbose: false,
            working_dir: PathBuf::from("."),
            memory: MemorySettings::default(),
            voice: VoiceSettings::default(),
            plugins: PluginSettings::default(),
            onboarding: OnboardingSettings::default(),
        }
    }
}

impl Settings {
    /// Load settings from file
    pub fn load() -> anyhow::Result<Self> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_path = home.join(".rustcode").join("settings.json");
        let legacy_path = home.join(".claude-code").join("settings.json");

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let settings: Settings = serde_json::from_str(&content)?;
            Ok(settings)
        } else if legacy_path.exists() {
            let content = std::fs::read_to_string(&legacy_path)?;
            let settings: Settings = serde_json::from_str(&content)?;
            settings.save()?;
            Ok(settings)
        } else {
            let settings = Settings::default();
            settings.save()?;
            Ok(settings)
        }
    }

    /// Save settings to file
    pub fn save(&self) -> anyhow::Result<()> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".rustcode");
        std::fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join("settings.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;

        Ok(())
    }

    /// Set a configuration value
    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        let mut settings = Self::load()?;

        match key {
            "model" => settings.model = value.to_string(),
            "verbose" => settings.verbose = value.parse().unwrap_or(false),
            "provider" | "api.provider" | "api_provider" => {
                settings.api.set_provider(value)?;
                settings.model = settings.api.default_model().to_string();
            }
            "protocol" | "api.protocol" | "api_protocol" => settings.api.set_protocol(value)?,
            "custom_provider_name" | "api.custom_provider_name" | "custom_provider" => {
                settings.api.custom_provider_name = Some(value.to_string())
            }
            "api_key" | "api.api_key" | "api_key_value" => {
                settings.api.api_key = Some(value.to_string())
            }
            "base_url" | "api.base_url" | "api_base_url" => {
                settings.api.base_url = value.to_string()
            }
            "max_tokens" => settings.api.max_tokens = value.parse().unwrap_or(4096),
            "timeout" => settings.api.timeout = value.parse().unwrap_or(120),
            "streaming" => settings.api.streaming = value.parse().unwrap_or(true),
            "fallback.enabled" | "api.fallback.enabled" => {
                settings.api.fallback.enabled = value.parse().unwrap_or(false)
            }
            "fallback.chain" | "api.fallback.chain" => {
                settings.api.set_fallback_chain_from_str(value)?
            }
            "memory.enabled" => settings.memory.enabled = value.parse().unwrap_or(true),
            "voice.enabled" => settings.voice.enabled = value.parse().unwrap_or(false),
            "onboarding.completed" => {
                settings.onboarding.has_completed_onboarding = value.parse().unwrap_or(false)
            }
            _ => return Err(anyhow::anyhow!("Unknown setting: {}", key)),
        }

        settings.save()?;
        Ok(())
    }

    /// Reset settings to defaults
    pub fn reset() -> anyhow::Result<()> {
        let settings = Settings::default();
        settings.save()?;
        Ok(())
    }

    pub fn should_run_onboarding(&self) -> bool {
        !self.onboarding.has_completed_onboarding
    }

    pub fn mark_onboarding_complete(&mut self) {
        self.onboarding.has_completed_onboarding = true;
        self.onboarding.last_onboarding_version = env!("CARGO_PKG_VERSION").to_string();
    }
}
