use crate::{
    api::{ApiClient, ChatMessage},
    compact::prompt::build_compact_messages,
    config::Settings,
    runtime::{QueryTurnResult, RuntimeMessage},
};
use serde::{Deserialize, Serialize};

pub const COMPACT_SUMMARY_PREFIX: &str = "Previous conversation compacted into summary:\n";
pub const MICRO_COMPACT_SUMMARY_PREFIX: &str =
    "Previous conversation micro-compacted into summary:\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactSettings {
    pub enabled: bool,
    pub auto_compact: bool,
    pub reactive_compact: bool,
    pub enable_microcompact: bool,
    pub max_turns_before_compact: usize,
    pub max_tokens_before_compact: usize,
    pub preserve_recent_turns: usize,
    pub reserved_completion_budget: usize,
    pub summary_model: Option<String>,
}

impl Default for CompactSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_compact: true,
            reactive_compact: true,
            enable_microcompact: true,
            max_turns_before_compact: 24,
            max_tokens_before_compact: 32_000,
            preserve_recent_turns: 4,
            reserved_completion_budget: 4_096,
            summary_model: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompactOutcome {
    pub history: Vec<RuntimeMessage>,
    pub summary: String,
    pub was_micro: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBudgetEstimate {
    pub prompt_estimate: usize,
    pub recent_turns_estimate: usize,
    pub tool_context_estimate: usize,
    pub reserved_completion_budget: usize,
    pub remaining_budget: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDecision {
    None,
    MicroCompact,
    FullCompact,
    RejectForOverflow,
}

pub struct CompactService {
    settings: Settings,
    client: ApiClient,
}

impl CompactService {
    pub fn new(settings: Settings) -> Self {
        let mut compact_settings = settings.clone();
        if let Some(summary_model) = settings.compact.summary_model.clone() {
            compact_settings.model = summary_model;
        }

        Self {
            settings,
            client: ApiClient::new(compact_settings),
        }
    }

    pub fn should_auto_compact(
        &self,
        history: &[RuntimeMessage],
        usage_total_tokens: Option<usize>,
    ) -> bool {
        matches!(
            self.compact_decision(history, usage_total_tokens, false),
            CompactDecision::MicroCompact | CompactDecision::FullCompact
        )
    }

    pub fn should_reactive_compact(
        &self,
        history: &[RuntimeMessage],
        pending_message: Option<&RuntimeMessage>,
    ) -> bool {
        if !self.settings.compact.enabled || !self.settings.compact.reactive_compact {
            return false;
        }

        let combined = self.history_with_pending_message(history, pending_message);
        matches!(
            self.compact_decision(&combined, None, true),
            CompactDecision::MicroCompact | CompactDecision::FullCompact
        )
    }

    pub fn reactive_compact_decision(
        &self,
        history: &[RuntimeMessage],
        pending_message: Option<&RuntimeMessage>,
    ) -> CompactDecision {
        let combined = self.history_with_pending_message(history, pending_message);
        self.compact_decision(&combined, None, true)
    }

    pub fn compact_decision(
        &self,
        history: &[RuntimeMessage],
        usage_total_tokens: Option<usize>,
        reactive: bool,
    ) -> CompactDecision {
        if !self.settings.compact.enabled
            || (!reactive && !self.settings.compact.auto_compact)
            || (reactive && !self.settings.compact.reactive_compact)
        {
            return CompactDecision::None;
        }

        let estimate = self.estimate_token_budget(history, usage_total_tokens);

        if estimate.remaining_budget == 0 {
            return if self.settings.compact.enable_microcompact
                && history.len() > preserved_message_count(0)
            {
                CompactDecision::MicroCompact
            } else if history.len()
                > preserved_message_count(self.settings.compact.preserve_recent_turns)
            {
                CompactDecision::FullCompact
            } else {
                CompactDecision::RejectForOverflow
            };
        }

        if history.len() >= self.settings.compact.max_turns_before_compact {
            return if self.settings.compact.enable_microcompact && !reactive {
                CompactDecision::MicroCompact
            } else {
                CompactDecision::FullCompact
            };
        }

        if estimate.prompt_estimate + estimate.reserved_completion_budget
            >= self.settings.compact.max_tokens_before_compact
        {
            return if self.settings.compact.enable_microcompact {
                CompactDecision::MicroCompact
            } else {
                CompactDecision::FullCompact
            };
        }

        CompactDecision::None
    }

    pub fn estimate_token_budget(
        &self,
        history: &[RuntimeMessage],
        usage_total_tokens: Option<usize>,
    ) -> TokenBudgetEstimate {
        let prompt_estimate =
            usage_total_tokens.unwrap_or_else(|| estimate_history_tokens(history));
        let preserved = preserved_message_count(self.settings.compact.preserve_recent_turns);
        let recent_start = history.len().saturating_sub(preserved);
        let recent_turns_estimate = estimate_history_tokens(&history[recent_start..]);
        let tool_context_estimate = history
            .iter()
            .filter(|message| message.has_tool_calls() || message.tool_result.is_some())
            .map(estimate_message_tokens)
            .sum();
        let budget_limit = self.settings.compact.max_tokens_before_compact;
        let reserved_completion_budget = self.settings.compact.reserved_completion_budget;
        let consumed = prompt_estimate.saturating_add(reserved_completion_budget);
        let remaining_budget = budget_limit.saturating_sub(consumed);

        TokenBudgetEstimate {
            prompt_estimate,
            recent_turns_estimate,
            tool_context_estimate,
            reserved_completion_budget,
            remaining_budget,
        }
    }

    pub async fn compact_history(
        &self,
        history: &[RuntimeMessage],
        instructions: Option<&str>,
    ) -> anyhow::Result<CompactOutcome> {
        self.compact_history_with_decision(history, instructions, CompactDecision::FullCompact)
            .await
    }

    pub async fn compact_history_with_decision(
        &self,
        history: &[RuntimeMessage],
        instructions: Option<&str>,
        decision: CompactDecision,
    ) -> anyhow::Result<CompactOutcome> {
        if !self.settings.compact.enabled {
            return Err(anyhow::anyhow!("Compact is disabled in settings"));
        }

        let preserved = match decision {
            CompactDecision::MicroCompact => preserved_message_count(0),
            _ => preserved_message_count(self.settings.compact.preserve_recent_turns),
        };
        if history.len() <= preserved {
            return Err(anyhow::anyhow!("Not enough messages to compact"));
        }

        let split_at = history.len() - preserved;
        let earlier = &history[..split_at];
        let recent = &history[split_at..];
        if earlier.is_empty() {
            return Err(anyhow::anyhow!("Not enough messages to compact"));
        }

        let (system, user) = build_compact_messages(earlier, instructions);
        let response = self
            .client
            .chat(&[ChatMessage::system(system), ChatMessage::user(user)])
            .await?;
        let summary = response
            .choices
            .first()
            .map(|choice| choice.message.content.trim().to_string())
            .filter(|content| !content.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Compact response did not include summary text"))?;

        let mut compacted_history = Vec::with_capacity(recent.len() + 1);
        compacted_history.push(match decision {
            CompactDecision::MicroCompact => RuntimeMessage::micro_compact_summary(summary.clone()),
            _ => RuntimeMessage::compact_summary(summary.clone()),
        });
        compacted_history.extend_from_slice(recent);

        Ok(CompactOutcome {
            history: compacted_history,
            summary,
            was_micro: matches!(decision, CompactDecision::MicroCompact),
        })
    }

    pub async fn maybe_auto_compact(
        &self,
        mut result: QueryTurnResult,
    ) -> anyhow::Result<QueryTurnResult> {
        if result.status != crate::runtime::TurnStatus::Completed {
            return Ok(result);
        }

        let decision = self.compact_decision(
            &result.history,
            result.usage.as_ref().map(|usage| usage.total_tokens),
            false,
        );
        if !matches!(
            decision,
            CompactDecision::MicroCompact | CompactDecision::FullCompact
        ) {
            return Ok(result);
        }

        match self
            .compact_history_with_decision(&result.history, None, decision)
            .await
        {
            Ok(outcome) => {
                result.history = outcome.history;
                result.was_compacted = true;
                result.compaction_summary = Some(if outcome.was_micro {
                    format!("[micro] {}", outcome.summary)
                } else {
                    outcome.summary
                });
                Ok(result)
            }
            Err(_) => Ok(result),
        }
    }

    fn history_with_pending_message(
        &self,
        history: &[RuntimeMessage],
        pending_message: Option<&RuntimeMessage>,
    ) -> Vec<RuntimeMessage> {
        let mut combined = history.to_vec();
        if let Some(message) = pending_message {
            combined.push(message.clone());
        }
        combined
    }
}

fn preserved_message_count(preserve_recent_turns: usize) -> usize {
    preserve_recent_turns.saturating_mul(2).max(1)
}

pub fn is_compact_summary_content(content: &str) -> bool {
    content.starts_with(COMPACT_SUMMARY_PREFIX) || content.starts_with(MICRO_COMPACT_SUMMARY_PREFIX)
}

fn estimate_history_tokens(history: &[RuntimeMessage]) -> usize {
    history.iter().map(estimate_message_tokens).sum()
}

fn estimate_message_tokens(message: &RuntimeMessage) -> usize {
    let content_tokens = message.content.chars().count() / 4;
    let tool_tokens = message
        .tool_calls
        .iter()
        .map(|call| call.name.len() + call.arguments.to_string().len())
        .sum::<usize>()
        / 4;
    let result_tokens = message
        .tool_result
        .as_ref()
        .map(|result| (result.name.len() + result.content.len()) / 4)
        .unwrap_or(0);
    content_tokens + tool_tokens + result_tokens + 12
}

#[cfg(test)]
mod tests {
    use super::CompactService;
    use crate::{config::Settings, runtime::RuntimeMessage};

    #[test]
    fn auto_compact_uses_turn_threshold_without_usage() {
        let mut settings = Settings::default();
        settings.compact.max_turns_before_compact = 3;
        let service = CompactService::new(settings);
        let history = vec![
            RuntimeMessage::user("one"),
            RuntimeMessage::assistant("two"),
            RuntimeMessage::user("three"),
        ];

        assert!(service.should_auto_compact(&history, None));
    }

    #[test]
    fn auto_compact_uses_token_threshold() {
        let settings = Settings::default();
        let service = CompactService::new(settings);
        let history = vec![RuntimeMessage::user("one")];

        assert!(service.should_auto_compact(&history, Some(32_000)));
        assert!(!service.should_auto_compact(&history, Some(10)));
    }

    #[test]
    fn reactive_compact_uses_pending_message_budget() {
        let mut settings = Settings::default();
        settings.compact.max_tokens_before_compact = 64;
        settings.compact.reserved_completion_budget = 32;
        let service = CompactService::new(settings);
        let history = vec![RuntimeMessage::user("small")];
        let pending =
            RuntimeMessage::user("this prompt is large enough to push the estimate over budget");

        assert!(service.should_reactive_compact(&history, Some(&pending)));
    }

    #[test]
    fn micro_compact_can_trigger_with_minimal_history() {
        let mut settings = Settings::default();
        settings.compact.max_tokens_before_compact = 32;
        settings.compact.reserved_completion_budget = 16;
        let service = CompactService::new(settings);
        let history = vec![
            RuntimeMessage::user("short"),
            RuntimeMessage::assistant("small reply"),
        ];

        assert_eq!(
            service.compact_decision(&history, Some(32), true),
            super::CompactDecision::MicroCompact
        );
    }
}
