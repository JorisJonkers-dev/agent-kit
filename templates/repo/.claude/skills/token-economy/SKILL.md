---
name: token-economy
description: Use when reducing token usage, agent cost, context bloat, prompt-caching misses, RAG/LightRAG behavior, memory policies, or durable instructions. Also use when designing automatic KB recall so retrieval stays bounded.
---

# Token Economy

- Stable instructions belong in `AGENTS.md`, `CLAUDE.md`, or skills; volatile
  facts belong in the KB.
- Keep hook recall to `limit=3`; keep manual setup recall to `limit <= 5`.
- Use adaptive recall mode: `fast` for prompts under 80 chars, `hybrid` for
  longer prompts, `deep` only after a miss or ambiguity.
- Keep runner MCP profiles narrow: `minimal` by default, wider profiles only
  when the task needs those extra tools.
- Preserve prompt-cache-friendly ordering: stable policy first, dynamic task
  context later.
- Summarize command output and avoid broad KB/context dumps.

## Tunable env vars

| Variable | Default | Effect |
| --- | --- | --- |
| `KB_RECALL_MIN_SCORE` | `0.004` | Minimum hit score injected into context; raise to tighten relevance. |
| `KB_RECALL_HOOK_LIMIT` | `3` | Max recall hits per hook invocation. |
| `KB_RECALL_HOOK_MODE` | auto | Override adaptive mode (`fast`/`hybrid`/`deep`). |
| `KB_DIGEST_MAX_CHARS` | `30000` | Transcript chars fed to the stop-digest hook; lower = cheaper. |
| `KB_DIGEST_MAX_CAPTURES` | `4` | Per-session capture cap for the stop hook. |
| `KB_AUTO_MCP_DISABLED` | `0` | Set to `1` to disable all automatic KB calls (panic switch). |
