---
name: kb-first
description: Use before designing or changing behavior that may depend on prior knowledge-base captures, repo history, architecture decisions, cluster state, agent conventions, or remembered lessons. Also use near task completion to capture durable lessons or decisions without dumping large KB context.
---

# KB First

Use the KB as a small retrieval layer, not as a large context dump.

1. Distill the task into a short recall query: nouns, service names,
   file names, and the decision being made.
2. Call `knowledge.recall` with `limit <= 5`. Prefer
   `scope=project:personal-stack` for repo behavior, `topic:<slug>`
   for general framework/tool facts, or omit scope for the curated
   default.
3. Choose the right mode:
   - `fast` — short/trivial lookups or latency-sensitive (< 80 chars).
   - `hybrid` — normal work; FTS + vector + RRF.
   - `deep` — only after fast or hybrid misses something important.
4. Read only what is needed. Usually snippets are enough. If a hit
   matters, call `knowledge.relations(id, depth=1)` before fetching
   the full note.
5. Filter mentally: hits with scores below 0.01 are rarely useful —
   treat as no match and continue from repo/source inspection.
6. If the KB has no useful context, continue from repo/source
   inspection and say the KB had no relevant hits.

Capture at the end only when the information is durable and reusable:
implementation pitfalls, verified behavior, operational runbooks,
architecture/process choices, or ambiguity that needs operator judgment.
Keep captures compact. Do not capture secrets, raw logs, full diffs, or
entire transcripts.

Never run broad `scope=all` recall as a first step. Use it only after
targeted recall fails and the task genuinely needs cross-scope context.
