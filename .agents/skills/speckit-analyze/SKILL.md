---
name: speckit-analyze
description: Analyze consistency across the active spec, plan, and tasks.
---

# Speckit Analyze

Use this skill for the `$speckit-analyze` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must support
`--json --require-tasks --include-tasks`.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`.
   Stop if `spec.md`, `plan.md`, or `tasks.md` is missing.
2. Read the spec, plan, tasks, constitution if present, and referenced
   supporting docs.
3. Perform read-only analysis. Do not modify files.
4. Report missing requirement coverage, spec-plan conflicts, task scope creep,
   duplicated or vague tasks, ordering problems, and unverifiable acceptance
   criteria.
5. Order findings by severity with file or section references. If no issues are
   found, say so and identify residual risk.
6. Recommend the next Speckit command.
