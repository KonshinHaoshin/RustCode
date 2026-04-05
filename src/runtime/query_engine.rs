use crate::{
    api::ApiClient,
    config::Settings,
    runtime::{
        query_loop::{GatewayResponse, ModelGateway, QueryLoop},
        results::{QueryTurnResult, RuntimeUsage},
        types::RuntimeMessage,
    },
};
use async_trait::async_trait;

pub struct QueryEngine<G = ApiModelGateway> {
    query_loop: QueryLoop<G>,
}

impl QueryEngine<ApiModelGateway> {
    pub fn new(settings: Settings) -> Self {
        Self::with_gateway(ApiModelGateway::new(settings))
    }
}

impl<G> QueryEngine<G> {
    pub fn with_gateway(gateway: G) -> Self {
        Self {
            query_loop: QueryLoop::new(gateway),
        }
    }
}

impl<G> QueryEngine<G>
where
    G: ModelGateway,
{
    pub async fn submit_message(
        &self,
        history: &[RuntimeMessage],
        message: RuntimeMessage,
    ) -> anyhow::Result<QueryTurnResult> {
        self.query_loop.submit_turn(history, message).await
    }

    pub async fn submit_text_turn(
        &self,
        history: &[RuntimeMessage],
        prompt: impl Into<String>,
    ) -> anyhow::Result<QueryTurnResult> {
        self.submit_message(history, RuntimeMessage::user(prompt))
            .await
    }
}

pub struct ApiModelGateway {
    client: ApiClient,
}

impl ApiModelGateway {
    pub fn new(settings: Settings) -> Self {
        Self {
            client: ApiClient::new(settings),
        }
    }
}

#[async_trait]
impl ModelGateway for ApiModelGateway {
    async fn complete(&self, messages: &[RuntimeMessage]) -> anyhow::Result<GatewayResponse> {
        let api_messages = messages
            .iter()
            .map(crate::api::ChatMessage::from)
            .collect::<Vec<_>>();
        let response = self.client.chat(&api_messages).await?;
        let choice = response.choices.first().cloned();

        Ok(GatewayResponse {
            assistant_message: choice
                .as_ref()
                .map(|choice| RuntimeMessage::from(&choice.message)),
            usage: response.usage.map(|usage| RuntimeUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            }),
            model: response.model,
            finish_reason: choice.and_then(|choice| choice.finish_reason),
        })
    }
}
