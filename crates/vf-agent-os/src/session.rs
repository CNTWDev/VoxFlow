use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEvent {
    pub kind: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions_root: PathBuf,
}

impl SessionStore {
    pub fn new(agent_os_root: impl Into<PathBuf>) -> Self {
        Self {
            sessions_root: agent_os_root.into().join("sessions"),
        }
    }

    pub fn append_event(&self, session_id: &str, event: &SessionEvent) -> Result<()> {
        fs::create_dir_all(&self.sessions_root)
            .with_context(|| format!("failed to create {}", self.sessions_root.display()))?;
        let path = self.sessions_root.join(format!("{session_id}.jsonl"));
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let line = serde_json::to_string(event).context("failed to encode session event")?;
        writeln!(file, "{line}").with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn save_checkpoint(&self, session_id: &str, summary: &str) -> Result<()> {
        fs::create_dir_all(&self.sessions_root)
            .with_context(|| format!("failed to create {}", self.sessions_root.display()))?;
        let path = self.sessions_root.join(format!("{session_id}.checkpoint.md"));
        fs::write(&path, summary).with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn restore_checkpoint(&self, session_id: &str) -> Result<Option<String>> {
        let path = self.sessions_root.join(format!("{session_id}.checkpoint.md"));
        match fs::read_to_string(&path) {
            Ok(content) => Ok(Some(content)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_restores_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        store.save_checkpoint("abc", "current task state").unwrap();

        assert_eq!(
            store.restore_checkpoint("abc").unwrap(),
            Some("current task state".to_string())
        );
    }
}
