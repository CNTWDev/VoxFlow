---
name = "Prompt Assembly"
description = "Build layered Agent OS prompts with stable cached context and per-turn ephemeral context."
triggers = ["prompt assembly", "prompt compiler", "agent os prompt", "system prompt"]
---

# Prompt Assembly

Use this workflow when changing Agent OS prompt construction.

1. Keep stable content in the cached system prompt: identity, rules, frozen memory, user preferences, skills index, and project context.
2. Keep current task details in ephemeral context: task, selected files, loaded skill bodies, runtime constraints, and current state.
3. Do not inject full skill bodies into the cached prompt. Load them only when selected.
4. Treat memory as a session-start snapshot. Mid-session writes should update files for future sessions, not silently change the current prompt.
5. Include diagnostics for loaded context paths, indexed skills, loaded skills, and prompt size.
