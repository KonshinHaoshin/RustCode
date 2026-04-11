use super::vendor::VENDORED_CLAUDE_COMPAT_PROMPT;
use crate::config::{global_config_dir, Settings};
use std::fs;
use std::path::{Path, PathBuf};

pub fn build_system_prompt(settings: &Settings, cwd: Option<&Path>) -> String {
    let cwd = cwd.unwrap_or(&settings.working_dir);
    let mut sections = vec![
        "You are RustCode, a pragmatic coding assistant operating inside the user's workspace. Make minimal correct changes, verify important behavior, and be explicit about uncertainty.".to_string(),
        format!(
            "Current provider/model: {}/{}.",
            settings.api.provider_label(),
            settings.model
        ),
        format!("Working directory: {}.", cwd.display()),
    ];

    if settings.prompt.vendor_claude_compat {
        sections.push(VENDORED_CLAUDE_COMPAT_PROMPT.trim().to_string());
    }

    if let Some(agents) = find_upwards_file(cwd, "AGENTS.md")
        .and_then(|path| fs::read_to_string(path).ok())
        .filter(|content| !content.trim().is_empty())
    {
        sections.push(format!("Project instructions:\n{}", agents.trim()));
    }

    for memory_name in &settings.prompt.project_memory_files {
        if let Some(content) = load_memory_file(cwd, memory_name) {
            sections.push(format!(
                "Project memory from {}:\n{}",
                memory_name,
                content.trim()
            ));
            break;
        }
    }

    sections.join("\n\n")
}

fn load_memory_file(cwd: &Path, file_name: &str) -> Option<String> {
    find_upwards_file(cwd, file_name)
        .or_else(|| Some(global_config_dir().join(file_name)).filter(|path| path.exists()))
        .and_then(|path| fs::read_to_string(path).ok())
        .filter(|content| !content.trim().is_empty())
}

fn find_upwards_file(cwd: &Path, file_name: &str) -> Option<PathBuf> {
    let mut current = Some(cwd.to_path_buf());
    while let Some(path) = current {
        let candidate = path.join(file_name);
        if candidate.exists() {
            return Some(candidate);
        }
        current = path.parent().map(Path::to_path_buf);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::build_system_prompt;
    use crate::config::Settings;

    #[test]
    fn includes_project_memory_when_present() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("rustcode.md"), "memory block").unwrap();
        let mut settings = Settings::default();
        settings.working_dir = temp.path().to_path_buf();
        let prompt = build_system_prompt(&settings, Some(temp.path()));
        assert!(prompt.contains("memory block"));
    }
}
