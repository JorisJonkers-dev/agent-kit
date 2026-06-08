---
description: Analyze consistency across the active spec, plan, and tasks.
---

## User Input

```text
$ARGUMENTS
```

Use the input as analysis focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR` plus available
docs. Do not overwrite existing scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
   from the repo root. Stop if `spec.md`, `plan.md`, or `tasks.md` is missing.
2. Load `FEATURE_DIR/spec.md`, `FEATURE_DIR/plan.md`,
   `FEATURE_DIR/tasks.md`, `.specify/memory/constitution.md` if present, and
   referenced supporting docs.
3. Perform a read-only analysis. Do not modify files.
4. Report only actionable findings:
   - Missing task coverage for requirements or success criteria.
   - Plan decisions that conflict with the spec or constitution.
   - Tasks that introduce scope not present in the spec.
   - Duplicated, contradictory, vague, or unordered tasks.
   - Acceptance criteria that cannot be verified by the task list.
5. Order findings by severity and include file/section references. If no issues
   are found, say so and note residual risks.
6. Recommend the next command: refine with `/speckit.specify`,
   `/speckit.plan`, `/speckit.tasks`, or proceed to `/speckit.implement`.
