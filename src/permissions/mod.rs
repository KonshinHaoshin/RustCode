use crate::runtime::types::{RuntimeToolCall, RuntimeToolResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    AllowAll,
    Ask,
    DenyAll,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::AllowAll
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PermissionsSettings {
    pub mode: PermissionMode,
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
    pub ask_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny(String),
    Ask(String),
}

pub trait PermissionGate: Send + Sync {
    fn evaluate_tool_call(&self, call: &RuntimeToolCall) -> PermissionDecision;
}

#[derive(Debug, Clone)]
pub struct SettingsPermissionGate {
    settings: PermissionsSettings,
}

impl SettingsPermissionGate {
    pub fn new(settings: PermissionsSettings) -> Self {
        Self { settings }
    }

    fn matches_rule(rules: &[String], tool_name: &str) -> bool {
        rules.iter().any(|rule| {
            let trimmed = rule.trim();
            trimmed == "*" || trimmed.eq_ignore_ascii_case(tool_name)
        })
    }

    pub fn denied_tool_result(call: &RuntimeToolCall, content: String) -> RuntimeToolResult {
        RuntimeToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            is_error: true,
        }
    }

    pub fn add_allow_rule(&mut self, tool_name: &str) {
        Self::remove_rule(&mut self.settings.deny_tools, tool_name);
        Self::remove_rule(&mut self.settings.ask_tools, tool_name);
        Self::push_unique_rule(&mut self.settings.allow_tools, tool_name);
    }

    pub fn add_deny_rule(&mut self, tool_name: &str) {
        Self::remove_rule(&mut self.settings.allow_tools, tool_name);
        Self::remove_rule(&mut self.settings.ask_tools, tool_name);
        Self::push_unique_rule(&mut self.settings.deny_tools, tool_name);
    }

    pub fn settings(&self) -> &PermissionsSettings {
        &self.settings
    }

    fn remove_rule(rules: &mut Vec<String>, tool_name: &str) {
        rules.retain(|rule| !rule.eq_ignore_ascii_case(tool_name));
    }

    fn push_unique_rule(rules: &mut Vec<String>, tool_name: &str) {
        if !Self::matches_rule(rules, tool_name) {
            rules.push(tool_name.to_string());
        }
    }
}

impl PermissionGate for SettingsPermissionGate {
    fn evaluate_tool_call(&self, call: &RuntimeToolCall) -> PermissionDecision {
        if Self::matches_rule(&self.settings.deny_tools, &call.name) {
            return PermissionDecision::Deny(format!(
                "Tool {} denied by permissions.deny_tools",
                call.name
            ));
        }

        if Self::matches_rule(&self.settings.allow_tools, &call.name) {
            return PermissionDecision::Allow;
        }

        if Self::matches_rule(&self.settings.ask_tools, &call.name) {
            return PermissionDecision::Ask(format!(
                "Tool {} requires explicit approval",
                call.name
            ));
        }

        match self.settings.mode {
            PermissionMode::AllowAll => PermissionDecision::Allow,
            PermissionMode::Ask => {
                PermissionDecision::Ask(format!("Tool {} requires approval", call.name))
            }
            PermissionMode::DenyAll => PermissionDecision::Deny(format!(
                "Tool {} denied by permissions.mode=deny_all",
                call.name
            )),
        }
    }
}
