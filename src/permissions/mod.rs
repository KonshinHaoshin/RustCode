pub mod events;

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
        rules.iter().any(|rule| Self::rule_matches(rule, tool_name))
    }

    fn rule_matches(rule: &str, tool_name: &str) -> bool {
        let trimmed = rule.trim();
        if trimmed.is_empty() {
            return false;
        }
        if trimmed == "*" {
            return true;
        }

        let normalized_rule = trimmed.to_ascii_lowercase();
        let normalized_tool = tool_name.to_ascii_lowercase();

        wildcard_match(&normalized_rule, &normalized_tool)
    }

    pub fn denied_tool_result(call: &RuntimeToolCall, content: String) -> RuntimeToolResult {
        RuntimeToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            content,
            is_error: true,
            metadata: std::collections::HashMap::new(),
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

        if Self::matches_rule(&self.settings.ask_tools, &call.name) {
            return PermissionDecision::Ask(format!(
                "Tool {} requires explicit approval",
                call.name
            ));
        }

        if Self::matches_rule(&self.settings.allow_tools, &call.name) {
            return PermissionDecision::Allow;
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

fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == candidate;
    }

    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');
    let parts = pattern
        .split('*')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return true;
    }

    let mut position = 0usize;

    for (index, part) in parts.iter().enumerate() {
        let Some(found) = candidate[position..].find(part) else {
            return false;
        };
        let absolute = position + found;

        if index == 0 && !starts_with_wildcard && absolute != 0 {
            return false;
        }

        position = absolute + part.len();
    }

    if !ends_with_wildcard {
        if let Some(last) = parts.last() {
            return candidate.ends_with(last);
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{
        PermissionDecision, PermissionGate, PermissionMode, PermissionsSettings,
        SettingsPermissionGate,
    };
    use crate::runtime::RuntimeToolCall;

    fn call(name: &str) -> RuntimeToolCall {
        RuntimeToolCall {
            id: "call-1".to_string(),
            name: name.to_string(),
            arguments: serde_json::json!({}),
        }
    }

    #[test]
    fn supports_global_and_prefix_rules() {
        let gate = SettingsPermissionGate::new(PermissionsSettings {
            mode: PermissionMode::DenyAll,
            allow_tools: vec!["mcp__*".to_string()],
            deny_tools: vec![],
            ask_tools: vec![],
        });

        assert_eq!(
            gate.evaluate_tool_call(&call("mcp__search")),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn deny_has_priority_over_ask_and_allow() {
        let gate = SettingsPermissionGate::new(PermissionsSettings {
            mode: PermissionMode::AllowAll,
            allow_tools: vec!["mcp__server__*".to_string()],
            deny_tools: vec!["mcp__server__read".to_string()],
            ask_tools: vec!["mcp__server__*".to_string()],
        });

        match gate.evaluate_tool_call(&call("mcp__server__read")) {
            PermissionDecision::Deny(reason) => assert!(reason.contains("deny_tools")),
            other => panic!("expected deny, got {:?}", other),
        }
    }

    #[test]
    fn ask_has_priority_over_allow() {
        let gate = SettingsPermissionGate::new(PermissionsSettings {
            mode: PermissionMode::AllowAll,
            allow_tools: vec!["file_*".to_string()],
            deny_tools: vec![],
            ask_tools: vec!["file_read".to_string()],
        });

        match gate.evaluate_tool_call(&call("file_read")) {
            PermissionDecision::Ask(reason) => assert!(reason.contains("explicit approval")),
            other => panic!("expected ask, got {:?}", other),
        }
    }

    #[test]
    fn supports_infix_wildcard_rules() {
        let gate = SettingsPermissionGate::new(PermissionsSettings {
            mode: PermissionMode::DenyAll,
            allow_tools: vec!["mcp__*__read".to_string()],
            deny_tools: vec![],
            ask_tools: vec![],
        });

        assert_eq!(
            gate.evaluate_tool_call(&call("mcp__filesystem__read")),
            PermissionDecision::Allow
        );
    }
}
