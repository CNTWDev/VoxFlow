---
name = "Memory Maintenance"
description = "Decide whether durable facts belong in MEMORY.md, USER.md, or a reusable skill."
triggers = ["memory", "remember", "save preference", "skill update"]
---

# Memory Maintenance

Use this workflow after complex or repeated work.

1. Save stable project facts, conventions, and known pitfalls to `MEMORY.md`.
2. Save user preferences and long-lived collaboration style to `USER.md`.
3. Save repeatable workflows to `skills/<name>/SKILL.md`.
4. Do not save secrets, transient logs, one-off guesses, or facts that are likely to become stale quickly.
5. Prefer append-only or minimal patch updates. Avoid rewriting the whole file unless restructuring is necessary.
