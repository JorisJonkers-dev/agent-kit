---
name: council-inject
description: Inject new constraints, evidence, files, or operator guidance into an active council flow. Use when Codex must add context to a run without restarting or rewriting unrelated plan artifacts.
---

# Council Inject

Use `inject` to add new information to an active council flow. It should make
context visible to later workers or reviewers without changing completed facts.

## Chunking Rules

- Inject one coherent context packet at a time.
- Label the packet as constraint, evidence, clarification, failure output, or
  operator decision.
- Attach it to affected task ids or plan sections.
- Keep raw logs compact; include paths to full logs when available.

## Lens-Building Rules

Build an injection lens:

- Source and timestamp of the new information.
- Whether it changes scope, validation, risk, or task boundaries.
- Affected and unaffected chunks.
- Required downstream action: continue, amend, re-run, or stop.

## Model Preference Doctrine

- Use cheap models for summarizing straightforward injected evidence.
- Use stronger models when deciding whether new context invalidates a plan.
- Prefer Codex for injected build, test, or code evidence.
- Prefer Claude for injected user decisions, product constraints, or narrative
  synthesis.

## Interop Surfaces

- Primary command shape: `node ~/.codex/skills/council/council.mjs inject ...`.
- Use `council-amend` if injected context changes the plan or DAG.
- Use supervised wrappers only for analysis of a bounded injected packet.
- Return provenance, affected tasks, and the recommended next council action.
