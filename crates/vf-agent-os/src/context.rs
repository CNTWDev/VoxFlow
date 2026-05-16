use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const CONTEXT_FILE_NAMES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "HERMES.md",
    ".hermes.md",
    ".cursorrules",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDocument {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ContextLoader {
    workspace: PathBuf,
}

impl ContextLoader {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    pub fn load_for(&self, task_path: impl AsRef<Path>) -> Result<Vec<ContextDocument>> {
        let mut docs = Vec::new();
        let mut dirs = self.ancestor_dirs(task_path.as_ref());
        dirs.reverse();

        for dir in dirs {
            for file_name in CONTEXT_FILE_NAMES {
                let path = dir.join(file_name);
                if path.is_file() {
                    let content = fs::read_to_string(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?;
                    docs.push(ContextDocument { path, content });
                }
            }

            let cursor_rules = dir.join(".cursor").join("rules");
            if cursor_rules.is_dir() {
                for entry in fs::read_dir(&cursor_rules)
                    .with_context(|| format!("failed to read {}", cursor_rules.display()))?
                {
                    let path = entry?.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("mdc") {
                        let content = fs::read_to_string(&path)
                            .with_context(|| format!("failed to read {}", path.display()))?;
                        docs.push(ContextDocument { path, content });
                    }
                }
            }
        }

        Ok(docs)
    }

    fn ancestor_dirs(&self, task_path: &Path) -> Vec<PathBuf> {
        let absolute = if task_path.is_absolute() {
            task_path.to_path_buf()
        } else {
            self.workspace.join(task_path)
        };
        let start = if absolute.is_dir() {
            absolute
        } else {
            absolute
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.workspace.clone())
        };

        let mut dirs = Vec::new();
        let mut current = Some(start.as_path());
        while let Some(dir) = current {
            if dir.starts_with(&self.workspace) {
                dirs.push(dir.to_path_buf());
            }
            if dir == self.workspace {
                break;
            }
            current = dir.parent();
        }
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_workspace_and_nested_context_in_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("crates").join("demo");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.path().join("AGENTS.md"), "root rules").unwrap();
        fs::write(nested.join("AGENTS.md"), "nested rules").unwrap();

        let loader = ContextLoader::new(dir.path());
        let docs = loader.load_for(nested.join("src/lib.rs")).unwrap();

        let contents: Vec<_> = docs.iter().map(|doc| doc.content.as_str()).collect();
        assert_eq!(contents, vec!["root rules", "nested rules"]);
    }
}
