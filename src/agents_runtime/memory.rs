use crate::{agents_runtime::types::AgentMemoryScope, config::global_config_dir};
use std::path::{Path, PathBuf};

pub fn get_agent_memory_dir(
    agent_type: &str,
    scope: AgentMemoryScope,
    project_root: Option<&Path>,
) -> PathBuf {
    let agent_dir = sanitize_agent_dir(agent_type);
    match scope {
        AgentMemoryScope::User => global_config_dir().join("agent-memory").join(agent_dir),
        AgentMemoryScope::Project => project_root
            .unwrap_or_else(|| Path::new("."))
            .join("rustcode")
            .join("agent-memory")
            .join(agent_dir),
        AgentMemoryScope::Local => project_root
            .unwrap_or_else(|| Path::new("."))
            .join("rustcode")
            .join("agent-memory-local")
            .join(agent_dir),
    }
}

pub fn get_agent_memory_entrypoint(
    agent_type: &str,
    scope: AgentMemoryScope,
    project_root: Option<&Path>,
) -> PathBuf {
    get_agent_memory_dir(agent_type, scope, project_root).join("MEMORY.md")
}

pub fn ensure_agent_memory_dir_exists(
    agent_type: &str,
    scope: AgentMemoryScope,
    project_root: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let dir = get_agent_memory_dir(agent_type, scope, project_root);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn load_agent_memory_prompt(
    agent_type: &str,
    scope: AgentMemoryScope,
    project_root: Option<&Path>,
) -> anyhow::Result<Option<String>> {
    let entrypoint = get_agent_memory_entrypoint(agent_type, scope, project_root);
    if let Some(parent) = entrypoint.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !entrypoint.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(entrypoint)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let scope_note = match scope {
        AgentMemoryScope::User => {
            "Persistent agent memory (user scope). Keep learnings generally applicable across projects."
        }
        AgentMemoryScope::Project => {
            "Persistent agent memory (project scope). Tailor decisions and terminology to this project."
        }
        AgentMemoryScope::Local => {
            "Persistent agent memory (local scope). This memory is machine-local and project-local."
        }
    };

    Ok(Some(format!(
        "{scope_note}\n\nRead and use the following memory when relevant:\n\n{trimmed}"
    )))
}

fn sanitize_agent_dir(agent_type: &str) -> String {
    agent_type.replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_entrypoint_uses_project_scope_dir() {
        let root = Path::new("F:/repo");
        let path = get_agent_memory_entrypoint("explore", AgentMemoryScope::Project, Some(root));
        assert!(path.ends_with("rustcode/agent-memory/explore/MEMORY.md"));
    }
}
