---
name: speckit-implement
description: Implement the active Spec Kit task list.
---

# Speckit Implement

Use this skill for the `$speckit-implement` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR`,
`SPEC_FILE`, `PLAN_FILE`, and `TASKS_FILE`.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`.
   Stop if the spec, plan, or tasks file is missing.
2. Read the active spec, plan, tasks, constitution, and supporting docs.
3. Execute incomplete tasks from `tasks.md` in dependency order. Run `[P]` tasks
   in parallel only when they touch independent files and the environment allows
   it.
4. Keep implementation scope limited to the task list and active spec.
5. Mark tasks complete in `tasks.md` after implementation and relevant
   validation.
6. Run the smallest meaningful tests required by the task, plan, or repository
   conventions. Report exact blockers for checks that cannot run.
7. Stop for conflicts, unactionable tasks, or decisions missing from the spec.
8. Finish with completed task ids, changed files, validation results, and
   remaining incomplete tasks.
