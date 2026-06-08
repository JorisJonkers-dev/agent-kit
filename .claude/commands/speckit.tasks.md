---
description: Generate an actionable task list from the active implementation plan.
---

## User Input

```text
$ARGUMENTS
```

Use the input as task-generation focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh` and
`.specify/scripts/bash/setup-tasks.sh` only when absent. The minimal
`setup-tasks.sh` must require `spec.md` and `plan.md`, create
`FEATURE_DIR/tasks.md` from `.specify/templates/tasks-template.md` if needed,
and print JSON with `FEATURE_DIR`, `PLAN_FILE`, and `TASKS_FILE`. Do not
overwrite existing Spec Kit scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/setup-tasks.sh --json` from the repo root. Stop if
   `spec.md` or `plan.md` is missing; tell the user which Speckit command to run
   first.
2. Load `FEATURE_DIR/spec.md`, `FEATURE_DIR/plan.md`, and available supporting
   docs (`research.md`, `data-model.md`, `contracts/`, `quickstart.md`).
3. Generate `FEATURE_DIR/tasks.md` as executable Markdown:
   - Number tasks `T001`, `T002`, and so on.
   - Group by setup, foundations, user story phases, polish, and validation.
   - Mark independent tasks with `[P]`.
   - Include exact file paths where practical.
   - If tests are required by the spec, constitution, or plan, put test tasks
     before implementation tasks for the same behavior.
   - Keep each user story independently implementable and testable.
4. Add dependency notes and parallel execution examples when they help execute
   the plan.
5. Report task count, parallelizable count, and readiness for
   `/speckit.analyze`, `/speckit.taskstoissues`, or `/speckit.implement`.
