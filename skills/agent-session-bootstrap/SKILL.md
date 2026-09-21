---
name: agent-session-bootstrap
description: Use when configuring Claude Code or Codex sessions, skills, MCP servers, durable instructions, agent runners, or future-session defaults. Ensures on-demand memory recall and token-efficient behavior are configured without relying on user reminders.
---

# Agent Session Bootstrap

Checklist:

1. Locate the active user and project config layers:
   `~/.claude/settings.json`, `~/.claude/CLAUDE.md`, project
   `CLAUDE.md`, project `.claude/settings.json`, `~/.claude/skills`,
   Codex `~/.codex/config.toml`, repo `AGENTS.md`, and `.agents/skills`.
2. Ensure the `memory-api` (Hindsight) and `memory-mcp` (Basic Memory) MCP
   servers are configured, each with its own per-host token
   (`HINDSIGHT_API_TOKEN`, `MEMORY_MCP_TOKEN`) rather than an inline secret.
3. Keep runner MCP profiles narrow:
   `minimal` for routine work, and `frontend`, `cluster`, `code-intel`,
   or `full-diagnostic` only when the task needs those tools. Prefer
   `AGENT_MCP_PROFILE` for one runner and
   `AGENT_RUNTIME_DEFAULT_MCP_PROFILE` only for fleet-wide default changes.
4. Add or update memory files so future sessions know to consult and
   record to the memory platform without user reminders.
5. Validate with at least one `tools/list` call against `memory-api` and
   `memory-mcp`.

The estate ships **no hand-authored agent hooks**; `setup-workstation.sh`
purges any it finds from a machine set up by the retired `install.sh` /
`install-agents.sh`. The `hindsight-memory` Claude Code plugin ships its own
`UserPromptSubmit`/`Stop` hooks for automatic recall and retain — those are
the plugin's hooks, not estate-authored ones, and survive the purge.

Every Codex project skill or durable instruction must have an
equivalent Claude implementation in the same branch. Treat Codex-only
`.agents`/`.codex` files as incomplete until `.claude`/`CLAUDE.md`/
installer parity exists.

Do not put bearer tokens, secrets, or full transcripts into committed
files.
