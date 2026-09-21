---
name: agent-session-bootstrap
description: Use when configuring Claude Code or Codex sessions, skills, MCP servers, durable instructions, agent runners, or future-session defaults. Ensures on-demand KB recall and token-efficient behavior are configured without relying on user reminders.
---

# Agent Session Bootstrap

Checklist:

1. Confirm `knowledge` MCP is configured with `KB_BEARER_TOKEN`.
2. Keep runner MCP profiles narrow: `minimal` by default, wider
   `frontend`/`cluster`/`code-intel`/`full-diagnostic` profiles only when the
   task needs those tools.
3. Add project/global memory files so future sessions consult and update the KB.
4. Validate with a `knowledge.recall` call.

The estate ships **no agent hooks**. Recall and capture are on demand: call
the recall skill or the `knowledge.recall` MCP tool when a task needs prior
context. A `PreToolUse` or `Stop` hook that reaches the KB is retired
machinery -- `setup-workstation.sh` removes any that a machine still has.

Every Codex project skill or durable instruction must have an equivalent
Claude implementation in the same branch. Treat Codex-only `.agents`/`.codex`
files as incomplete until `.claude`/`CLAUDE.md`/installer parity exists.
