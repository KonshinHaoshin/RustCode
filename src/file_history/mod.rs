use crate::{config::project_file_history_dir, session::Session};
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

#[cfg(test)]
const DEFAULT_MAX_TRACKED_FILES: usize = 200;
const IGNORED_DIR_NAMES: &[&str] = &[".git", "target", "node_modules"];
const IGNORED_RELATIVE_PREFIXES: &[&str] = &["rustcode/state/", "rustcode/sessions/"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMutationMetadata {
    pub session_id: String,
    pub original_path: String,
    pub absolute_path: String,
    pub existed_before: bool,
    pub backup_file: Option<String>,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileRewindResult {
    pub restored_files: Vec<String>,
    pub deleted_files: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileHistoryOrigin {
    FileWrite,
    FileEdit,
    ExecuteCommandSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileHistoryBatchEntry {
    pub mutation: FileMutationMetadata,
    pub origin: FileHistoryOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChangeDescriptor {
    pub path: String,
    pub origin: FileHistoryOrigin,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct CommandSnapshot {
    entries: BTreeMap<String, CommandSnapshotEntry>,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct CommandSnapshotEntry {
    signature: u64,
    mutation: FileMutationMetadata,
}

#[derive(Debug, Clone)]
struct ProjectScan {
    entries: BTreeMap<String, u64>,
    truncated: bool,
}

#[derive(Debug, Clone)]
pub struct FileHistoryStore {
    root_dir: PathBuf,
    project_root: Option<PathBuf>,
}

impl FileHistoryStore {
    pub fn for_project(project_root: Option<&Path>) -> anyhow::Result<Self> {
        let root_dir = project_file_history_dir(project_root)
            .ok_or_else(|| anyhow::anyhow!("Unable to determine file history directory"))?;
        Ok(Self {
            root_dir,
            project_root: project_root.map(Path::to_path_buf),
        })
    }

    pub fn capture_mutation(
        &self,
        session_id: &str,
        file_path: &str,
    ) -> anyhow::Result<FileMutationMetadata> {
        let absolute_path = self.resolve_path(file_path);
        let existed_before = absolute_path.exists();
        let mut backup_file = None;

        std::fs::create_dir_all(self.session_dir(session_id))?;
        if existed_before {
            let backup_name = format!("{}.bak", uuid::Uuid::new_v4());
            let backup_path = self.session_dir(session_id).join(&backup_name);
            std::fs::copy(&absolute_path, &backup_path)?;
            backup_file = Some(backup_name);
        }

        Ok(FileMutationMetadata {
            session_id: session_id.to_string(),
            original_path: file_path.to_string(),
            absolute_path: absolute_path.to_string_lossy().to_string(),
            existed_before,
            backup_file,
            captured_at: Utc::now(),
        })
    }

    pub fn file_history_has_any_changes(&self, session: &Session, message_id: &str) -> bool {
        self.collect_change_entries_after_message(session, message_id)
            .map(|(entries, _)| !entries.is_empty())
            .unwrap_or(false)
    }

    pub fn file_history_get_changed_files(
        &self,
        session: &Session,
        message_id: &str,
    ) -> Vec<String> {
        let mut files = BTreeSet::new();
        if let Ok(descriptors) = self.file_history_get_change_descriptors(session, message_id) {
            for descriptor in descriptors {
                files.insert(descriptor.path);
            }
        }
        files.into_iter().collect()
    }

    pub fn file_history_get_change_descriptors(
        &self,
        session: &Session,
        message_id: &str,
    ) -> anyhow::Result<Vec<FileChangeDescriptor>> {
        let (entries, truncated) =
            self.collect_change_entries_after_message(session, message_id)?;
        let mut descriptors = BTreeMap::new();
        for entry in entries {
            descriptors
                .entry(entry.mutation.original_path.clone())
                .and_modify(|descriptor: &mut FileChangeDescriptor| {
                    descriptor.truncated |= truncated;
                })
                .or_insert(FileChangeDescriptor {
                    path: entry.mutation.original_path,
                    origin: entry.origin,
                    truncated,
                });
        }
        Ok(descriptors.into_values().collect())
    }

    pub fn rewind_session_to_message(
        &self,
        session: &Session,
        message_id: &str,
    ) -> anyhow::Result<FileRewindResult> {
        let (entries, _) = self.collect_change_entries_after_message(session, message_id)?;
        let mut restored_files = Vec::new();
        let mut deleted_files = Vec::new();

        for mutation in entries.into_iter().map(|entry| entry.mutation).rev() {
            let absolute_path = PathBuf::from(&mutation.absolute_path);
            if mutation.existed_before {
                let backup_name = mutation.backup_file.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("Missing backup file for {}", mutation.original_path)
                })?;
                let backup_path = self.session_dir(&mutation.session_id).join(backup_name);
                if let Some(parent) = absolute_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&backup_path, &absolute_path)?;
                restored_files.push(mutation.original_path);
            } else if absolute_path.exists() {
                std::fs::remove_file(&absolute_path)?;
                deleted_files.push(mutation.original_path);
            }
        }

        Ok(FileRewindResult {
            restored_files,
            deleted_files,
        })
    }

    pub fn capture_command_snapshot(
        &self,
        session_id: &str,
        max_files: usize,
    ) -> anyhow::Result<CommandSnapshot> {
        let scan = self.scan_project(max_files.max(1))?;
        let mut entries = BTreeMap::new();
        for (relative, signature) in scan.entries {
            let mutation = self.capture_mutation(session_id, &relative)?;
            entries.insert(
                relative,
                CommandSnapshotEntry {
                    signature,
                    mutation,
                },
            );
        }
        Ok(CommandSnapshot {
            entries,
            truncated: scan.truncated,
        })
    }

    pub fn diff_command_snapshot(
        &self,
        before: &CommandSnapshot,
        session_id: &str,
        max_files: usize,
    ) -> anyhow::Result<(Vec<FileMutationMetadata>, bool)> {
        let after = self.scan_project(max_files.max(1))?;
        let mut changed = Vec::new();

        for (relative, before_entry) in &before.entries {
            match after.entries.get(relative) {
                Some(after_signature) if after_signature == &before_entry.signature => {}
                _ => changed.push(before_entry.mutation.clone()),
            }
        }

        for relative in after.entries.keys() {
            if before.entries.contains_key(relative) {
                continue;
            }
            changed.push(self.capture_new_file_metadata(session_id, relative));
        }

        Ok((changed, before.truncated || after.truncated))
    }

    fn capture_new_file_metadata(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> FileMutationMetadata {
        let absolute_path = self.resolve_path(relative_path);
        FileMutationMetadata {
            session_id: session_id.to_string(),
            original_path: relative_path.to_string(),
            absolute_path: absolute_path.to_string_lossy().to_string(),
            existed_before: false,
            backup_file: None,
            captured_at: Utc::now(),
        }
    }

    fn collect_change_entries_after_message(
        &self,
        session: &Session,
        message_id: &str,
    ) -> anyhow::Result<(Vec<FileHistoryBatchEntry>, bool)> {
        let target_index = session
            .messages
            .iter()
            .position(|message| message.id == message_id)
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;
        let target = session
            .messages
            .get(target_index)
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;
        if !target.role.eq_ignore_ascii_case("user") {
            return Err(anyhow::anyhow!(
                "Rewind requires a user message id, got role {}",
                target.role
            ));
        }

        let mut entries = Vec::new();
        let mut truncated = false;
        for message in session.messages.iter().skip(target_index + 1) {
            let Some(tool_result) = &message.tool_result else {
                continue;
            };
            if let Some(metadata) = tool_result.metadata.get("file_history") {
                let mutation: FileMutationMetadata = serde_json::from_value(metadata.clone())?;
                let origin = match tool_result.name.as_str() {
                    "file_edit" => FileHistoryOrigin::FileEdit,
                    _ => FileHistoryOrigin::FileWrite,
                };
                entries.push(FileHistoryBatchEntry { mutation, origin });
            }
            if let Some(metadata) = tool_result.metadata.get("file_history_batch_v2") {
                let batch: Vec<FileHistoryBatchEntry> = serde_json::from_value(metadata.clone())?;
                entries.extend(batch);
            }
            if let Some(metadata) = tool_result.metadata.get("file_history_batch") {
                let batch: Vec<FileMutationMetadata> = serde_json::from_value(metadata.clone())?;
                entries.extend(batch.into_iter().map(|mutation| FileHistoryBatchEntry {
                    mutation,
                    origin: FileHistoryOrigin::ExecuteCommandSnapshot,
                }));
            }
            truncated |= tool_result
                .metadata
                .get("file_history_truncated")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
        }
        Ok((entries, truncated))
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root_dir.join(session_id)
    }

    fn resolve_path(&self, file_path: &str) -> PathBuf {
        let path = PathBuf::from(file_path);
        if path.is_absolute() {
            path
        } else {
            self.project_root
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
                .join(path)
        }
    }

    fn scan_project(&self, max_files: usize) -> anyhow::Result<ProjectScan> {
        let Some(root) = &self.project_root else {
            return Ok(ProjectScan {
                entries: BTreeMap::new(),
                truncated: false,
            });
        };
        let mut entries = BTreeMap::new();
        let mut truncated = false;
        self.scan_dir(root, root, max_files, &mut entries, &mut truncated)?;
        Ok(ProjectScan { entries, truncated })
    }

    fn scan_dir(
        &self,
        root: &Path,
        dir: &Path,
        max_files: usize,
        entries: &mut BTreeMap<String, u64>,
        truncated: &mut bool,
    ) -> anyhow::Result<()> {
        if *truncated {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)
            .with_context(|| format!("Failed to read directory {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if IGNORED_DIR_NAMES
                    .iter()
                    .any(|ignored| ignored.eq_ignore_ascii_case(&name))
                {
                    continue;
                }
                self.scan_dir(root, &path, max_files, entries, truncated)?;
                if *truncated {
                    return Ok(());
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(relative) = self.relative_path_for(root, &path) else {
                continue;
            };
            if is_ignored_relative_path(&relative) {
                continue;
            }
            if entries.len() >= max_files {
                *truncated = true;
                return Ok(());
            }
            entries.insert(relative, file_signature(&path)?);
        }
        Ok(())
    }

    fn relative_path_for(&self, root: &Path, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(root).ok()?;
        Some(relative.to_string_lossy().replace('\\', "/"))
    }
}

fn is_ignored_relative_path(path: &str) -> bool {
    IGNORED_RELATIVE_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

fn file_signature(path: &Path) -> anyhow::Result<u64> {
    let bytes = std::fs::read(path)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Message, Session, TranscriptEntryType};

    #[test]
    fn capture_and_rewind_new_file_restores_absence() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let store = FileHistoryStore::for_project(Some(&project_root)).unwrap();
        let file_path = "notes.txt";
        let absolute_path = project_root.join(file_path);

        let mutation = store.capture_mutation("session-1", file_path).unwrap();
        std::fs::write(&absolute_path, "hello").unwrap();

        let session = Session {
            messages: vec![
                Message {
                    id: "user-1".to_string(),
                    role: "user".to_string(),
                    content: "write file".to_string(),
                    tool_calls: Vec::new(),
                    tool_result: None,
                    entry_type: TranscriptEntryType::Message,
                    parent_id: None,
                    timestamp: Utc::now(),
                },
                Message {
                    id: "tool-1".to_string(),
                    role: "tool".to_string(),
                    content: "ok".to_string(),
                    tool_calls: Vec::new(),
                    tool_result: Some(crate::runtime::RuntimeToolResult {
                        tool_call_id: "call-1".to_string(),
                        name: "file_write".to_string(),
                        content: "ok".to_string(),
                        is_error: false,
                        metadata: std::collections::HashMap::from([(
                            "file_history".to_string(),
                            serde_json::to_value(mutation).unwrap(),
                        )]),
                    }),
                    entry_type: TranscriptEntryType::Message,
                    parent_id: Some("user-1".to_string()),
                    timestamp: Utc::now(),
                },
            ],
            ..Session::default()
        };

        let rewind = store.rewind_session_to_message(&session, "user-1").unwrap();
        assert!(!absolute_path.exists());
        assert_eq!(rewind.deleted_files, vec!["notes.txt".to_string()]);
    }

    #[test]
    fn command_snapshot_detects_modified_and_new_files() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::write(project_root.join("a.txt"), "before").unwrap();
        let store = FileHistoryStore::for_project(Some(&project_root)).unwrap();

        let snapshot = store
            .capture_command_snapshot("session-1", DEFAULT_MAX_TRACKED_FILES)
            .unwrap();
        std::fs::write(project_root.join("a.txt"), "after").unwrap();
        std::fs::write(project_root.join("b.txt"), "new").unwrap();

        let (mut mutations, truncated) = store
            .diff_command_snapshot(&snapshot, "session-1", DEFAULT_MAX_TRACKED_FILES)
            .unwrap();
        mutations.sort_by(|a, b| a.original_path.cmp(&b.original_path));

        assert!(!truncated);
        assert_eq!(
            mutations
                .into_iter()
                .map(|mutation| mutation.original_path)
                .collect::<Vec<_>>(),
            vec!["a.txt".to_string(), "b.txt".to_string()]
        );
    }

    #[test]
    fn rewind_reads_file_history_batch_v2() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let store = FileHistoryStore::for_project(Some(&project_root)).unwrap();
        let absolute_path = project_root.join("notes.txt");

        let mutation = store.capture_mutation("session-1", "notes.txt").unwrap();
        std::fs::write(&absolute_path, "hello").unwrap();

        let session = Session {
            messages: vec![
                Message {
                    id: "user-1".to_string(),
                    role: "user".to_string(),
                    content: "write file".to_string(),
                    tool_calls: Vec::new(),
                    tool_result: None,
                    entry_type: TranscriptEntryType::Message,
                    parent_id: None,
                    timestamp: Utc::now(),
                },
                Message {
                    id: "tool-1".to_string(),
                    role: "tool".to_string(),
                    content: "ok".to_string(),
                    tool_calls: Vec::new(),
                    tool_result: Some(crate::runtime::RuntimeToolResult {
                        tool_call_id: "call-1".to_string(),
                        name: "execute_command".to_string(),
                        content: "ok".to_string(),
                        is_error: false,
                        metadata: std::collections::HashMap::from([(
                            "file_history_batch_v2".to_string(),
                            serde_json::to_value(vec![FileHistoryBatchEntry {
                                mutation,
                                origin: FileHistoryOrigin::ExecuteCommandSnapshot,
                            }])
                            .unwrap(),
                        )]),
                    }),
                    entry_type: TranscriptEntryType::Message,
                    parent_id: Some("user-1".to_string()),
                    timestamp: Utc::now(),
                },
            ],
            ..Session::default()
        };

        let descriptors = store
            .file_history_get_change_descriptors(&session, "user-1")
            .unwrap();
        assert_eq!(
            descriptors[0].origin,
            FileHistoryOrigin::ExecuteCommandSnapshot
        );

        let rewind = store.rewind_session_to_message(&session, "user-1").unwrap();
        assert!(!absolute_path.exists());
        assert_eq!(rewind.deleted_files, vec!["notes.txt".to_string()]);
    }
}
