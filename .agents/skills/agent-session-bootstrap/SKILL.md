---
name: agent-session-bootstrap
description: Use when configuring Claude Code or Codex sessions, hooks, skills, MCP servers, durable instructions, agent runners, or future-session defaults. Ensures KB recall/capture and token-efficient behavior are installed without relying on user reminders.
---

# Agent Session Bootstrap

## Checklist

1. Locate the active user and project config layers:
   - Claude: `~/.claude/settings.json`, `~/.claude/CLAUDE.md`, project
     `CLAUDE.md`, project `.claude/settings.json`, project `.claude/hooks`,
     and `~/.claude/skills`.
   - Codex: `~/.codex/config.toml`, `~/.codex/hooks.json`, repo `AGENTS.md`,
     and `.agents/skills`.
2. Ensure the `knowledge` MCP server is configured and uses
   `KB_BEARER_TOKEN` rather than an inline secret where possible.
3. Keep runner MCP profiles narrow:
   - Default to `minimal` for routine work; it should include `knowledge` and
     repo-scoped `github` only.
   - Use `frontend`, `cluster`, `code-intel`, or `full-diagnostic` only when
     the task needs those tools.
   - Prefer `AGENT_MCP_PROFILE` for one runner and
     `AGENT_RUNTIME_DEFAULT_MCP_PROFILE` only when changing the fleet default.
4. Register bounded recall hooks:
   - `UserPromptSubmit`: short prompt recall, `limit=3`, `mode=hybrid`.
   - `PreToolUse` for edits: path/module recall, deduped per session.
   - `Stop`: transcript digest with a per-session capture cap.
5. Keep hooks silent on KB failure and add `KB_AUTO_MCP_DISABLED=1` as a panic
   switch.
6. Add or update global memory files so future sessions know to consult and
   update the KB without user reminders.
7. Validate with dry-run hook payloads and at least one `tools/list` or
   `knowledge.recall` MCP call.

Every Codex project skill, hook, or durable instruction must have an equivalent
Claude implementation in the same branch. Treat Codex-only `.agents`/`.codex`
files as incomplete until `.claude`/`CLAUDE.md`/installer parity exists.

Do not put bearer tokens, secrets, or full transcripts into committed files.
