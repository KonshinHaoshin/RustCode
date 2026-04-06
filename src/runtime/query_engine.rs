use crate::{
    api::ApiClient,
    config::{project_root_from, save_project_local_permissions, Settings},
    permissions::{PermissionGate, SettingsPermissionGate},
    runtime::{
        query_loop::{GatewayResponse, ModelGateway, QueryLoop},
        results::{ApprovalAction, QueryTurnResult, RuntimeUsage},
        types::RuntimeMessage,
    },
    tools_runtime::{
        BuiltinToolExecutor, CompositeToolExecutor, ExternalMcpToolExecutor, McpToolExecutor,
        ToolDefinition, ToolExecutor,
    },
    PermissionsSettings,
};
use async_trait::async_trait;
use std::path::PathBuf;

pub struct QueryEngine<G = ApiModelGateway, T = CompositeToolExecutor, P = SettingsPermissionGate> {
    query_loop: QueryLoop<G, T, P>,
    project_root: Option<PathBuf>,
}

impl QueryEngine<ApiModelGateway, CompositeToolExecutor, SettingsPermissionGate> {
    pub fn new(settings: Settings) -> Self {
        let permissions = settings.permissions.clone();
        let mcp_servers = settings.mcp_servers.clone();
        Self::with_parts(
            ApiModelGateway::new(settings),
            CompositeToolExecutor::new(vec![
                Box::new(BuiltinToolExecutor::new()),
                Box::new(McpToolExecutor::new()),
                Box::new(ExternalMcpToolExecutor::new(mcp_servers)),
            ]),
            SettingsPermissionGate::new(permissions),
            project_root_from(None),
        )
    }
}

impl<G, T, P> QueryEngine<G, T, P> {
    pub fn with_parts(
        gateway: G,
        tool_executor: T,
        permission_gate: P,
        project_root: Option<PathBuf>,
    ) -> Self {
        Self {
            query_loop: QueryLoop::new(gateway, tool_executor, permission_gate),
            project_root,
        }
    }
}

impl<G> QueryEngine<G, CompositeToolExecutor, SettingsPermissionGate> {
    pub fn with_gateway_and_permissions(gateway: G, permissions: PermissionsSettings) -> Self {
        Self::with_parts(
            gateway,
            CompositeToolExecutor::new(vec![
                Box::new(BuiltinToolExecutor::new()),
                Box::new(McpToolExecutor::new()),
            ]),
            SettingsPermissionGate::new(permissions),
            None,
        )
    }
}

impl<G, T, P> QueryEngine<G, T, P>
where
    G: ModelGateway,
    T: ToolExecutor,
    P: PermissionGate,
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

impl<G, T> QueryEngine<G, T, SettingsPermissionGate>
where
    G: ModelGateway,
    T: ToolExecutor,
{
    pub async fn resume_after_approval(
        &self,
        history: &[RuntimeMessage],
        action: ApprovalAction,
    ) -> anyhow::Result<QueryTurnResult> {
        let tool_result = match &action {
            ApprovalAction::AllowOnce(pending) | ApprovalAction::AlwaysAllow(pending) => {
                self.query_loop
                    .tool_executor()
                    .execute(&pending.tool_call)
                    .await
            }
            ApprovalAction::DenyOnce(pending) | ApprovalAction::AlwaysDeny(pending) => {
                SettingsPermissionGate::denied_tool_result(
                    &pending.tool_call,
                    pending.reason.clone(),
                )
            }
        };

        match action {
            ApprovalAction::AlwaysAllow(pending) => {
                let mut gate = self.query_loop.permission_gate().clone();
                gate.add_allow_rule(&pending.tool_call.name);
                save_project_local_permissions(self.project_root.as_deref(), gate.settings())?;
            }
            ApprovalAction::AlwaysDeny(pending) => {
                let mut gate = self.query_loop.permission_gate().clone();
                gate.add_deny_rule(&pending.tool_call.name);
                save_project_local_permissions(self.project_root.as_deref(), gate.settings())?;
            }
            ApprovalAction::AllowOnce(_) | ApprovalAction::DenyOnce(_) => {}
        }

        self.query_loop.resume_turn(history, tool_result).await
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
    async fn complete(
        &self,
        messages: &[RuntimeMessage],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<GatewayResponse> {
        let api_messages = messages
            .iter()
            .map(crate::api::ChatMessage::from)
            .collect::<Vec<_>>();
        let response = self.client.chat_with_tools(&api_messages, tools).await?;
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
