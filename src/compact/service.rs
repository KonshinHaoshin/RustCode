use crate::{
    api::{ApiClient, ChatMessage},
    compact::prompt::build_compact_messages,
    config::Settings,
    runtime::{QueryTurnResult, RuntimeMessage},
};
use serde::{Deserialize, Serialize};

pub const COMPACT_SUMMARY_PREFIX: &str = "Previous conversation compacted into summary:\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactSettings {
    pub enabled: bool,
    pub auto_compact: bool,
    pub max_turns_before_compact: usize,
    pub max_tokens_before_compact: usize,
    pub preserve_recent_turns: usize,
    pub summary_model: Option<String>,
}

impl Default for CompactSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_compact: true,
            max_turns_before_compact: 24,
            max_tokens_before_compact: 32_000,
            preserve_recent_turns: 4,
            summary_model: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompactOutcome {
    pub history: Vec<RuntimeMessage>,
    pub summary: String,
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
        if !self.settings.compact.enabled || !self.settings.compact.auto_compact {
            return false;
        }

        if history.len() >= self.settings.compact.max_turns_before_compact {
            return true;
        }

        usage_total_tokens
            .map(|total| total >= self.settings.compact.max_tokens_before_compact)
            .unwrap_or(false)
    }

    pub async fn compact_history(
        &self,
        history: &[RuntimeMessage],
        instructions: Option<&str>,
    ) -> anyhow::Result<CompactOutcome> {
        if !self.settings.compact.enabled {
            return Err(anyhow::anyhow!("Compact is disabled in settings"));
        }

        let preserved = preserved_message_count(self.settings.compact.preserve_recent_turns);
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
        compacted_history.push(RuntimeMessage::compact_summary(summary.clone()));
        compacted_history.extend_from_slice(recent);

        Ok(CompactOutcome {
            history: compacted_history,
            summary,
        })
    }

    pub async fn maybe_auto_compact(
        &self,
        mut result: QueryTurnResult,
    ) -> anyhow::Result<QueryTurnResult> {
        if result.status != crate::runtime::TurnStatus::Completed {
            return Ok(result);
        }

        if !self.should_auto_compact(
            &result.history,
            result.usage.as_ref().map(|usage| usage.total_tokens),
        ) {
            return Ok(result);
        }

        match self.compact_history(&result.history, None).await {
            Ok(outcome) => {
                result.history = outcome.history;
                result.was_compacted = true;
                result.compaction_summary = Some(outcome.summary);
                Ok(result)
            }
            Err(_) => Ok(result),
        }
    }
}

fn preserved_message_count(preserve_recent_turns: usize) -> usize {
    preserve_recent_turns.saturating_mul(2).max(1)
}

pub fn is_compact_summary_content(content: &str) -> bool {
    content.starts_with(COMPACT_SUMMARY_PREFIX)
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
}
