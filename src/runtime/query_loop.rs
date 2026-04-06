use crate::permissions::{PermissionDecision, PermissionGate, SettingsPermissionGate};
use crate::runtime::{
    results::{
        NoopProgressSink, PendingApproval, ProgressSink, QueryProgressEvent, QueryTurnResult,
        RuntimeUsage, TurnStatus,
    },
    types::{RuntimeMessage, RuntimeToolResult},
};
use crate::tools_runtime::{ToolDefinition, ToolExecutionContext, ToolExecutor};
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
    async fn complete(
        &self,
        messages: &[RuntimeMessage],
        tools: &[ToolDefinition],
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<GatewayResponse>;
}

pub struct QueryLoop<G, T, P> {
    gateway: G,
    tool_executor: T,
    permission_gate: P,
}

impl<G, T, P> QueryLoop<G, T, P> {
    pub fn new(gateway: G, tool_executor: T, permission_gate: P) -> Self {
        Self {
            gateway,
            tool_executor,
            permission_gate,
        }
    }

    pub fn tool_executor(&self) -> &T {
        &self.tool_executor
    }

    pub fn permission_gate(&self) -> &P {
        &self.permission_gate
    }
}

impl<G, T, P> QueryLoop<G, T, P>
where
    G: ModelGateway,
    T: ToolExecutor,
    P: PermissionGate,
{
    pub async fn submit_turn(
        &self,
        history: &[RuntimeMessage],
        user_message: RuntimeMessage,
    ) -> anyhow::Result<QueryTurnResult> {
        let mut progress = NoopProgressSink;
        self.submit_turn_with_progress(
            history,
            user_message,
            ToolExecutionContext::default(),
            &mut progress,
        )
        .await
    }

    pub async fn submit_turn_with_progress(
        &self,
        history: &[RuntimeMessage],
        user_message: RuntimeMessage,
        context: ToolExecutionContext,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        let mut next_history = history.to_vec();
        next_history.push(user_message);
        self.continue_turn(next_history, 0, context, progress).await
    }

    pub async fn resume_turn(
        &self,
        history: &[RuntimeMessage],
        tool_result: RuntimeToolResult,
    ) -> anyhow::Result<QueryTurnResult> {
        let mut progress = NoopProgressSink;
        self.resume_turn_with_progress(
            history,
            tool_result,
            ToolExecutionContext::default(),
            &mut progress,
        )
        .await
    }

    pub async fn resume_turn_with_progress(
        &self,
        history: &[RuntimeMessage],
        tool_result: RuntimeToolResult,
        context: ToolExecutionContext,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        let mut next_history = history.to_vec();
        next_history.push(RuntimeMessage::tool_result(tool_result));
        let tool_call_count = next_history
            .iter()
            .map(|message| message.tool_calls.len())
            .sum();
        self.continue_turn(next_history, tool_call_count, context, progress)
            .await
    }

    async fn continue_turn(
        &self,
        mut next_history: Vec<RuntimeMessage>,
        mut tool_call_count: usize,
        context: ToolExecutionContext,
        progress: &mut dyn ProgressSink,
    ) -> anyhow::Result<QueryTurnResult> {
        let tools = self.tool_executor.definitions().await;

        for _ in 0..8 {
            let response = self
                .gateway
                .complete(&next_history, &tools, progress)
                .await?;

            if let Some(message) = response.assistant_message.clone() {
                let tool_calls = message.tool_calls.clone();
                next_history.push(message.clone());

                if tool_calls.is_empty() {
                    return Ok(QueryTurnResult {
                        history: next_history,
                        assistant_message: Some(message),
                        usage: response.usage,
                        model: response.model,
                        finish_reason: response.finish_reason,
                        tool_call_count,
                        status: TurnStatus::Completed,
                        pending_approval: None,
                        was_compacted: false,
                        compaction_summary: None,
                    });
                }

                tool_call_count += tool_calls.len();
                for tool_call in tool_calls {
                    progress.emit(QueryProgressEvent::ToolCall(tool_call.clone()));
                    let result = match self.permission_gate.evaluate_tool_call(&tool_call) {
                        PermissionDecision::Allow => {
                            self.tool_executor.execute(&tool_call, &context).await
                        }
                        PermissionDecision::Deny(message) => {
                            SettingsPermissionGate::denied_tool_result(&tool_call, message)
                        }
                        PermissionDecision::Ask(reason) => {
                            progress.emit(QueryProgressEvent::AwaitingApproval(PendingApproval {
                                tool_call: tool_call.clone(),
                                reason: reason.clone(),
                            }));
                            return Ok(QueryTurnResult {
                                history: next_history,
                                assistant_message: Some(message),
                                usage: response.usage,
                                model: response.model,
                                finish_reason: response.finish_reason,
                                tool_call_count,
                                status: TurnStatus::AwaitingApproval,
                                pending_approval: Some(PendingApproval { tool_call, reason }),
                                was_compacted: false,
                                compaction_summary: None,
                            });
                        }
                    };
                    progress.emit(QueryProgressEvent::ToolResult(result.clone()));
                    next_history.push(RuntimeMessage::tool_result(result));
                }

                continue;
            }

            return Ok(QueryTurnResult {
                history: next_history,
                assistant_message: None,
                usage: response.usage,
                model: response.model,
                finish_reason: response.finish_reason,
                tool_call_count,
                status: TurnStatus::Completed,
                pending_approval: None,
                was_compacted: false,
                compaction_summary: None,
            });
        }

        Err(anyhow::anyhow!(
            "Tool loop exceeded maximum iterations for one turn"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayResponse, ModelGateway, QueryLoop};
    use crate::permissions::{PermissionDecision, PermissionGate};
    use crate::runtime::{
        results::{ProgressSink, QueryProgressEvent, RuntimeUsage, TurnStatus},
        types::{RuntimeMessage, RuntimeRole, RuntimeToolCall, RuntimeToolResult},
    };
    use crate::tools_runtime::{ToolDefinition, ToolExecutionContext, ToolExecutor};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct MockGateway {
        captured: Arc<Mutex<Vec<RuntimeMessage>>>,
    }

    #[async_trait]
    impl ModelGateway for MockGateway {
        async fn complete(
            &self,
            messages: &[RuntimeMessage],
            _tools: &[ToolDefinition],
            _progress: &mut dyn ProgressSink,
        ) -> anyhow::Result<GatewayResponse> {
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

    struct NoopToolExecutor;

    #[async_trait]
    impl ToolExecutor for NoopToolExecutor {
        async fn definitions(&self) -> Vec<ToolDefinition> {
            Vec::new()
        }

        async fn execute(
            &self,
            call: &RuntimeToolCall,
            _context: &ToolExecutionContext,
        ) -> RuntimeToolResult {
            RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: "noop".to_string(),
                is_error: false,
            }
        }
    }

    #[tokio::test]
    async fn submit_turn_appends_user_and_assistant_messages() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let loop_ = QueryLoop::new(
            MockGateway {
                captured: Arc::clone(&captured),
            },
            NoopToolExecutor,
            AllowAllPermissionGate,
        );
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
        assert_eq!(result.tool_call_count, 0);
        assert_eq!(result.status, TurnStatus::Completed);
    }

    struct EmptyGateway;

    #[async_trait]
    impl ModelGateway for EmptyGateway {
        async fn complete(
            &self,
            _messages: &[RuntimeMessage],
            _tools: &[ToolDefinition],
            _progress: &mut dyn ProgressSink,
        ) -> anyhow::Result<GatewayResponse> {
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
        let loop_ = QueryLoop::new(EmptyGateway, NoopToolExecutor, AllowAllPermissionGate);
        let history = vec![RuntimeMessage::new(RuntimeRole::User, "hi")];

        let result = loop_
            .submit_turn(&history, RuntimeMessage::user("next"))
            .await
            .unwrap();

        assert_eq!(result.history.len(), 2);
        assert!(result.assistant_message.is_none());
        assert_eq!(result.status, TurnStatus::Completed);
    }

    struct StreamingGateway;

    #[async_trait]
    impl ModelGateway for StreamingGateway {
        async fn complete(
            &self,
            _messages: &[RuntimeMessage],
            _tools: &[ToolDefinition],
            progress: &mut dyn ProgressSink,
        ) -> anyhow::Result<GatewayResponse> {
            progress.emit(QueryProgressEvent::ModelRequest {
                target: "custom/test".to_string(),
            });
            progress.emit(QueryProgressEvent::AssistantText("hello".to_string()));
            Ok(GatewayResponse {
                assistant_message: Some(RuntimeMessage::assistant("hello")),
                usage: None,
                model: "test-model".to_string(),
                finish_reason: Some("stop".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn submit_turn_reports_progress_events() {
        let loop_ = QueryLoop::new(StreamingGateway, NoopToolExecutor, AllowAllPermissionGate);
        let mut events = Vec::new();

        let result = loop_
            .submit_turn_with_progress(
                &[],
                RuntimeMessage::user("hi"),
                ToolExecutionContext::default(),
                &mut |event| {
                    events.push(event);
                },
            )
            .await
            .unwrap();

        assert_eq!(result.assistant_text(), Some("hello"));
        assert_eq!(
            events,
            vec![
                QueryProgressEvent::ModelRequest {
                    target: "custom/test".to_string()
                },
                QueryProgressEvent::AssistantText("hello".to_string())
            ]
        );
    }

    struct ToolLoopGateway {
        step: Arc<Mutex<u8>>,
    }

    #[async_trait]
    impl ModelGateway for ToolLoopGateway {
        async fn complete(
            &self,
            messages: &[RuntimeMessage],
            _tools: &[ToolDefinition],
            _progress: &mut dyn ProgressSink,
        ) -> anyhow::Result<GatewayResponse> {
            let mut step = self.step.lock().unwrap();
            let response = if *step == 0 {
                *step = 1;
                GatewayResponse {
                    assistant_message: Some(RuntimeMessage::assistant_with_tool_calls(vec![
                        RuntimeToolCall {
                            id: "call_1".to_string(),
                            name: "list_files".to_string(),
                            arguments: serde_json::json!({"path": "."}),
                        },
                    ])),
                    usage: None,
                    model: "test-model".to_string(),
                    finish_reason: Some("tool_calls".to_string()),
                }
            } else {
                assert_eq!(messages.last().unwrap().role, RuntimeRole::Tool);
                GatewayResponse {
                    assistant_message: Some(RuntimeMessage::assistant("done")),
                    usage: None,
                    model: "test-model".to_string(),
                    finish_reason: Some("stop".to_string()),
                }
            };
            Ok(response)
        }
    }

    struct RecordingToolExecutor {
        executed: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ToolExecutor for RecordingToolExecutor {
        async fn definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "list_files".to_string(),
                description: "List files".to_string(),
                input_schema: serde_json::json!({"type":"object"}),
            }]
        }

        async fn execute(
            &self,
            call: &RuntimeToolCall,
            _context: &ToolExecutionContext,
        ) -> RuntimeToolResult {
            self.executed.lock().unwrap().push(call.name.clone());
            RuntimeToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: "file_a\nfile_b".to_string(),
                is_error: false,
            }
        }
    }

    #[tokio::test]
    async fn submit_turn_executes_tool_calls_before_final_answer() {
        let executed = Arc::new(Mutex::new(Vec::new()));
        let loop_ = QueryLoop::new(
            ToolLoopGateway {
                step: Arc::new(Mutex::new(0)),
            },
            RecordingToolExecutor {
                executed: Arc::clone(&executed),
            },
            AllowAllPermissionGate,
        );

        let result = loop_
            .submit_turn(&[], RuntimeMessage::user("inspect project"))
            .await
            .unwrap();

        assert_eq!(executed.lock().unwrap().as_slice(), ["list_files"]);
        assert_eq!(result.assistant_text(), Some("done"));
        assert_eq!(result.tool_call_count, 1);
        assert_eq!(result.status, TurnStatus::Completed);
        assert_eq!(
            result
                .history
                .iter()
                .filter(|message| message.role == RuntimeRole::Tool)
                .count(),
            1
        );
    }

    struct DenyAllPermissionGate;

    impl PermissionGate for DenyAllPermissionGate {
        fn evaluate_tool_call(&self, _call: &RuntimeToolCall) -> PermissionDecision {
            PermissionDecision::Deny("tool denied".to_string())
        }
    }

    struct AllowAllPermissionGate;

    impl PermissionGate for AllowAllPermissionGate {
        fn evaluate_tool_call(&self, _call: &RuntimeToolCall) -> PermissionDecision {
            PermissionDecision::Allow
        }
    }

    #[tokio::test]
    async fn submit_turn_injects_error_tool_result_when_permission_denies_tool() {
        let loop_ = QueryLoop::new(
            ToolLoopGateway {
                step: Arc::new(Mutex::new(0)),
            },
            RecordingToolExecutor {
                executed: Arc::new(Mutex::new(Vec::new())),
            },
            DenyAllPermissionGate,
        );

        let result = loop_
            .submit_turn(&[], RuntimeMessage::user("inspect project"))
            .await
            .unwrap();

        let tool_message = result
            .history
            .iter()
            .find(|message| message.role == RuntimeRole::Tool)
            .and_then(|message| message.tool_result.as_ref())
            .unwrap();

        assert!(tool_message.is_error);
        assert!(tool_message.content.contains("tool denied"));
    }

    struct AskPermissionGate;

    impl PermissionGate for AskPermissionGate {
        fn evaluate_tool_call(&self, call: &RuntimeToolCall) -> PermissionDecision {
            PermissionDecision::Ask(format!("{} requires approval", call.name))
        }
    }

    #[tokio::test]
    async fn submit_turn_pauses_when_permission_requires_approval() {
        let loop_ = QueryLoop::new(
            ToolLoopGateway {
                step: Arc::new(Mutex::new(0)),
            },
            RecordingToolExecutor {
                executed: Arc::new(Mutex::new(Vec::new())),
            },
            AskPermissionGate,
        );

        let result = loop_
            .submit_turn(&[], RuntimeMessage::user("inspect project"))
            .await
            .unwrap();

        assert_eq!(result.status, TurnStatus::AwaitingApproval);
        let pending = result.pending_approval.unwrap();
        assert_eq!(pending.tool_call.name, "list_files");
        assert!(pending.reason.contains("requires approval"));
    }
}
