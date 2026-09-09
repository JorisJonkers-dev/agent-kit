---
name: agent-session-bootstrap
description: Use when configuring Claude Code or Codex sessions, skills, MCP servers, durable instructions, agent runners, or future-session defaults. Ensures on-demand KB recall and token-efficient behavior are configured without relying on user reminders.
---

# Agent Session Bootstrap

Checklist:

1. Locate the active user and project config layers:
   `~/.claude/settings.json`, `~/.claude/CLAUDE.md`, project
   `CLAUDE.md`, project `.claude/settings.json`, `~/.claude/skills`,
   Codex `~/.codex/config.toml`, repo `AGENTS.md`, and `.agents/skills`.
2. Ensure the `knowledge` MCP server is configured and uses
   `KB_BEARER_TOKEN` rather than an inline secret where possible.
3. Keep runner MCP profiles narrow:
   `minimal` for routine work, and `frontend`, `cluster`, `code-intel`,
   or `full-diagnostic` only when the task needs those tools. Prefer
   `AGENT_MCP_PROFILE` for one runner and
   `AGENT_RUNTIME_DEFAULT_MCP_PROFILE` only for fleet-wide default changes.
4. Add or update memory files so future sessions know to consult and
   update the KB without user reminders.
5. Validate with at least one `tools/list` or `knowledge.recall` MCP
   call.

The estate ships **no agent hooks**. Recall and capture are on demand: call
the recall skill or the `knowledge.recall` MCP tool when a task needs prior
context. A `PreToolUse` or `Stop` hook that reaches the KB is retired
machinery -- `install-agents.sh` removes any that a machine still has.

Every Codex project skill or durable instruction must have an
equivalent Claude implementation in the same branch. Treat Codex-only
`.agents`/`.codex` files as incomplete until `.claude`/`CLAUDE.md`/
installer parity exists.

Do not put bearer tokens, secrets, or full transcripts into committed
files.
