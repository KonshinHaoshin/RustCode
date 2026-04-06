use crate::{
    api::ApiClient,
    compact::CompactService,
    config::{
        add_project_local_permission_rule, project_root_from, ProjectPermissionRuleKind, Settings,
    },
    permissions::{PermissionGate, SettingsPermissionGate},
    runtime::{
        query_loop::{GatewayResponse, ModelGateway, QueryLoop},
        results::{ApprovalAction, NoopProgressSink, ProgressSink, QueryTurnResult, RuntimeUsage},
        types::RuntimeMessage,
    },
    tools_runtime::{
        BuiltinToolExecutor, CompositeToolExecutor, ExternalMcpToolExecutor, McpToolExecutor,
        ToolDefinition, ToolExecutionContext, ToolExecutor,
    },
    PermissionsSettings,
};
use async_trait::async_trait;
use std::path::PathBuf;

pub struct QueryEngine<G = ApiModelGateway, T = CompositeToolExecutor, P = SettingsPermissionGate> {
    query_loop: QueryLoop<G, T, P>,
    project_root: Option<PathBuf>,
    compact_service: Option<CompactService>,
}

impl QueryEngine<ApiModelGateway, CompositeToolExecutor, SettingsPermissionGate> {
    pub fn new(settings: Settings) -> Self {
        let permissions = settings.permissions.clone();
        let mcp_servers = settings.mcp_servers.clone();
        let compact_service = CompactService::new(settings.clone());
        let project_root = project_root_from(None);
        Self::with_parts(
            ApiModelGateway::new(settings.clone()),
            CompositeToolExecutor::new(vec![
                Box::new(BuiltinToolExecutor::new(
                    settings.clone(),
                    project_root.clone(),
                )),
                Box::new(McpToolExecutor::new()),
                Box::new(ExternalMcpToolExecutor::new(mcp_servers)),
            ]),
            SettingsPermissionGate::new(permissions),
            project_root,
        )
        .with_compact_service(compact_service)
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
            compact_service: None,
        }
    }

    pub fn with_compact_service(mut self, compact_service: CompactService) -> Self {
        self.compact_service = Some(compact_service);
        self
    }
}

impl<G> QueryEngine<G, CompositeToolExecutor, SettingsPermissionGate> {
    pub fn with_gateway_and_permissions(gateway: G, permissions: PermissionsSettings) -> Self {
        Self::with_parts(
            gateway,
            CompositeToolExecutor::new(vec![
                Box::new(BuiltinToolExecutor::new(
                    Settings::default(),
                    project_root_from(None),
                )),
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
        let mut progress = NoopProgressSink;
        self.submit_message_with_progress(history, message, &mut progress)
            .await
    }

    pub async fn submit_message_with_progress(
        &self,
        history: &[RuntimeMessage],
        message: RuntimeMessage,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        self.submit_message_with_context_and_progress(history, message, None, progress)
            .await
    }

    pub async fn submit_message_with_context_and_progress(
        &self,
        history: &[RuntimeMessage],
        message: RuntimeMessage,
        session_id: Option<String>,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        let result = self
            .query_loop
            .submit_turn_with_progress(
                history,
                message,
                ToolExecutionContext { session_id },
                progress,
            )
            .await?;
        self.apply_auto_compact(result).await
    }

    pub async fn submit_text_turn(
        &self,
        history: &[RuntimeMessage],
        prompt: impl Into<String>,
    ) -> anyhow::Result<QueryTurnResult> {
        self.submit_message(history, RuntimeMessage::user(prompt))
            .await
    }

    pub async fn submit_text_turn_with_context(
        &self,
        history: &[RuntimeMessage],
        prompt: impl Into<String>,
        session_id: Option<String>,
    ) -> anyhow::Result<QueryTurnResult> {
        let mut progress = NoopProgressSink;
        self.submit_message_with_context_and_progress(
            history,
            RuntimeMessage::user(prompt),
            session_id,
            &mut progress,
        )
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
        let mut progress = NoopProgressSink;
        self.resume_after_approval_with_progress(history, action, &mut progress)
            .await
    }

    pub async fn resume_after_approval_with_progress(
        &self,
        history: &[RuntimeMessage],
        action: ApprovalAction,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        self.resume_after_approval_with_context_and_progress(history, action, None, progress)
            .await
    }

    pub async fn resume_after_approval_with_context_and_progress(
        &self,
        history: &[RuntimeMessage],
        action: ApprovalAction,
        session_id: Option<String>,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        let tool_result = match &action {
            ApprovalAction::AllowOnce(pending) | ApprovalAction::AlwaysAllow(pending) => {
                self.query_loop
                    .tool_executor()
                    .execute(
                        &pending.tool_call,
                        &ToolExecutionContext {
                            session_id: session_id.clone(),
                        },
                    )
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
                add_project_local_permission_rule(
                    self.project_root.as_deref(),
                    ProjectPermissionRuleKind::Allow,
                    &pending.tool_call.name,
                )?;
            }
            ApprovalAction::AlwaysDeny(pending) => {
                add_project_local_permission_rule(
                    self.project_root.as_deref(),
                    ProjectPermissionRuleKind::Deny,
                    &pending.tool_call.name,
                )?;
            }
            ApprovalAction::AllowOnce(_) | ApprovalAction::DenyOnce(_) => {}
        }

        progress.emit(crate::runtime::QueryProgressEvent::ToolResult(
            tool_result.clone(),
        ));
        let result = self
            .query_loop
            .resume_turn_with_progress(
                history,
                tool_result,
                ToolExecutionContext { session_id },
                progress,
            )
            .await?;
        self.apply_auto_compact(result).await
    }
}

impl<G, T, P> QueryEngine<G, T, P> {
    async fn apply_auto_compact(&self, result: QueryTurnResult) -> anyhow::Result<QueryTurnResult> {
        match &self.compact_service {
            Some(compact_service) => compact_service.maybe_auto_compact(result).await,
            None => Ok(result),
        }
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
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<GatewayResponse> {
        let api_messages = messages
            .iter()
            .map(crate::api::ChatMessage::from)
            .collect::<Vec<_>>();
        let response = self
            .client
            .chat_with_tools_and_progress(&api_messages, tools, progress)
            .await?;
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
