---
description: Implement the active Spec Kit task list.
---

## User Input

```text
$ARGUMENTS
```

Use the input as implementation focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR`,
`SPEC_FILE`, `PLAN_FILE`, and `TASKS_FILE`. Do not overwrite existing Spec Kit
scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
   from the repo root. Stop if `spec.md`, `plan.md`, or `tasks.md` is missing.
2. Load the active spec, plan, tasks, constitution, and supporting docs. Treat
   `tasks.md` as the execution source of truth.
3. Parse incomplete tasks in order. Respect dependencies and phase boundaries.
   `[P]` tasks may be run in parallel only when they touch independent files and
   the environment allows it.
4. Implement one coherent task or small dependency group at a time. Keep scope
   limited to the task list and do not add new product behavior absent from the
   spec.
5. Mark each task complete in `tasks.md` after its implementation and relevant
   validation pass.
6. Run the smallest meaningful tests described by the plan, task, or repository
   conventions. If a test cannot run, report the exact blocker.
7. Stop and report if the plan/spec/tasks conflict, a task is not actionable, or
   implementation would require a decision not captured in the spec.
8. Finish with completed task ids, files changed, validation results, and any
   remaining incomplete tasks.
