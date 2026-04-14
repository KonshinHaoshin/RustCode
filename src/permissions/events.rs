use crate::config::project_permission_events_path;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEvent {
    pub tool_name: String,
    pub decision: String,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Default)]
pub struct PermissionEventStore;

impl PermissionEventStore {
    pub fn load(cwd: Option<&Path>) -> anyhow::Result<Vec<PermissionEvent>> {
        let Some(path) = project_permission_events_path(cwd) else {
            return Ok(Vec::new());
        };
        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(path)?;
        let events = serde_json::from_str(&content)?;
        Ok(events)
    }

    pub fn append(cwd: Option<&Path>, event: PermissionEvent) -> anyhow::Result<()> {
        let Some(path) = project_permission_events_path(cwd) else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut events = Self::load(cwd)?;
        events.push(event);
        if events.len() > 50 {
            let drain = events.len().saturating_sub(50);
            events.drain(0..drain);
        }
        std::fs::write(path, serde_json::to_string_pretty(&events)?)?;
        Ok(())
    }
}
