use crate::utils::project::{detect_project_type, ProjectType};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitMode {
    Create,
    Force,
    Append,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOutcome {
    pub path: PathBuf,
    pub message: String,
}

pub fn run_init(cwd: &Path, mode: InitMode) -> anyhow::Result<InitOutcome> {
    let path = cwd.join("rustcode.md");
    let content = build_rustcode_md(cwd);
    if path.exists() {
        match mode {
            InitMode::Create => {
                return Ok(InitOutcome {
                    path,
                    message: "rustcode.md already exists. Use /init --force to overwrite or /init --append to append.".to_string(),
                });
            }
            InitMode::Force => fs::write(&path, content)?,
            InitMode::Append => {
                let mut existing = fs::read_to_string(&path).unwrap_or_default();
                if !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing.push('\n');
                existing.push_str("<!-- rustcode init refresh -->\n\n");
                existing.push_str(&content);
                fs::write(&path, existing)?;
            }
        }
    } else {
        fs::write(&path, content)?;
    }

    let action = match mode {
        InitMode::Create => "Created",
        InitMode::Force => "Updated",
        InitMode::Append => "Appended",
    };
    Ok(InitOutcome {
        path: path.clone(),
        message: format!("{} {}.", action, path.display()),
    })
}

fn build_rustcode_md(cwd: &Path) -> String {
    let project_type = detect_project_type(cwd);
    let commands = common_commands(cwd, project_type);
    let structure = top_level_structure(cwd);
    format!(
        "# rustcode.md\n\nThis file provides guidance to RustCode when working in this repository.\n\n## Project Overview\n\n- Project type: {}\n- Workspace root: {}\n\n## Build and Test Commands\n\n{}\n\n## Repository Structure\n\n{}\n\n## Coding Conventions\n\n- Follow the existing style in nearby files.\n- Prefer minimal, reviewable changes.\n- Verify changed behavior with the most relevant test or build command.\n\n## Tool and Permission Notes\n\n- Do not commit secrets or real credentials.\n- Treat file writes, shell commands, plugin installation, and networked actions as high-impact operations.\n\n## Known Constraints\n\n- Keep this file concise. Add only repository-specific facts that RustCode would not reliably infer from source files.\n",
        project_type,
        cwd.display(),
        commands,
        structure
    )
}

fn common_commands(cwd: &Path, project_type: ProjectType) -> String {
    let mut commands = Vec::new();
    match project_type {
        ProjectType::Rust => {
            commands.push("- `cargo check` - verify the Rust workspace compiles");
            commands.push("- `cargo test` - run Rust tests");
            commands.push("- `cargo fmt --check` - check Rust formatting");
        }
        ProjectType::JavaScript => {
            if cwd.join("pnpm-lock.yaml").exists() {
                commands.push("- `pnpm install` - install dependencies");
                commands.push("- `pnpm run build` - build the project when available");
                commands.push("- `pnpm test` - run tests when available");
            } else {
                commands.push("- `npm install` - install dependencies");
                commands.push("- `npm run build` - build the project when available");
                commands.push("- `npm test` - run tests when available");
            }
        }
        ProjectType::Python => {
            commands.push("- `pytest` - run Python tests when configured");
        }
        ProjectType::Go => {
            commands.push("- `go test ./...` - run Go tests");
        }
        ProjectType::Cpp => {
            commands.push("- Use the repository CMake or Make workflow if present");
        }
        ProjectType::Unknown => {
            commands.push("- Add project-specific build, lint, and test commands here");
        }
    }
    commands.join("\n")
}

fn top_level_structure(cwd: &Path) -> String {
    let Ok(entries) = fs::read_dir(cwd) else {
        return "- Add key directories and files here".to_string();
    };
    let mut names = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (!name.starts_with(".git") && name != "target" && name != "node_modules")
                .then_some(name)
        })
        .take(12)
        .collect::<Vec<_>>();
    names.sort();
    if names.is_empty() {
        "- Add key directories and files here".to_string()
    } else {
        names
            .into_iter()
            .map(|name| format!("- `{}`", name))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{run_init, InitMode};

    #[test]
    fn init_creates_rustcode_md_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let first = run_init(temp.path(), InitMode::Create).unwrap();
        assert!(first.path.exists());
        std::fs::write(&first.path, "custom").unwrap();
        let second = run_init(temp.path(), InitMode::Create).unwrap();
        assert!(second.message.contains("already exists"));
        assert_eq!(std::fs::read_to_string(&first.path).unwrap(), "custom");
    }

    #[test]
    fn init_force_and_append_update_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rustcode.md");
        std::fs::write(&path, "custom").unwrap();
        run_init(temp.path(), InitMode::Append).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("rustcode init refresh"));
        run_init(temp.path(), InitMode::Force).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("# rustcode.md"));
    }
}
