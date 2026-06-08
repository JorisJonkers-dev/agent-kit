---
name: speckit-tasks
description: Generate an actionable task list from the active implementation plan.
---

# Speckit Tasks

Use this skill for the `$speckit-tasks` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh` and
`.specify/scripts/bash/setup-tasks.sh` only when absent. The minimal
`setup-tasks.sh` must require `spec.md` and `plan.md`, create
`FEATURE_DIR/tasks.md` from `.specify/templates/tasks-template.md` if needed,
and print JSON with `FEATURE_DIR`, `PLAN_FILE`, and `TASKS_FILE`.

## Workflow

1. Run `.specify/scripts/bash/setup-tasks.sh --json`. Stop if the spec or plan
   is missing.
2. Read the spec, plan, and supporting docs.
3. Generate `FEATURE_DIR/tasks.md` with `T001` style task ids, user-story
   phases, dependency notes, and `[P]` markers for independent parallel tasks.
4. Include exact file paths where practical. Put test tasks before
   implementation tasks whenever tests are required by the spec, constitution,
   or plan.
5. Keep each user story independently implementable and testable.
6. Report task count, parallelizable count, and readiness for
   `$speckit-analyze`, `$speckit-taskstoissues`, or `$speckit-implement`.
