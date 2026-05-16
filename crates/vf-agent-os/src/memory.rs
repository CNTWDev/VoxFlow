use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub memory_md: String,
    pub user_md: String,
}

#[derive(Debug, Clone)]
pub struct MemoryStore {
    root: PathBuf,
}

impl MemoryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn snapshot(&self) -> Result<MemorySnapshot> {
        Ok(MemorySnapshot {
            memory_md: self.read_optional("MEMORY.md")?,
            user_md: self.read_optional("USER.md")?,
        })
    }

    pub fn propose_update(&self, kind: MemoryKind, patch: String, reason: String) -> MemoryUpdate {
        MemoryUpdate {
            kind,
            patch,
            reason,
        }
    }

    pub fn apply_update(&self, update: &MemoryUpdate) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let path = self.root.join(update.kind.file_name());
        let mut current = self.read_optional(update.kind.file_name())?;
        if !current.is_empty() && !current.ends_with('\n') {
            current.push('\n');
        }
        current.push_str("\n");
        current.push_str(update.patch.trim());
        current.push('\n');
        fs::write(&path, current).with_context(|| format!("failed to write {}", path.display()))
    }

    fn read_optional(&self, file_name: &str) -> Result<String> {
        let path = self.root.join(file_name);
        match fs::read_to_string(&path) {
            Ok(content) => Ok(content),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Memory,
    User,
}

impl MemoryKind {
    fn file_name(self) -> &'static str {
        match self {
            Self::Memory => "MEMORY.md",
            Self::User => "USER.md",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryUpdate {
    pub kind: MemoryKind,
    pub patch: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_memory_files_return_empty_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());

        let snapshot = store.snapshot().unwrap();

        assert_eq!(snapshot.memory_md, "");
        assert_eq!(snapshot.user_md, "");
    }

    #[test]
    fn applies_memory_update_as_append_only_patch() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());
        let update = store.propose_update(
            MemoryKind::Memory,
            "- Reuse the existing prompt compiler.".into(),
            "stable project convention".into(),
        );

        store.apply_update(&update).unwrap();

        let snapshot = store.snapshot().unwrap();
        assert!(snapshot
            .memory_md
            .contains("- Reuse the existing prompt compiler."));
    }
}
