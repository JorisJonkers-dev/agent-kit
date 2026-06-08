---
name: kb-first
description: Use before designing or changing behavior that may depend on prior knowledge-base captures, repo history, architecture decisions, cluster state, agent conventions, or remembered lessons. Also use near task completion to capture durable lessons or decisions without dumping large KB context.
---

# KB First

Use the KB as a small retrieval layer, not as a large context dump.

1. Distill the task into a short query.
2. Call `knowledge.recall` with `limit <= 5` and a scope when possible.
   - `mode=fast` for short/trivial lookups (< 80 chars or latency-sensitive).
   - `mode=hybrid` for normal work.
   - `mode=deep` only after a miss or when cross-topic context matters.
3. Prefer `scope=project:<repo>` or `topic:<slug>` over broad recall.
4. Read snippets first, relations next, and full notes only when needed.
5. Filter mentally: hits with very low scores (< 0.01) are rarely actionable —
   skip them and say the KB had no relevant hits.
6. Capture durable lessons or decisions at the end. Never capture secrets,
   raw logs, full diffs, or full transcripts.
