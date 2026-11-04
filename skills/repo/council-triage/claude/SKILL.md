---
name: council-triage
description: Decide whether work belongs in a normal session, a council split skill, or a full council run. Use when Claude needs to classify scope, risk, decomposition, model mix, or next council action before spending multi-agent budget.
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

## Model Choice

- Rate each task's difficulty and choose its model with the `delegate` skill
  (`skills/workstation/delegate/SKILL.md` in agent-kit). This skill carries no
  rubric of its own.

## Interop Surfaces

- Primary command shape: `node ~/.claude/skills/council/council.mjs triage ...`.
- Route full decomposable work to the base `council` skill or `council-run`.
- Return one decision: normal session, grill, design, run, amend, inject,
  supervise, or full council.
