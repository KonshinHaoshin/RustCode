//! Configuration Module

pub mod api_config;
pub mod mcp_config;

pub use api_config::{
    ApiConfig, ApiProtocol, ApiProvider, FallbackConfig, FallbackTarget, ResolvedApiTarget,
};
pub use mcp_config::{McpConfig, McpServerStatus};

use crate::permissions::PermissionsSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Runtime permission settings
    pub permissions: PermissionsSettings,
    /// Session persistence settings
    pub session: SessionSettings,
    /// First-run onboarding state
    pub onboarding: OnboardingSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// Whether to restore the last session on startup
    pub auto_restore_last_session: bool,
    /// Whether to persist transcript/session state
    pub persist_transcript: bool,
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

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            auto_restore_last_session: true,
            persist_transcript: true,
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
            permissions: PermissionsSettings::default(),
            session: SessionSettings::default(),
            onboarding: OnboardingSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ProjectLocalSettings {
    permissions: Option<PermissionsSettings>,
    session: Option<SessionSettings>,
}

pub fn global_config_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".rustcode")
}

pub fn global_settings_path() -> PathBuf {
    global_config_dir().join("settings.json")
}

pub fn project_root_from(cwd: Option<&Path>) -> Option<PathBuf> {
    let cwd = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())?;
    Some(cwd)
}

pub fn project_rustcode_dir(cwd: Option<&Path>) -> Option<PathBuf> {
    project_root_from(cwd).map(|root| root.join("rustcode"))
}

pub fn project_local_settings_path(cwd: Option<&Path>) -> Option<PathBuf> {
    project_rustcode_dir(cwd).map(|dir| dir.join("settings.local.json"))
}

pub fn project_sessions_dir(cwd: Option<&Path>) -> Option<PathBuf> {
    project_rustcode_dir(cwd).map(|dir| dir.join("sessions"))
}

impl Settings {
    /// Load settings from file
    pub fn load() -> anyhow::Result<Self> {
        let config_path = global_settings_path();
        let legacy_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude-code")
            .join("settings.json");

        let mut settings = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            serde_json::from_str(&content)?
        } else if legacy_path.exists() {
            let content = std::fs::read_to_string(&legacy_path)?;
            let settings: Settings = serde_json::from_str(&content)?;
            settings.save()?;
            settings
        } else {
            let settings = Settings::default();
            settings.save()?;
            settings
        };

        if let Some(local_path) = project_local_settings_path(None).filter(|path| path.exists()) {
            let content = std::fs::read_to_string(local_path)?;
            let local_settings: ProjectLocalSettings = serde_json::from_str(&content)?;
            if let Some(permissions) = local_settings.permissions {
                settings.permissions = permissions;
            }
            if let Some(session) = local_settings.session {
                settings.session = session;
            }
        }

        Ok(settings)
    }

    /// Save settings to file
    pub fn save(&self) -> anyhow::Result<()> {
        let config_dir = global_config_dir();
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
            "permissions.mode" => {
                settings.permissions.mode = match value.trim().to_ascii_lowercase().as_str() {
                    "allow_all" | "allow" => crate::permissions::PermissionMode::AllowAll,
                    "ask" => crate::permissions::PermissionMode::Ask,
                    "deny_all" | "deny" => crate::permissions::PermissionMode::DenyAll,
                    other => {
                        return Err(anyhow::anyhow!("Unsupported permissions mode: {}", other))
                    }
                }
            }
            "permissions.allow_tools" => {
                settings.permissions.allow_tools = value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "permissions.deny_tools" => {
                settings.permissions.deny_tools = value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "permissions.ask_tools" => {
                settings.permissions.ask_tools = value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "session.auto_restore_last_session" => {
                settings.session.auto_restore_last_session = value.parse().unwrap_or(true)
            }
            "session.persist_transcript" => {
                settings.session.persist_transcript = value.parse().unwrap_or(true)
            }
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

pub fn save_project_local_permissions(
    cwd: Option<&Path>,
    permissions: &PermissionsSettings,
) -> anyhow::Result<()> {
    let rustcode_dir = project_rustcode_dir(cwd)
        .ok_or_else(|| anyhow::anyhow!("Unable to determine project-local rustcode directory"))?;
    std::fs::create_dir_all(&rustcode_dir)?;

    let local_path = rustcode_dir.join("settings.local.json");
    let mut local_settings = if local_path.exists() {
        let content = std::fs::read_to_string(&local_path)?;
        serde_json::from_str::<ProjectLocalSettings>(&content)?
    } else {
        ProjectLocalSettings::default()
    };

    local_settings.permissions = Some(permissions.clone());
    std::fs::write(local_path, serde_json::to_string_pretty(&local_settings)?)?;
    Ok(())
}
