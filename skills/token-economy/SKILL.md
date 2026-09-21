---
name: token-economy
description: Use when the user asks to reduce token usage, agent cost, context bloat, prompt-caching misses, or memory/durable-instruction policy. Also use when installing many skills or designing automatic memory recall so retrieval stays bounded.
---

# Token Economy

- Keep stable instructions in `CLAUDE.md` or skills; keep volatile facts in
  `memory-api` (Hindsight) or `memory-mcp` (Basic Memory) and retrieve them
  on demand rather than inlining them.
- Prefer progressive disclosure: list/search first, open small file ranges
  next, fetch full files or notes only when needed.
- Keep recall bounded: search with a small result limit for a quick check,
  and widen only for deliberate task setup.
- Keep runner MCP profiles narrow: `minimal` by default, wider profiles
  only when the task needs those extra tools.
- Do not install or enable low-fit skills just to grow the list. Skill
  metadata itself consumes prompt budget and very large skill sets can
  hide useful skills.
- Preserve prompt-cache-friendly ordering when writing durable
  instructions: stable policy first, dynamic task-specific context later.

When reporting command results, summarize only the lines needed to support
the decision. A session digest (if the user asks for one) should capture
only reusable lessons above a genuine confidence bar and should be checked
against existing memory-api/memory-mcp hits before writing, so it does not
duplicate what is already captured.

## Tunable env vars

| Variable | Effect |
| --- | --- |
| `AGENT_MCP_PROFILE` | Narrows the MCP tool set for one runner (`minimal`, `frontend`, `cluster`, `code-intel`, `full-diagnostic`). |
| `AGENT_RUNTIME_DEFAULT_MCP_PROFILE` | Same, as a fleet-wide default; prefer the per-runner variable unless the change is deliberately fleet-wide. |
| `HINDSIGHT_API_URL` | Overrides the `memory-api` endpoint; falls back to a personal local daemon when unset. |

The `knowledge-api`-era env vars (`KB_RECALL_MIN_SCORE`, `KB_RECALL_MODE`,
`KB_AUTO_MCP_DISABLED`) are retired along with the service they tuned; they
do nothing against `memory-api` or `memory-mcp`.
