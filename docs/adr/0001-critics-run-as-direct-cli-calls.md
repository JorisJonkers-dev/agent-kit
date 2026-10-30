# Critics run as direct CLI calls, not through PAL's clink

The implementation brief routed every critic through PAL MCP's `clink` tool. We
start the critic CLIs (`vibe -p`, `agy -p`) as subprocesses of the critique
runner instead.

PAL's clink rejects any CLI name outside its hard-coded defaults (gemini, codex,
claude) and keeps its parsers in a fixed dict, so adding Vibe or Antigravity
means maintaining a fork. Upstream has had no push since 2025-12-15 while issues,
including a security report, go unanswered, and Gemini CLI, one of those three
defaults, is no longer supported. clink also folds conversation history into the
prompt, which works against blind critics. Both CLIs already offer what clink
would add: headless runs, JSON output, a way to stay read-only, time limits, and
in Vibe a price cap. Vibe keeps a critic read-only with its list of allowed
tools. agy has a plan mode, but it has no effect once slash commands are
disabled, which a critic needs; what keeps an agy critic read-only is that
headless mode denies every action that needs permission. Calling the CLIs
directly also keeps MCP tool definitions out of the orchestrator's context
window.

## Consequences

- No `pal/` tree, no MCP registry entry, no `DISABLED_TOOLS`.
- A critic client is agent-kit configuration (binary, arguments, model, family),
  and the runner owns timeouts, parsing and the read-only tripwire.
