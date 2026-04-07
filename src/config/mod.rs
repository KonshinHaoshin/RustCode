//! Configuration Module

pub mod api_config;
pub mod mcp_config;

pub use api_config::{
    ApiConfig, ApiProtocol, ApiProvider, FallbackConfig, FallbackTarget, ResolvedApiTarget,
};
pub use mcp_config::{McpConfig, McpServerStatus};

use crate::compact::CompactSettings;
use crate::permissions::{PermissionMode, PermissionsSettings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    /// Compact settings
    pub compact: CompactSettings,
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
            compact: CompactSettings::default(),
            onboarding: OnboardingSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectLocalSettings {
    pub permissions: Option<ProjectLocalPermissions>,
    pub session: Option<ProjectLocalSessionSettings>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectLocalPermissions {
    pub mode: Option<PermissionMode>,
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
    pub ask_tools: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectLocalSessionSettings {
    pub auto_restore_last_session: Option<bool>,
    pub persist_transcript: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectPermissionRuleKind {
    Allow,
    Deny,
    Ask,
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

pub fn project_state_dir(cwd: Option<&Path>) -> Option<PathBuf> {
    project_rustcode_dir(cwd).map(|dir| dir.join("state"))
}

pub fn project_tasks_dir(cwd: Option<&Path>) -> Option<PathBuf> {
    project_state_dir(cwd).map(|dir| dir.join("tasks"))
}

pub fn project_file_history_dir(cwd: Option<&Path>) -> Option<PathBuf> {
    project_state_dir(cwd).map(|dir| dir.join("file-history"))
}

pub fn project_permission_events_path(cwd: Option<&Path>) -> Option<PathBuf> {
    project_state_dir(cwd).map(|dir| dir.join("permission-events.json"))
}

impl Settings {
    /// Load global settings from file without project-local overrides.
    pub fn load_global() -> anyhow::Result<Self> {
        let config_path = global_settings_path();
        let legacy_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude-code")
            .join("settings.json");

        let settings = if config_path.exists() {
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

        Ok(settings)
    }

    /// Load settings from file with project-local overrides.
    pub fn load() -> anyhow::Result<Self> {
        let mut settings = Self::load_global()?;
        if let Some(local_settings) = load_project_local_settings(None)? {
            settings.permissions =
                merge_permissions(&settings.permissions, local_settings.permissions.as_ref());
            settings.session = merge_session(&settings.session, local_settings.session.as_ref());
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
            "compact.enabled" => settings.compact.enabled = value.parse().unwrap_or(true),
            "compact.auto_compact" => settings.compact.auto_compact = value.parse().unwrap_or(true),
            "compact.max_turns_before_compact" => {
                settings.compact.max_turns_before_compact = value.parse().unwrap_or(24)
            }
            "compact.max_tokens_before_compact" => {
                settings.compact.max_tokens_before_compact = value.parse().unwrap_or(32_000)
            }
            "compact.preserve_recent_turns" => {
                settings.compact.preserve_recent_turns = value.parse().unwrap_or(4)
            }
            "compact.summary_model" => {
                let trimmed = value.trim();
                settings.compact.summary_model = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
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
    permissions: &ProjectLocalPermissions,
) -> anyhow::Result<()> {
    mutate_project_local_settings(cwd, |local_settings| {
        local_settings.permissions = Some(permissions.clone());
    })
    .map(|_| ())
}

pub fn load_project_local_settings(
    cwd: Option<&Path>,
) -> anyhow::Result<Option<ProjectLocalSettings>> {
    let Some(local_path) = project_local_settings_path(cwd).filter(|path| path.exists()) else {
        return Ok(None);
    };
    let content = std::fs::read_to_string(local_path)?;
    let local_settings = serde_json::from_str(&content)?;
    Ok(Some(local_settings))
}

pub fn add_project_local_permission_rule(
    cwd: Option<&Path>,
    kind: ProjectPermissionRuleKind,
    tool_name: &str,
) -> anyhow::Result<ProjectLocalPermissions> {
    mutate_project_local_settings(cwd, |local_settings| {
        let permissions = local_settings
            .permissions
            .get_or_insert_with(Default::default);
        project_permissions_remove(permissions, tool_name);
        project_permissions_push(permissions.rules_mut(kind), tool_name);
    })
    .map(|settings| settings.permissions.unwrap_or_default())
}

pub fn remove_project_local_permission_rule(
    cwd: Option<&Path>,
    kind: ProjectPermissionRuleKind,
    tool_name: &str,
) -> anyhow::Result<ProjectLocalPermissions> {
    mutate_project_local_settings(cwd, |local_settings| {
        let permissions = local_settings
            .permissions
            .get_or_insert_with(Default::default);
        permissions
            .rules_mut(kind)
            .retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
    })
    .map(|settings| settings.permissions.unwrap_or_default())
}

pub fn mutate_project_local_settings<F>(
    cwd: Option<&Path>,
    mutate: F,
) -> anyhow::Result<ProjectLocalSettings>
where
    F: FnOnce(&mut ProjectLocalSettings),
{
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

    mutate(&mut local_settings);
    std::fs::write(local_path, serde_json::to_string_pretty(&local_settings)?)?;
    Ok(local_settings)
}

fn merge_permissions(
    global: &PermissionsSettings,
    local: Option<&ProjectLocalPermissions>,
) -> PermissionsSettings {
    let Some(local) = local else {
        return global.clone();
    };

    PermissionsSettings {
        mode: local.mode.unwrap_or(global.mode),
        allow_tools: merge_rule_lists(&global.allow_tools, &local.allow_tools),
        deny_tools: merge_rule_lists(&global.deny_tools, &local.deny_tools),
        ask_tools: merge_rule_lists(&global.ask_tools, &local.ask_tools),
    }
}

fn merge_session(
    global: &SessionSettings,
    local: Option<&ProjectLocalSessionSettings>,
) -> SessionSettings {
    let Some(local) = local else {
        return global.clone();
    };

    SessionSettings {
        auto_restore_last_session: local
            .auto_restore_last_session
            .unwrap_or(global.auto_restore_last_session),
        persist_transcript: local
            .persist_transcript
            .unwrap_or(global.persist_transcript),
    }
}

fn merge_rule_lists(global: &[String], local: &[String]) -> Vec<String> {
    let mut merged = global.to_vec();
    for rule in local {
        project_permissions_push(&mut merged, rule);
    }
    merged
}

fn project_permissions_remove(permissions: &mut ProjectLocalPermissions, tool_name: &str) {
    permissions
        .allow_tools
        .retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
    permissions
        .deny_tools
        .retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
    permissions
        .ask_tools
        .retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
}

fn project_permissions_push(rules: &mut Vec<String>, tool_name: &str) {
    if !rules
        .iter()
        .any(|rule| rule.eq_ignore_ascii_case(tool_name))
    {
        rules.push(tool_name.to_string());
    }
}

impl ProjectLocalPermissions {
    fn rules_mut(&mut self, kind: ProjectPermissionRuleKind) -> &mut Vec<String> {
        match kind {
            ProjectPermissionRuleKind::Allow => &mut self.allow_tools,
            ProjectPermissionRuleKind::Deny => &mut self.deny_tools,
            ProjectPermissionRuleKind::Ask => &mut self.ask_tools,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_permissions_unions_rules_and_prefers_local_mode() {
        let global = PermissionsSettings {
            mode: PermissionMode::DenyAll,
            allow_tools: vec!["file_read".to_string()],
            deny_tools: vec!["execute_command".to_string()],
            ask_tools: vec!["search".to_string()],
        };
        let local = ProjectLocalPermissions {
            mode: Some(PermissionMode::Ask),
            allow_tools: vec!["file_write".to_string()],
            deny_tools: vec!["file_edit".to_string()],
            ask_tools: vec!["search".to_string(), "list_files".to_string()],
            extra: Map::new(),
        };

        let merged = merge_permissions(&global, Some(&local));

        assert_eq!(merged.mode, PermissionMode::Ask);
        assert_eq!(merged.allow_tools, vec!["file_read", "file_write"]);
        assert_eq!(merged.deny_tools, vec!["execute_command", "file_edit"]);
        assert_eq!(merged.ask_tools, vec!["search", "list_files"]);
    }

    #[test]
    fn merge_session_prefers_local_overrides() {
        let global = SessionSettings {
            auto_restore_last_session: false,
            persist_transcript: false,
        };
        let local = ProjectLocalSessionSettings {
            auto_restore_last_session: Some(true),
            persist_transcript: None,
            extra: Map::new(),
        };

        let merged = merge_session(&global, Some(&local));

        assert!(merged.auto_restore_last_session);
        assert!(!merged.persist_transcript);
    }
}
