use crate::runtime::{
    results::{QueryTurnResult, RuntimeUsage},
    types::RuntimeMessage,
};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct GatewayResponse {
    pub assistant_message: Option<RuntimeMessage>,
    pub usage: Option<RuntimeUsage>,
    pub model: String,
    pub finish_reason: Option<String>,
}

#[async_trait]
pub trait ModelGateway: Send + Sync {
    async fn complete(&self, messages: &[RuntimeMessage]) -> anyhow::Result<GatewayResponse>;
}

pub struct QueryLoop<G> {
    gateway: G,
}

impl<G> QueryLoop<G> {
    pub fn new(gateway: G) -> Self {
        Self { gateway }
    }
}

impl<G> QueryLoop<G>
where
    G: ModelGateway,
{
    pub async fn submit_turn(
        &self,
        history: &[RuntimeMessage],
        user_message: RuntimeMessage,
    ) -> anyhow::Result<QueryTurnResult> {
        let mut next_history = history.to_vec();
        next_history.push(user_message);

        let response = self.gateway.complete(&next_history).await?;
        if let Some(message) = response.assistant_message.clone() {
            next_history.push(message);
        }

        Ok(QueryTurnResult {
            history: next_history,
            assistant_message: response.assistant_message,
            usage: response.usage,
            model: response.model,
            finish_reason: response.finish_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayResponse, ModelGateway, QueryLoop};
    use crate::runtime::{
        results::RuntimeUsage,
        types::{RuntimeMessage, RuntimeRole},
    };
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct MockGateway {
        captured: Arc<Mutex<Vec<RuntimeMessage>>>,
    }

    #[async_trait]
    impl ModelGateway for MockGateway {
        async fn complete(&self, messages: &[RuntimeMessage]) -> anyhow::Result<GatewayResponse> {
            *self.captured.lock().unwrap() = messages.to_vec();
            Ok(GatewayResponse {
                assistant_message: Some(RuntimeMessage::assistant("answer")),
                usage: Some(RuntimeUsage {
                    prompt_tokens: 3,
                    completion_tokens: 2,
                    total_tokens: 5,
                }),
                model: "test-model".to_string(),
                finish_reason: Some("stop".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn submit_turn_appends_user_and_assistant_messages() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let loop_ = QueryLoop::new(MockGateway {
            captured: Arc::clone(&captured),
        });
        let history = vec![RuntimeMessage::system("system prompt")];

        let result = loop_
            .submit_turn(&history, RuntimeMessage::user("hello"))
            .await
            .unwrap();

        let sent = captured.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].role, RuntimeRole::System);
        assert_eq!(sent[1].role, RuntimeRole::User);
        assert_eq!(result.history.len(), 3);
        assert_eq!(result.assistant_text(), Some("answer"));
        assert_eq!(result.model, "test-model");
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert_eq!(result.usage.unwrap().total_tokens, 5);
    }

    struct EmptyGateway;

    #[async_trait]
    impl ModelGateway for EmptyGateway {
        async fn complete(&self, _messages: &[RuntimeMessage]) -> anyhow::Result<GatewayResponse> {
            Ok(GatewayResponse {
                assistant_message: None,
                usage: None,
                model: "test-model".to_string(),
                finish_reason: None,
            })
        }
    }

    #[tokio::test]
    async fn submit_turn_handles_empty_assistant_message() {
        let loop_ = QueryLoop::new(EmptyGateway);
        let history = vec![RuntimeMessage::new(RuntimeRole::User, "hi")];

        let result = loop_
            .submit_turn(&history, RuntimeMessage::user("next"))
            .await
            .unwrap();

        assert_eq!(result.history.len(), 2);
        assert!(result.assistant_message.is_none());
    }
}
