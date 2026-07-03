---
name: council-amend
description: Amend an existing council brief, design, task DAG, or run after user feedback, failed validation, changed constraints, or worker findings. Use when Codex must revise the plan without discarding useful completed work.
---

# Council Amend

Use `amend` when the current council artifacts need a controlled revision. Keep
completed valid work intact and change only what the new evidence requires.

## Chunking Rules

- Separate feedback into scope changes, bug fixes, validation gaps, and blocked
  tasks.
- Amend the smallest affected chunk, preserving stable task ids when semantics
  are unchanged.
- Add new tasks only when they can be independently bounded and validated.
- Mark obsolete tasks explicitly instead of silently reusing them for different
  work.

## Lens-Building Rules

Build an amendment lens:

- Original approved intent.
- New evidence or feedback.
- Affected tasks and downstream dependencies.
- What remains valid and should not be reworked.
- Validation needed to prove the amendment is enough.

## Model Preference Doctrine

- Use a cheap model to classify simple feedback.
- Use stronger reasoning when changing dependencies, scope, or integration
  strategy.
- Prefer Codex high or xhigh for amendments driven by code/test failures.
- Prefer Claude sonnet or opus for amendments driven by product or planning
  ambiguity.

## Interop Surfaces

- Primary command shape: `node ~/.codex/skills/council/council.mjs amend ...`.
- Use `council-grill` when the amendment could invalidate major assumptions.
- Return the diff in plan/task intent, affected task ids, and the next action:
  resume run, re-design, inject context, or ask the user.
