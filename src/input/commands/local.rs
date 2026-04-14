use crate::{
    config::{ApiProvider, McpConfig, Settings},
    input::{McpSlashAction, PluginSlashAction, PluginUpdateTarget, SkillsSlashAction},
    mcp::McpManager,
    services::PluginMarketplaceService,
    state::AppState,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSkill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

pub fn skills_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agents")
        .join("skills")
}

pub fn run_diff_command(cwd: Option<&Path>, full: bool) -> anyhow::Result<String> {
    let cwd = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    if !git_ok(&cwd, ["rev-parse", "--is-inside-work-tree"])? {
        return Ok("Current working directory is not a git repository.".to_string());
    }

    let status = git_output(&cwd, ["status", "--short"])?;
    if status.trim().is_empty() {
        return Ok("Working tree is clean.".to_string());
    }

    if full {
        let diff = git_output(&cwd, ["diff", "--no-ext-diff", "--minimal"])?;
        if diff.trim().is_empty() {
            Ok(format!("Changed files:\n{}", status.trim()))
        } else {
            Ok(format!(
                "Changed files:\n{}\n\nPatch:\n{}",
                status.trim(),
                diff.trim()
            ))
        }
    } else {
        let stat = git_output(&cwd, ["diff", "--stat"])?;
        if stat.trim().is_empty() {
            Ok(format!("Changed files:\n{}", status.trim()))
        } else {
            Ok(format!(
                "Changed files:\n{}\n\nDiff stat:\n{}",
                status.trim(),
                stat.trim()
            ))
        }
    }
}

pub fn run_doctor_command(settings: &Settings) -> anyhow::Result<String> {
    let mut ok = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    ok.push(format!(
        "Current target: {}/{} ({})",
        settings.api.provider_label(),
        settings.model,
        settings.api.protocol().as_str()
    ));
    ok.push(format!(
        "Working directory: {}",
        settings.working_dir.display()
    ));
    if let Ok(exe) = std::env::current_exe() {
        ok.push(format!("Executable: {}", exe.display()));
    }

    if settings.should_run_onboarding() {
        warnings.push("Onboarding is not completed.".to_string());
    } else {
        ok.push("Onboarding completed.".to_string());
    }

    if settings.api.get_api_key().is_none() && settings.api.provider() != ApiProvider::Ollama {
        warnings.push(format!(
            "API key is not configured for provider {}.",
            settings.api.provider_label()
        ));
    } else {
        ok.push("API key configuration looks present.".to_string());
    }

    if which::which("git").is_ok() {
        ok.push("git is available on PATH.".to_string());
    } else {
        warnings.push("git is not available on PATH.".to_string());
    }

    let mcp_manager = McpManager::new();
    let mcp_servers = runtime_block_on(async { mcp_manager.list_servers().await })??;
    if mcp_servers.is_empty() {
        warnings.push("No MCP servers are configured.".to_string());
    } else {
        ok.push(format!("Configured MCP servers: {}", mcp_servers.len()));
    }

    if settings.plugins.plugin_dir.exists() {
        ok.push(format!(
            "Plugin directory exists: {}",
            settings.plugins.plugin_dir.display()
        ));
    } else {
        warnings.push(format!(
            "Plugin directory does not exist yet: {}",
            settings.plugins.plugin_dir.display()
        ));
    }

    let plugin_service =
        PluginMarketplaceService::new(Arc::new(RwLock::new(AppState::default())), None);
    let plugin_status =
        runtime_block_on(async { Ok::<_, anyhow::Error>(plugin_service.get_status().await) })??;
    ok.push(format!(
        "Installed plugins: {}",
        plugin_status.installed_count
    ));

    let root = skills_root();
    match discover_skills() {
        Ok(skills) if skills.is_empty() => {
            warnings.push(format!("No local skills found under {}.", root.display()));
        }
        Ok(skills) => ok.push(format!(
            "Discovered local skills: {} under {}",
            skills.len(),
            root.display()
        )),
        Err(error) if !root.exists() => {
            warnings.push(format!(
                "Skills directory does not exist: {}",
                root.display()
            ));
            warnings.push(format!("Skill discovery skipped: {}", error));
        }
        Err(error) => errors.push(format!("Skill discovery failed: {}", error)),
    }

    ok.push(format!(
        "Session persistence: auto_restore={} persist_transcript={}",
        settings.session.auto_restore_last_session, settings.session.persist_transcript
    ));

    let mut sections = Vec::new();
    sections.push("Doctor report".to_string());
    if !ok.is_empty() {
        sections.push(format!("OK:\n- {}", ok.join("\n- ")));
    }
    if !warnings.is_empty() {
        sections.push(format!("Warnings:\n- {}", warnings.join("\n- ")));
    }
    if !errors.is_empty() {
        sections.push(format!("Errors:\n- {}", errors.join("\n- ")));
    }

    Ok(sections.join("\n\n"))
}

pub async fn run_mcp_command(action: &McpSlashAction) -> anyhow::Result<String> {
    let manager = McpManager::new();
    match action {
        McpSlashAction::Help => Ok(mcp_help_text()),
        McpSlashAction::List => {
            let servers = manager.list_servers().await?;
            if servers.is_empty() {
                Ok("No MCP servers configured.".to_string())
            } else {
                Ok(servers
                    .into_iter()
                    .map(|server| format!("- {} [{}]", server.name, server.status))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        McpSlashAction::Add {
            name,
            command,
            args,
        } => {
            let mut config = McpConfig::new(name, command);
            config.args = args.clone();
            manager.add_server(config).await?;
            Ok(format!(
                "Added MCP server {} -> {}{}",
                name,
                command,
                format_args_suffix(args)
            ))
        }
        McpSlashAction::Remove { name } => {
            manager.remove_server(name).await?;
            Ok(format!("Removed MCP server {}.", name))
        }
        McpSlashAction::Restart { name } => {
            manager.restart_server(name).await?;
            Ok(format!("Restarted MCP server {}.", name))
        }
        McpSlashAction::Start { name } => {
            manager.start_server(name).await?;
            Ok(format!("Started MCP server {}.", name))
        }
        McpSlashAction::Stop { name } => {
            manager.stop_server(name).await?;
            Ok(format!("Stopped MCP server {}.", name))
        }
    }
}

pub async fn run_plugin_command(action: &PluginSlashAction) -> anyhow::Result<String> {
    let service = PluginMarketplaceService::new(Arc::new(RwLock::new(AppState::default())), None);
    match action {
        PluginSlashAction::Help => Ok(plugin_help_text()),
        PluginSlashAction::List => {
            let plugins = service.list_installed().await;
            if plugins.is_empty() {
                Ok("No plugins installed.".to_string())
            } else {
                Ok(plugins
                    .into_iter()
                    .map(|plugin| {
                        let status = if plugin.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        };
                        format!("- {} v{} [{}]", plugin.name, plugin.version, status)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        PluginSlashAction::Search { query } => {
            let plugins = service.search(query).await;
            if plugins.is_empty() {
                Ok(format!("No marketplace plugins matched '{}'.", query))
            } else {
                Ok(plugins
                    .into_iter()
                    .map(|plugin| {
                        format!(
                            "- {} v{} by {} (⭐ {})\n  {}",
                            plugin.name,
                            plugin.version,
                            plugin.author,
                            plugin.rating,
                            plugin.description
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        PluginSlashAction::Install { plugin } => {
            let installed = service.install(plugin).await?;
            Ok(format!(
                "Installed plugin {} v{}.",
                installed.name, installed.version
            ))
        }
        PluginSlashAction::Remove { name } => {
            service.remove(name).await?;
            Ok(format!("Removed plugin {}.", name))
        }
        PluginSlashAction::Enable { name } => {
            service.enable(name).await?;
            Ok(format!("Enabled plugin {}.", name))
        }
        PluginSlashAction::Disable { name } => {
            service.disable(name).await?;
            Ok(format!("Disabled plugin {}.", name))
        }
        PluginSlashAction::Update { target } => match target {
            PluginUpdateTarget::All => {
                let updated = service.update_all().await?;
                Ok(format!("Updated {} plugin(s).", updated.len()))
            }
            PluginUpdateTarget::One(name) => {
                let updated = service.update(name).await?;
                Ok(format!(
                    "Updated plugin {} to {}.",
                    updated.name, updated.version
                ))
            }
        },
    }
}

pub fn run_skills_command(action: &SkillsSlashAction) -> anyhow::Result<String> {
    match action {
        SkillsSlashAction::Help => Ok(skills_help_text()),
        SkillsSlashAction::List => {
            let skills = discover_skills()?;
            if skills.is_empty() {
                Ok(format!(
                    "No local skills found under {}.",
                    skills_root().display()
                ))
            } else {
                Ok(skills
                    .into_iter()
                    .map(|skill| {
                        format!(
                            "- {}: {}\n  {}",
                            skill.name,
                            skill.description,
                            skill.path.display()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        SkillsSlashAction::Show { name } => {
            let skills = discover_skills()?;
            let skill = skills
                .into_iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", name))?;
            let raw = std::fs::read_to_string(skill.path.join("SKILL.md"))?;
            let body = split_frontmatter(&raw).1;
            let preview = body
                .lines()
                .take(24)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
            Ok(format!(
                "Skill: {}\nDescription: {}\nPath: {}\n\n{}",
                skill.name,
                skill.description,
                skill.path.display(),
                preview
            ))
        }
        SkillsSlashAction::Path => Ok(skills_root().display().to_string()),
    }
}

pub fn discover_skills() -> anyhow::Result<Vec<DiscoveredSkill>> {
    let root = skills_root();
    let entries = std::fs::read_dir(&root)?;
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&skill_file)?;
        let (frontmatter, body) = split_frontmatter(&raw);
        let name = frontmatter_value(&frontmatter, "name").unwrap_or_else(|| {
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown")
                .to_string()
        });
        let description = frontmatter_value(&frontmatter, "description")
            .or_else(|| first_non_empty_line(&body))
            .unwrap_or_else(|| "No description available.".to_string());
        skills.push(DiscoveredSkill {
            name,
            description,
            path,
        });
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(skills)
}

pub fn mcp_help_text() -> String {
    "/mcp commands:\n- /mcp list\n- /mcp add <name> <command ...>\n- /mcp remove <name>\n- /mcp restart <name>\n- /mcp start <name>\n- /mcp stop <name>".to_string()
}

pub fn plugin_help_text() -> String {
    "/plugin commands:\n- /plugin list\n- /plugin search <query>\n- /plugin install <name>\n- /plugin remove <name>\n- /plugin enable <name>\n- /plugin disable <name>\n- /plugin update <name>\n- /plugin update --all".to_string()
}

pub fn skills_help_text() -> String {
    "/skills commands:\n- /skills list\n- /skills show <name>\n- /skills path".to_string()
}

fn git_ok<const N: usize>(cwd: &Path, args: [&str; N]) -> anyhow::Result<bool> {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).output()?;
    Ok(output.status.success())
}

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> anyhow::Result<String> {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "{}",
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).replace('\r', ""))
}

fn format_args_suffix(args: &[String]) -> String {
    if args.is_empty() {
        String::new()
    } else {
        format!(" {}", args.join(" "))
    }
}

fn split_frontmatter(raw: &str) -> (String, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (String::new(), raw.to_string());
    }
    let mut lines = trimmed.lines();
    let _ = lines.next();
    let mut frontmatter = Vec::new();
    let mut body = Vec::new();
    let mut in_frontmatter = true;
    for line in lines {
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
            continue;
        }
        if in_frontmatter {
            frontmatter.push(line);
        } else {
            body.push(line);
        }
    }
    (frontmatter.join("\n"), body.join("\n"))
}

fn frontmatter_value(frontmatter: &str, key: &str) -> Option<String> {
    frontmatter.lines().find_map(|line| {
        let (candidate_key, value) = line.split_once(':')?;
        (candidate_key.trim() == key).then(|| value.trim().trim_matches('"').to_string())
    })
}

fn first_non_empty_line(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
}

fn runtime_block_on<F, T>(future: F) -> anyhow::Result<T>
where
    F: std::future::Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(future))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_skill_frontmatter() {
        let raw = "---\nname: example\ndescription: sample\n---\n# Title\n\nBody";
        let (frontmatter, body) = split_frontmatter(raw);
        assert_eq!(
            frontmatter_value(&frontmatter, "name").as_deref(),
            Some("example")
        );
        assert!(body.contains("Body"));
    }

    #[test]
    fn falls_back_to_first_non_heading_line() {
        assert_eq!(
            first_non_empty_line("# Title\n\nSummary line\nMore"),
            Some("Summary line".to_string())
        );
    }
}
