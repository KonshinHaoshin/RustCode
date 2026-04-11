use crate::{
    agents_runtime::{memory::load_agent_memory_prompt, AgentContextStrategy, AgentTask},
    runtime::{RuntimeMessage, RuntimeRole},
    services::agents::AgentDefinition,
    session::Session,
};
use std::path::Path;

pub fn build_direct_history(
    parent_history: &[RuntimeMessage],
    prompt: &str,
    agent: &AgentDefinition,
    parent_session: Option<&Session>,
    project_root: Option<&Path>,
) -> anyhow::Result<Vec<RuntimeMessage>> {
    let mut history = build_agent_preamble(parent_history, agent, parent_session, project_root)?;
    history.push(RuntimeMessage::system(build_direct_task_contract(agent)));
    history.push(RuntimeMessage::user(prompt.to_string()));
    Ok(history)
}

pub fn build_child_history(
    parent_history: &[RuntimeMessage],
    task: &AgentTask,
    agent: &AgentDefinition,
    parent_session: Option<&Session>,
    project_root: Option<&Path>,
) -> anyhow::Result<Vec<RuntimeMessage>> {
    let mut history = build_agent_preamble(parent_history, agent, parent_session, project_root)?;
    history.push(RuntimeMessage::system(build_child_task_contract(
        task, agent,
    )));
    history.push(RuntimeMessage::user(task.description.clone()));
    Ok(history)
}

fn build_agent_preamble(
    parent_history: &[RuntimeMessage],
    agent: &AgentDefinition,
    parent_session: Option<&Session>,
    project_root: Option<&Path>,
) -> anyhow::Result<Vec<RuntimeMessage>> {
    let mut history = vec![RuntimeMessage::system(agent.system_prompt.clone())];

    if let Some(preamble) = agent
        .prompt_preamble
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        history.push(RuntimeMessage::system(preamble.to_string()));
    }

    if let Some(scope) = agent.memory_scope {
        if let Some(memory_prompt) =
            load_agent_memory_prompt(&agent.agent_type.to_string(), scope, project_root)?
        {
            history.push(RuntimeMessage::system(memory_prompt));
        }
    }

    if let Some(summary) = build_parent_context_summary(
        parent_history,
        parent_session,
        agent.context_strategy.unwrap_or_default(),
    ) {
        history.push(RuntimeMessage::system(summary));
    }

    Ok(history)
}

pub fn build_child_task_contract(task: &AgentTask, agent: &AgentDefinition) -> String {
    format!(
        "You are running as child agent '{agent_name}' for task {task_id}.\n\
Parent session: {parent_session}.\n\
Max turns: {max_turns}.\n\
Stay strictly within the assigned scope.\n\
Do not create subagents.\n\
If blocked by permissions or missing context, report the blocker concisely in your final answer.\n\
Focus only on the task described by the user message.",
        agent_name = agent.name,
        task_id = task.id,
        parent_session = task.parent_session_id.as_deref().unwrap_or("unknown"),
        max_turns = agent.max_turns.unwrap_or(1).max(1)
    )
}

pub fn build_direct_task_contract(agent: &AgentDefinition) -> String {
    format!(
        "You are running as specialized agent '{agent_name}'.\n\
Stay tightly scoped to the user request.\n\
Do not create subagents.\n\
If blocked by permissions or missing context, report the blocker concisely in your final answer.",
        agent_name = agent.name
    )
}

fn build_parent_context_summary(
    parent_history: &[RuntimeMessage],
    parent_session: Option<&Session>,
    strategy: AgentContextStrategy,
) -> Option<String> {
    match strategy {
        AgentContextStrategy::TaskOnly => None,
        AgentContextStrategy::TaskPlusCompactSummary => latest_compact_summary(parent_history)
            .or_else(|| recent_assistant_summary(parent_history, 6)),
        AgentContextStrategy::TaskPlusRecentAssistantSummary => {
            recent_assistant_summary(parent_history, 6)
        }
    }
    .or_else(|| {
        parent_session.map(|session| {
            format!(
                "Parent session context: session {} ({:?}) spawned this child task.",
                session.id, session.session_kind
            )
        })
    })
}

fn latest_compact_summary(parent_history: &[RuntimeMessage]) -> Option<String> {
    parent_history
        .iter()
        .rev()
        .find(|message| message.is_compact_summary())
        .map(|message| format!("Parent compact summary:\n{}", message.content))
}

fn recent_assistant_summary(
    parent_history: &[RuntimeMessage],
    max_messages: usize,
) -> Option<String> {
    let mut parts = Vec::new();
    for message in parent_history
        .iter()
        .rev()
        .filter(|message| matches!(message.role, RuntimeRole::User | RuntimeRole::Assistant))
        .take(max_messages)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let content = message.content.trim();
        if content.is_empty() {
            continue;
        }
        let label = match message.role {
            RuntimeRole::User => "User",
            RuntimeRole::Assistant => "Assistant",
            _ => continue,
        };
        parts.push(format!("{label}: {}", truncate(content, 500)));
    }

    if parts.is_empty() {
        None
    } else {
        Some(format!("Parent recent context:\n{}", parts.join("\n")))
    }
}

fn truncate(content: &str, max_chars: usize) -> String {
    let truncated = content.chars().take(max_chars).collect::<String>();
    if content.chars().count() > max_chars {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents_runtime::AgentContextStrategy;
    use crate::services::agents::{AgentDefinition, AgentType};

    fn test_agent() -> AgentDefinition {
        AgentDefinition {
            agent_type: AgentType::Explore,
            name: "Explore Agent".to_string(),
            description: String::new(),
            when_to_use: String::new(),
            tools: vec!["file_read".to_string()],
            model: "sonnet".to_string(),
            system_prompt: "system".to_string(),
            source: "built-in".to_string(),
            base_dir: "built-in".to_string(),
            permission_mode: None,
            memory_scope: None,
            max_turns: Some(1),
            context_strategy: Some(AgentContextStrategy::TaskPlusRecentAssistantSummary),
            prompt_preamble: None,
            allow_task_tools: false,
            allow_mcp_tools: false,
            allow_external_mcp_tools: false,
        }
    }

    #[test]
    fn child_history_ends_with_user_task_message() {
        let task = AgentTask {
            id: "task-1".to_string(),
            description: "inspect code".to_string(),
            ..AgentTask::default()
        };
        let history = build_child_history(&[], &task, &test_agent(), None, None).unwrap();
        assert_eq!(history.last().map(|msg| msg.role), Some(RuntimeRole::User));
    }

    #[test]
    fn child_task_contract_includes_lineage() {
        let task = AgentTask {
            id: "task-1".to_string(),
            parent_session_id: Some("parent-1".to_string()),
            ..AgentTask::default()
        };
        let mut agent = test_agent();
        agent.max_turns = Some(3);

        let contract = build_child_task_contract(&task, &agent);

        assert!(contract.contains("task task-1"));
        assert!(contract.contains("Parent session: parent-1"));
        assert!(contract.contains("Max turns: 3"));
    }
}
