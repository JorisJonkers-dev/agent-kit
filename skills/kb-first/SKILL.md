---
name: kb-first
description: Use before designing or changing behavior that may depend on prior memory captures, repo history, architecture decisions, cluster state, agent conventions, or remembered lessons. Also use near task completion to capture durable lessons or decisions without dumping large context.
---

# Memory First

The estate's knowledge-api and its `knowledge.*` MCP tools are retired
(fleet-infra#231). The current memory platform is two separate MCP servers,
each with its own tools and its own purpose — check each one's `tools/list`
for the exact call names and parameters, since they are independent products
with their own interfaces:

- **`memory-api`** (Hindsight) — long-term memory with explicit
  read/write/search tools. Use it for durable lessons: verified behavior,
  pitfalls, decisions, and operational facts you want a future session to
  find without re-deriving them.
- **`memory-mcp`** (Basic Memory) — shared Markdown notes with a semantic
  link graph. Use it for durable, human-readable notes that benefit from
  being edited in place and linked to related notes — never overwrite an
  existing note wholesale; edit it.

Use either as a small retrieval layer, not as a large context dump:

1. Distill the task into a short query: nouns, service names, file names,
   and the decision being made.
2. Search with a tight result limit first. Widen only if the narrow search
   comes back empty or is clearly missing something the task needs.
3. Read only what is needed — a snippet or note summary is usually enough.
   Fetch the full note or record only when a hit actually matters.
4. If neither server has useful context, say so explicitly and continue
   from repo/source inspection instead of guessing.

Capture at the end only when the information is durable and reusable:
implementation pitfalls, verified behavior, operational runbooks,
architecture/process choices, or ambiguity that needed operator judgment.
Keep captures compact. Do not capture secrets, raw logs, full diffs, or
entire transcripts. Prefer `memory-api` for a short lesson entry and
`memory-mcp` for a longer note that should link to related notes.

Never run a broad, unscoped search across everything as a first step. Use it
only after a targeted search fails and the task genuinely needs cross-cutting
context.
