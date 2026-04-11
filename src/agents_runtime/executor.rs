use crate::{
    agents_runtime::{
        context::{build_child_history, build_direct_history},
        store::AgentTaskStore,
        AgentPermissionMode,
    },
    config::Settings,
    permissions::SettingsPermissionGate,
    runtime::{ApiModelGateway, ApprovalAction, QueryEngine, RuntimeMessage, TurnStatus},
    services::agents::{AgentDefinition, AgentsService},
    session::{SessionManager, SessionStatus},
    tools_runtime::{
        BuiltinToolExecutor, CompositeToolExecutor, ExternalMcpToolExecutor, McpToolExecutor,
    },
};
use std::path::PathBuf;

pub fn spawn_agent_task(
    settings: Settings,
    project_root: Option<PathBuf>,
    task_id: String,
) -> anyhow::Result<()> {
    let store = AgentTaskStore::for_project(project_root.as_deref())?;
    let task = store
        .get(&task_id)?
        .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;
    let Some(agent) = AgentsService::builtin_definition_by_name(&task.agent_type) else {
        store.fail(&task.id, format!("Unknown agent type: {}", task.agent_type))?;
        return Ok(());
    };

    std::thread::spawn(move || {
        let task_project_root = project_root.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let result = match runtime {
            Ok(runtime) => runtime.block_on(run_task(
                settings,
                task_project_root,
                task_id.clone(),
                agent,
            )),
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            if let Ok(store) = AgentTaskStore::for_project(project_root.as_deref()) {
                let _ = store.fail(&task_id, error.to_string());
            }
        }
    });
    Ok(())
}

pub fn resume_agent_task_after_approval(
    settings: Settings,
    project_root: Option<PathBuf>,
    task_id: String,
    action: ApprovalAction,
) -> anyhow::Result<()> {
    let store = AgentTaskStore::for_project(project_root.as_deref())?;
    store.resume_after_approval(&task_id)?;

    std::thread::spawn(move || {
        let task_project_root = project_root.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let result = match runtime {
            Ok(runtime) => runtime.block_on(resume_task_after_approval(
                settings,
                task_project_root,
                task_id.clone(),
                action,
            )),
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            if let Ok(store) = AgentTaskStore::for_project(project_root.as_deref()) {
                let _ = store.fail(&task_id, error.to_string());
            }
        }
    });
    Ok(())
}

pub async fn run_agent_direct(
    settings: Settings,
    project_root: Option<PathBuf>,
    agent: AgentDefinition,
    prompt: String,
) -> anyhow::Result<String> {
    let session_manager = SessionManager::for_working_dir(project_root.as_deref());
    let parent_session = session_manager.load_latest_resumable()?;
    let parent_history = parent_session
        .as_ref()
        .map(|session| session.runtime_history())
        .unwrap_or_default();
    let history = build_direct_history(
        &parent_history,
        &prompt,
        &agent,
        parent_session.as_ref(),
        project_root.as_deref(),
    )?;

    let turn = execute_agent_turns(settings, project_root, &agent, &history, None, true).await?;

    if turn.status == TurnStatus::AwaitingApproval {
        return Err(anyhow::anyhow!(
            "Agent execution requires interactive approval. Re-run in TUI."
        ));
    }

    Ok(turn
        .assistant_text()
        .map(str::to_string)
        .unwrap_or_default())
}

async fn run_task(
    settings: Settings,
    project_root: Option<PathBuf>,
    task_id: String,
    agent: AgentDefinition,
) -> anyhow::Result<()> {
    let store = AgentTaskStore::for_project(project_root.as_deref())?;
    let task = store
        .get(&task_id)?
        .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;
    store.mark_running(&task.id)?;

    let session_manager = SessionManager::for_working_dir(project_root.as_deref());
    let mut child_session = session_manager.create_child_session(
        task.parent_session_id.as_deref(),
        &task.id,
        Some(&format!("agent-{}", agent.name)),
    )?;
    store.attach_child_session(&task.id, &child_session.id)?;

    let parent_session = task
        .parent_session_id
        .as_deref()
        .map(|session_id| session_manager.load(session_id))
        .transpose()?
        .flatten();
    let parent_history = parent_session
        .as_ref()
        .map(|session| session.runtime_history())
        .unwrap_or_default();
    let history = build_child_history(
        &parent_history,
        &task,
        &agent,
        parent_session.as_ref(),
        project_root.as_deref(),
    )?;

    let turn = execute_agent_turns(
        settings,
        project_root,
        &agent,
        &history,
        Some(child_session.id.clone()),
        false,
    )
    .await?;

    let status = persist_task_turn(
        &store,
        &session_manager,
        &mut child_session,
        &task.id,
        &turn,
    )?;

    session_manager.save_runtime_state(
        &mut child_session,
        &turn.history,
        status,
        turn.pending_approval.as_ref(),
    )?;
    Ok(())
}

async fn resume_task_after_approval(
    settings: Settings,
    project_root: Option<PathBuf>,
    task_id: String,
    action: ApprovalAction,
) -> anyhow::Result<()> {
    let store = AgentTaskStore::for_project(project_root.as_deref())?;
    let task = store
        .get(&task_id)?
        .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;
    let Some(agent) = AgentsService::builtin_definition_by_name(&task.agent_type) else {
        store.fail(&task.id, format!("Unknown agent type: {}", task.agent_type))?;
        return Ok(());
    };
    let child_session_id = task
        .child_session_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Task {} has no child session", task.id))?;
    let session_manager = SessionManager::for_working_dir(project_root.as_deref());
    let mut child_session = session_manager
        .load(&child_session_id)?
        .ok_or_else(|| anyhow::anyhow!("Child session not found: {}", child_session_id))?;
    let history = child_session.runtime_history();

    let turn = resume_agent_turns(
        settings,
        project_root,
        &agent,
        &history,
        action,
        Some(child_session.id.clone()),
        false,
    )
    .await?;

    let status = persist_task_turn(
        &store,
        &session_manager,
        &mut child_session,
        &task.id,
        &turn,
    )?;

    session_manager.save_runtime_state(
        &mut child_session,
        &turn.history,
        status,
        turn.pending_approval.as_ref(),
    )?;
    Ok(())
}

fn persist_task_turn(
    store: &AgentTaskStore,
    _session_manager: &SessionManager,
    _child_session: &mut crate::session::Session,
    task_id: &str,
    turn: &crate::runtime::QueryTurnResult,
) -> anyhow::Result<SessionStatus> {
    let status = if turn.status == TurnStatus::AwaitingApproval {
        let pending = turn
            .pending_approval
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Awaiting approval turn missing pending approval"))?;
        store.mark_awaiting_approval(task_id, pending)?;
        SessionStatus::AwaitingApproval
    } else {
        let summary = turn
            .assistant_text()
            .map(str::to_string)
            .unwrap_or_else(|| "Child agent completed without a text response.".to_string());
        store.complete(task_id, summary)?;
        SessionStatus::Completed
    };
    Ok(status)
}

async fn execute_agent_turns(
    settings: Settings,
    project_root: Option<PathBuf>,
    agent: &AgentDefinition,
    history: &[RuntimeMessage],
    session_id: Option<String>,
    interactive_error: bool,
) -> anyhow::Result<crate::runtime::QueryTurnResult> {
    let (user_message, history_prefix) = history
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("Agent history must end with a user message"))?;
    if !matches!(user_message.role, crate::runtime::RuntimeRole::User) {
        return Err(anyhow::anyhow!(
            "Agent history must end with a user message before execution"
        ));
    }

    let gateway = ApiModelGateway::new(settings.clone());
    let tool_executor = build_agent_tool_executor(&settings, project_root.clone(), agent);
    let permission_gate = SettingsPermissionGate::new(agent_permissions(
        settings,
        agent.permission_mode.unwrap_or_default(),
    ));
    let engine = QueryEngine::with_parts(gateway, tool_executor, permission_gate, project_root);
    let max_turns = agent.max_turns.unwrap_or(1).max(1);
    let mut current_history_prefix = history_prefix.to_vec();
    let mut current_user_message = user_message.clone();

    for turn_index in 0..max_turns {
        let turn = engine
            .submit_message_with_context_and_progress(
                &current_history_prefix,
                current_user_message.clone(),
                session_id.clone(),
                &mut crate::runtime::NoopProgressSink,
            )
            .await?;
        if interactive_error && turn.status == TurnStatus::AwaitingApproval {
            return Err(anyhow::anyhow!(
                "Agent execution requires interactive approval. Re-run in TUI."
            ));
        }
        if !should_continue_agent_turn(&turn, turn_index + 1, max_turns) {
            return Ok(turn);
        }
        current_history_prefix = turn.history.clone();
        current_user_message = RuntimeMessage::user(
            "Continue until the assigned task is complete. If you are blocked, report the blocker concisely.",
        );
    }

    Err(anyhow::anyhow!(
        "Agent '{}' exceeded max turns ({}) without producing a final response",
        agent.name,
        max_turns
    ))
}

async fn resume_agent_turns(
    settings: Settings,
    project_root: Option<PathBuf>,
    agent: &AgentDefinition,
    history: &[RuntimeMessage],
    action: ApprovalAction,
    session_id: Option<String>,
    interactive_error: bool,
) -> anyhow::Result<crate::runtime::QueryTurnResult> {
    let gateway = ApiModelGateway::new(settings.clone());
    let tool_executor = build_agent_tool_executor(&settings, project_root.clone(), agent);
    let permission_gate = SettingsPermissionGate::new(agent_permissions(
        settings,
        agent.permission_mode.unwrap_or_default(),
    ));
    let engine = QueryEngine::with_parts(gateway, tool_executor, permission_gate, project_root);
    let max_turns = agent.max_turns.unwrap_or(1).max(1);

    let mut turn = engine
        .resume_after_approval_with_context_and_progress(
            history,
            action,
            session_id.clone(),
            &mut crate::runtime::NoopProgressSink,
        )
        .await?;
    if interactive_error && turn.status == TurnStatus::AwaitingApproval {
        return Err(anyhow::anyhow!(
            "Agent execution requires interactive approval. Re-run in TUI."
        ));
    }
    let mut turns_used = 1usize;
    while should_continue_agent_turn(&turn, turns_used, max_turns) {
        turns_used += 1;
        turn = engine
            .submit_text_turn_with_context(
                &turn.history,
                "Continue until the assigned task is complete. If you are blocked, report the blocker concisely.",
                session_id.clone(),
            )
            .await?;
        if interactive_error && turn.status == TurnStatus::AwaitingApproval {
            return Err(anyhow::anyhow!(
                "Agent execution requires interactive approval. Re-run in TUI."
            ));
        }
    }
    Ok(turn)
}

fn build_agent_tool_executor(
    settings: &Settings,
    project_root: Option<PathBuf>,
    agent: &AgentDefinition,
) -> CompositeToolExecutor {
    let mut executors: Vec<Box<dyn crate::tools_runtime::ToolExecutor>> =
        vec![Box::new(BuiltinToolExecutor::with_profile(
            settings.clone(),
            project_root.clone(),
            Some(agent.tools.clone()),
            agent.allow_task_tools,
        ))];
    if agent.allow_mcp_tools {
        executors.push(Box::new(McpToolExecutor::new()));
    }
    if agent.allow_external_mcp_tools {
        executors.push(Box::new(ExternalMcpToolExecutor::new(
            settings.mcp_servers.clone(),
        )));
    }
    CompositeToolExecutor::new(executors)
}

fn should_continue_agent_turn(
    turn: &crate::runtime::QueryTurnResult,
    turns_used: usize,
    max_turns: usize,
) -> bool {
    if turns_used >= max_turns || turn.status == TurnStatus::AwaitingApproval {
        return false;
    }

    let assistant_text = turn.assistant_text().unwrap_or("").trim();
    turn.finish_reason.as_deref() == Some("length")
        || (assistant_text.is_empty() && turn.tool_call_count > 0)
}

fn agent_permissions(
    mut settings: Settings,
    mode: AgentPermissionMode,
) -> crate::permissions::PermissionsSettings {
    match mode {
        AgentPermissionMode::Inherit => settings.permissions,
        AgentPermissionMode::DenySensitive => {
            extend_unique(&mut settings.permissions.deny_tools, "execute_command");
            extend_unique(&mut settings.permissions.deny_tools, "file_write");
            extend_unique(&mut settings.permissions.deny_tools, "file_edit");
            settings.permissions
        }
        AgentPermissionMode::BackgroundSafe => {
            if matches!(
                settings.permissions.mode,
                crate::permissions::PermissionMode::Ask
            ) {
                settings.permissions.mode = crate::permissions::PermissionMode::DenyAll;
            }
            let ask_tools = std::mem::take(&mut settings.permissions.ask_tools);
            for tool in ask_tools {
                extend_unique(&mut settings.permissions.deny_tools, &tool);
            }
            settings.permissions
        }
    }
}

fn extend_unique(rules: &mut Vec<String>, tool_name: &str) {
    if !rules
        .iter()
        .any(|rule| rule.eq_ignore_ascii_case(tool_name))
    {
        rules.push(tool_name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{QueryTurnResult, RuntimeMessage};

    fn turn(
        content: &str,
        finish_reason: Option<&str>,
        tool_call_count: usize,
        status: TurnStatus,
    ) -> QueryTurnResult {
        QueryTurnResult {
            history: Vec::new(),
            assistant_message: Some(RuntimeMessage::assistant(content)),
            usage: None,
            model: "test".to_string(),
            finish_reason: finish_reason.map(str::to_string),
            tool_call_count,
            status,
            pending_approval: None,
            was_compacted: false,
            compaction_summary: None,
        }
    }

    #[test]
    fn should_continue_agent_turn_on_length_finish() {
        let turn = turn("partial", Some("length"), 0, TurnStatus::Completed);

        assert!(should_continue_agent_turn(&turn, 1, 2));
    }

    #[test]
    fn should_continue_agent_turn_after_empty_tool_turn() {
        let turn = turn("", None, 1, TurnStatus::Completed);

        assert!(should_continue_agent_turn(&turn, 1, 2));
    }

    #[test]
    fn should_not_continue_when_max_turns_used() {
        let turn = turn("", None, 1, TurnStatus::Completed);

        assert!(!should_continue_agent_turn(&turn, 2, 2));
    }

    #[test]
    fn should_not_continue_when_awaiting_approval() {
        let turn = turn("", Some("length"), 1, TurnStatus::AwaitingApproval);

        assert!(!should_continue_agent_turn(&turn, 1, 3));
    }
}
