# Agent OS Rules

This workspace uses a lightweight Agent OS model inspired by Hermes Agent.

- Prompt assembly is layered: identity, project rules, frozen memory, user preferences, skills index, project context, then ephemeral task context.
- Memory files are read as a frozen snapshot at session start. Updates should affect future sessions, not mutate the current prompt mid-task.
- Skills are indexed by metadata and loaded on demand. Do not place full skill bodies in the cached system prompt.
- Store reusable workflows as skills. Store stable facts and preferences as memory.
- Keep the first implementation filesystem-based. Do not add a database or vector index until plain files are demonstrably insufficient.
