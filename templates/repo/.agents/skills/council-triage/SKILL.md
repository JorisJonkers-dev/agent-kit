---
name: council-triage
description: Decide whether work belongs in a normal session, a council split skill, or a full council run. Use when Codex needs to classify scope, risk, decomposition, model mix, or next council action before spending multi-agent budget.
---

# Council Triage

Use `triage` before launching expensive or parallel work. The result is a routing
decision, not an implementation plan.

## Chunking Rules

- Separate the request into independently decidable work items.
- Mark each item as sequential, parallel, blocked, research-only, or validation.
- Reject council fan-out for tiny changes or tightly-coupled steps.
- Preserve user boundaries exactly; do not create chunks outside allowed scope.

## Lens-Building Rules

Build a triage lens with:

- Goal and definition of done.
- Known constraints and forbidden areas.
- Coupling map: which steps depend on prior outputs.
- Risk map: irreversible operations, production impact, or high uncertainty.
- Budget signal: why the extra model spend is or is not justified.

## Model Preference Doctrine

- Use the host session for obvious small work.
- Use a cheap model for first-pass classification when facts are clear.
- Escalate to a stronger model only when routing depends on subtle architecture,
  safety, or product tradeoffs.
- Prefer mixed-model review when the main value is independent disagreement.

## Interop Surfaces

- Primary command shape: `node ~/.codex/skills/council/council.mjs triage ...`.
- Route independent Claude work through `claude-worker`.
- Route full decomposable work to the base `council` skill or `council-run`.
- Return one decision: normal session, grill, design, run, amend, inject,
  supervise, or full council.
