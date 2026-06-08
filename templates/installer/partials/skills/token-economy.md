---
name: token-economy
description: Use when the user asks to reduce token usage, agent cost, context bloat, prompt-caching misses, RAG/LightRAG behavior, memory policies, or durable instructions. Also use when installing many skills or designing automatic KB recall so retrieval stays bounded.
---

# Token Economy

- Keep stable instructions in `CLAUDE.md` or skills; keep volatile facts
  in the KB and retrieve them on demand.
- Prefer progressive disclosure: list/search first, open small file
  ranges next, fetch full files or notes only when needed.
- Keep recall bounded: default to `limit=3` for hook-injected context and
  `limit <= 5` for manual task setup.
- Use adaptive recall mode: `fast` for prompts under 80 chars, `hybrid`
  for normal work, `deep` only after a miss or non-obvious cross-topic
  dependency.
- Keep runner MCP profiles narrow: `minimal` by default, wider profiles
  only when the task needs those extra tools.
- Do not install or enable low-fit skills just to grow the list. Skill
  metadata itself consumes prompt budget and very large skill sets can
  hide useful skills.
- Preserve prompt-cache-friendly ordering when writing durable
  instructions: stable policy first, dynamic task-specific context later.

When reporting command results, summarize only the lines needed to
support the decision. Session digests should capture only reusable
lessons above a confidence floor and should dedupe against existing KB
hits before writing.

## Tunable env vars

| Variable | Default | Effect |
| --- | --- | --- |
| `KB_RECALL_MIN_SCORE` | `0.004` | Minimum hit score injected into context; raise to tighten relevance. |
| `KB_RECALL_HOOK_LIMIT` | `3` | Max recall hits per hook invocation. |
| `KB_RECALL_HOOK_MODE` | auto | Override adaptive mode (`fast`/`hybrid`/`deep`). |
| `KB_DIGEST_MAX_CHARS` | `30000` | Transcript chars fed to the stop-digest hook; lower = cheaper. |
| `KB_DIGEST_MAX_CAPTURES` | `4` | Per-session capture cap for the stop hook. |
| `KB_AUTO_MCP_DISABLED` | `0` | Set to `1` to disable all automatic KB calls (panic switch). |
