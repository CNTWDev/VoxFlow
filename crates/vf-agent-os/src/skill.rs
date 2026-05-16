use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBody {
    pub summary: SkillSummary,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct SkillRegistry {
    skills_root: PathBuf,
}

impl SkillRegistry {
    pub fn new(agent_os_root: impl Into<PathBuf>) -> Self {
        Self {
            skills_root: agent_os_root.into().join("skills"),
        }
    }

    pub fn index(&self) -> Result<Vec<SkillSummary>> {
        if !self.skills_root.is_dir() {
            return Ok(Vec::new());
        }

        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.skills_root)
            .with_context(|| format!("failed to read {}", self.skills_root.display()))?
        {
            let path = entry?.path().join("SKILL.md");
            if path.is_file() {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                summaries.push(parse_skill_summary(&path, &content)?);
            }
        }
        summaries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(summaries)
    }

    pub fn load(&self, skill_id: &str) -> Result<Option<SkillBody>> {
        let path = self.skills_root.join(skill_id).join("SKILL.md");
        if !path.is_file() {
            return Ok(None);
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let summary = parse_skill_summary(&path, &content)?;
        let body = strip_frontmatter(&content).trim().to_string();
        Ok(Some(SkillBody { summary, body }))
    }

    pub fn match_task(&self, task: &str) -> Result<Vec<SkillSummary>> {
        let lower_task = task.to_lowercase();
        let matches = self
            .index()?
            .into_iter()
            .filter(|skill| {
                skill
                    .triggers
                    .iter()
                    .any(|trigger| lower_task.contains(&trigger.to_lowercase()))
                    || lower_task.contains(&skill.name.to_lowercase())
            })
            .collect();
        Ok(matches)
    }
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    triggers: Option<Vec<String>>,
}

fn parse_skill_summary(path: &Path, content: &str) -> Result<SkillSummary> {
    let id = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let frontmatter = parse_frontmatter(content)?;
    Ok(SkillSummary {
        name: frontmatter.name.unwrap_or_else(|| id.clone()),
        description: frontmatter.description.unwrap_or_default(),
        triggers: frontmatter.triggers.unwrap_or_default(),
        id,
        path: path.to_path_buf(),
    })
}

fn parse_frontmatter(content: &str) -> Result<SkillFrontmatter> {
    let Some(rest) = content.strip_prefix("---\n") else {
        return Ok(SkillFrontmatter {
            name: None,
            description: None,
            triggers: None,
        });
    };
    let Some((frontmatter, _body)) = rest.split_once("\n---") else {
        return Ok(SkillFrontmatter {
            name: None,
            description: None,
            triggers: None,
        });
    };
    toml::from_str(frontmatter).context("failed to parse skill frontmatter")
}

fn strip_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content;
    };
    rest.split_once("\n---")
        .map(|(_, body)| body)
        .unwrap_or(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_skills_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("debug-build");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name = "Debug Build"
description = "Fix build failures."
triggers = ["build failed", "cargo error"]
---
# Debug Build
"#,
        )
        .unwrap();

        let registry = SkillRegistry::new(dir.path());
        let skills = registry.index().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "debug-build");
        assert_eq!(skills[0].triggers, vec!["build failed", "cargo error"]);
    }

    #[test]
    fn loads_skill_body_without_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("review");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname = \"Review\"\n---\n# Review\nCheck risks.\n",
        )
        .unwrap();

        let registry = SkillRegistry::new(dir.path());
        let skill = registry.load("review").unwrap().unwrap();

        assert_eq!(skill.body, "# Review\nCheck risks.");
    }
}
