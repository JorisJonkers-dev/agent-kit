---
name: council-grill
description: Stress-test a plan, patch, task DAG, or decision through a council-style adversarial review. Use when Claude needs a focused critique, risk scan, missing-evidence pass, or cross-model challenge before implementation or merge.
---

# Council Grill

Use `grill` to attack a proposal before spending implementation effort. The
output should be actionable: concrete defects, missing evidence, risky
assumptions, and the smallest changes that would make the proposal acceptable.

## Chunking Rules

- Grill one artifact class at a time: brief, plan, task DAG, patch, or report.
- For large inputs, split by decision area or task node, not by arbitrary token
  size.
- Keep each chunk independently reviewable with its own objective, source files,
  constraints, and acceptance criteria.
- Recombine by severity and dependency so duplicate findings collapse into one
  fix request.

## Lens-Building Rules

Build 2-4 explicit review lenses before invoking the grill:

- Correctness: whether the plan actually satisfies the brief.
- Boundary: whether file, ownership, or user constraints are violated.
- Evidence: whether claims are grounded in files, tests, or command output.
- Risk: security, data loss, migration, rollout, or irreversible decisions.

Prefer precise lenses over generic "review this" prompts. Include what would
change your mind.

## Model Preference Doctrine

- Use a cheap model for initial issue clustering when the artifact is simple.
- Prefer a stronger reasoning model for final severity calls or architecture
  critique.
- Prefer Codex at high or xhigh effort for code-heavy diffs and test logic.
- Prefer Claude at sonnet or opus tier for product, architecture, and narrative
  coherence.

## Interop Surfaces

- Primary command shape: `node ~/.claude/skills/council/council.mjs grill ...`.
- Use `codex-med` for routine code critique and `codex-xhigh` for difficult
  code or correctness review.
- Hand off only the bounded chunk plus its lens; do not ask a delegate to infer
  hidden context.
- Return findings ordered by severity with file or artifact references.
