use super::spec::{SlashCommandKind, SlashCommandSource, SlashCommandSpec};
use crate::config::global_config_dir;
use std::fs;
use std::path::{Path, PathBuf};

pub fn discover_markdown_commands(cwd: &Path) -> Vec<SlashCommandSpec> {
    let mut commands = Vec::new();
    for (dir, source) in command_dirs(cwd) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            if let Ok(Some(command)) = parse_markdown_command(&path, source) {
                commands.push(command);
            }
        }
    }
    commands
}

fn command_dirs(cwd: &Path) -> Vec<(PathBuf, SlashCommandSource)> {
    vec![
        (
            cwd.join(".rustcode").join("commands"),
            SlashCommandSource::Project,
        ),
        (
            global_config_dir().join("commands"),
            SlashCommandSource::User,
        ),
        (
            cwd.join(".claude").join("commands"),
            SlashCommandSource::ClaudeCompat,
        ),
        (
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
                .join("commands"),
            SlashCommandSource::ClaudeCompat,
        ),
    ]
}

fn parse_markdown_command(
    path: &Path,
    source: SlashCommandSource,
) -> anyhow::Result<Option<SlashCommandSpec>> {
    let raw = fs::read_to_string(path)?;
    let (frontmatter, body) = split_frontmatter(&raw);
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .trim();
    if name.is_empty() || body.trim().is_empty() {
        return Ok(None);
    }
    let description = frontmatter_value(&frontmatter, "description")
        .unwrap_or_else(|| format!("Run {} command", name));
    let argument_hint = frontmatter_value(&frontmatter, "argument-hint");
    Ok(Some(SlashCommandSpec {
        name: name.to_string(),
        aliases: Vec::new(),
        description,
        argument_hint,
        source,
        kind: SlashCommandKind::FileBacked {
            template: body.trim().to_string(),
        },
    }))
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

#[cfg(test)]
mod tests {
    use super::discover_markdown_commands;

    #[test]
    fn discovers_project_markdown_command() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".rustcode").join("commands");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("explain.md"),
            "---\ndescription: Explain target\nargument-hint: <target>\n---\nExplain $ARGUMENTS",
        )
        .unwrap();

        let commands = discover_markdown_commands(temp.path());
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "explain");
        assert_eq!(commands[0].description, "Explain target");
    }
}
