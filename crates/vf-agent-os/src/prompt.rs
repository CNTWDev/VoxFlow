use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
};

use anyhow::Result;

use crate::{
    context::{ContextDocument, ContextLoader},
    memory::{MemorySnapshot, MemoryStore},
    skill::{SkillBody, SkillRegistry, SkillSummary},
};

const AGENT_OS_SYSTEM_PROMPT: &str = r#"You are running inside Agent OS.

Priority order:
1. Follow system and developer instructions.
2. Follow project rules from AGENTS.md.
3. Use USER.md preferences when they do not conflict.
4. Use MEMORY.md for stable facts and lessons.
5. Use matching skills when they apply.
6. Ask only when a missing decision materially changes the result.

Memory rules:
- Save stable, reusable facts only.
- Do not save secrets, transient logs, or guesses.
- Mid-session memory writes affect future sessions, not this frozen prompt.

Skill rules:
- Load a skill when its trigger matches the task.
- After complex reusable work, consider creating or patching a skill.
- Prefer small accurate patches over rewriting a skill.

Execution rules:
- Verify discoverable facts with tools.
- Keep changes scoped.
- Preserve user work.
- Record important outcomes in session history."#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptInput {
    pub session_id: String,
    pub task: String,
    pub workspace: PathBuf,
    pub task_path: Option<PathBuf>,
    pub selected_files: Vec<PathBuf>,
    pub loaded_skill_ids: Vec<String>,
    pub runtime_constraints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBundle {
    pub cached_system: String,
    pub ephemeral_context: String,
    pub cache_key: String,
    pub diagnostics: PromptDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDiagnostics {
    pub context_paths: Vec<PathBuf>,
    pub indexed_skill_ids: Vec<String>,
    pub loaded_skill_ids: Vec<String>,
    pub cached_system_chars: usize,
    pub ephemeral_context_chars: usize,
}

#[derive(Debug, Clone)]
pub struct PromptCompiler {
    agent_os_root: PathBuf,
}

impl PromptCompiler {
    pub fn new(agent_os_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_os_root: agent_os_root.into(),
        }
    }

    pub fn compile(&self, input: PromptInput) -> Result<PromptBundle> {
        let memory = MemoryStore::new(&self.agent_os_root).snapshot()?;
        let skills = SkillRegistry::new(&self.agent_os_root).index()?;
        let context_loader = ContextLoader::new(&input.workspace);
        let context_docs = context_loader.load_for(
            input
                .task_path
                .as_deref()
                .unwrap_or_else(|| input.workspace.as_path()),
        )?;
        let loaded_skills = self.load_skills(&input.loaded_skill_ids)?;

        let cached_system = render_cached_system(&memory, &skills, &context_docs);
        let ephemeral_context = render_ephemeral_context(&input, &loaded_skills);
        let cache_key = cache_key(&cached_system);
        let diagnostics = PromptDiagnostics {
            context_paths: context_docs.into_iter().map(|doc| doc.path).collect(),
            indexed_skill_ids: skills.into_iter().map(|skill| skill.id).collect(),
            loaded_skill_ids: loaded_skills
                .iter()
                .map(|skill| skill.summary.id.clone())
                .collect(),
            cached_system_chars: cached_system.chars().count(),
            ephemeral_context_chars: ephemeral_context.chars().count(),
        };

        Ok(PromptBundle {
            cached_system,
            ephemeral_context,
            cache_key,
            diagnostics,
        })
    }

    fn load_skills(&self, skill_ids: &[String]) -> Result<Vec<SkillBody>> {
        let registry = SkillRegistry::new(&self.agent_os_root);
        let mut bodies = Vec::new();
        for skill_id in skill_ids {
            if let Some(skill) = registry.load(skill_id)? {
                bodies.push(skill);
            }
        }
        Ok(bodies)
    }
}

fn render_cached_system(
    memory: &MemorySnapshot,
    skills: &[SkillSummary],
    context_docs: &[ContextDocument],
) -> String {
    let mut out = String::new();
    out.push_str(AGENT_OS_SYSTEM_PROMPT);
    out.push_str("\n\n## USER.md\n");
    out.push_str(empty_marker(&memory.user_md));
    out.push_str("\n\n## MEMORY.md\n");
    out.push_str(empty_marker(&memory.memory_md));
    out.push_str("\n\n## Skills Index\n");
    if skills.is_empty() {
        out.push_str("(none)\n");
    } else {
        for skill in skills {
            out.push_str(&format!(
                "- {} (`{}`): {} Triggers: {}\n",
                skill.name,
                skill.id,
                empty_inline(&skill.description),
                if skill.triggers.is_empty() {
                    "(none)".to_string()
                } else {
                    skill.triggers.join(", ")
                }
            ));
        }
    }
    out.push_str("\n## Project Context\n");
    if context_docs.is_empty() {
        out.push_str("(none)\n");
    } else {
        for doc in context_docs {
            out.push_str(&format!("\n### {}\n", doc.path.display()));
            out.push_str(doc.content.trim());
            out.push('\n');
        }
    }
    out
}

fn render_ephemeral_context(input: &PromptInput, loaded_skills: &[SkillBody]) -> String {
    let mut out = String::new();
    out.push_str("## Current Task\n");
    out.push_str(input.task.trim());
    out.push_str("\n\n## Runtime\n");
    out.push_str(&format!("- session_id: {}\n", input.session_id));
    out.push_str(&format!("- workspace: {}\n", input.workspace.display()));
    if input.selected_files.is_empty() {
        out.push_str("- selected_files: (none)\n");
    } else {
        for file in &input.selected_files {
            out.push_str(&format!("- selected_file: {}\n", file.display()));
        }
    }
    if !input.runtime_constraints.is_empty() {
        out.push_str("\n## Runtime Constraints\n");
        for constraint in &input.runtime_constraints {
            out.push_str(&format!("- {}\n", constraint.trim()));
        }
    }
    out.push_str("\n## Loaded Skills\n");
    if loaded_skills.is_empty() {
        out.push_str("(none)\n");
    } else {
        for skill in loaded_skills {
            out.push_str(&format!(
                "\n### {} (`{}`)\n{}\n",
                skill.summary.name, skill.summary.id, skill.body
            ));
        }
    }
    out
}

fn empty_marker(value: &str) -> &str {
    if value.trim().is_empty() {
        "(empty)\n"
    } else {
        value
    }
}

fn empty_inline(value: &str) -> &str {
    if value.trim().is_empty() {
        "(no description)"
    } else {
        value
    }
}

fn cache_key(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn compiles_stable_cached_prompt_and_ephemeral_loaded_skill() {
        let dir = tempfile::tempdir().unwrap();
        let agent_root = dir.path().join(".agent-os");
        fs::create_dir_all(agent_root.join("skills").join("review")).unwrap();
        fs::write(agent_root.join("USER.md"), "- Prefer concise answers.\n").unwrap();
        fs::write(agent_root.join("MEMORY.md"), "- Workspace uses Rust.\n").unwrap();
        fs::write(
            agent_root.join("skills").join("review").join("SKILL.md"),
            "---\nname = \"Review\"\ndescription = \"Review code.\"\ntriggers = [\"review\"]\n---\n# Review\nFind risks first.\n",
        )
        .unwrap();
        fs::write(dir.path().join("AGENTS.md"), "Use rg before grep.").unwrap();

        let compiler = PromptCompiler::new(&agent_root);
        let bundle = compiler
            .compile(PromptInput {
                session_id: "s1".into(),
                task: "review this change".into(),
                workspace: dir.path().to_path_buf(),
                task_path: None,
                selected_files: vec![],
                loaded_skill_ids: vec!["review".into()],
                runtime_constraints: vec!["no destructive commands".into()],
            })
            .unwrap();

        assert!(bundle.cached_system.contains("Prefer concise answers"));
        assert!(bundle.cached_system.contains("Review (`review`)"));
        assert!(bundle.cached_system.contains("Use rg before grep"));
        assert!(bundle.ephemeral_context.contains("# Review"));
        assert_eq!(bundle.diagnostics.loaded_skill_ids, vec!["review"]);
    }
}
