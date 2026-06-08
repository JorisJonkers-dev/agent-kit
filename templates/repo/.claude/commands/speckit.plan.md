---
description: Create a technical implementation plan for the active specification.
---

## User Input

```text
$ARGUMENTS
```

Treat the input as the user's technical preferences and constraints.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh`,
`.specify/scripts/bash/setup-plan.sh`, and
`.specify/scripts/bash/update-agent-context.sh` only when absent. The minimal
`setup-plan.sh` must locate the active feature, copy
`.specify/templates/plan-template.md` to `FEATURE_DIR/plan.md` if needed, and
print JSON with `FEATURE_DIR`, `SPEC_FILE`, and `PLAN_FILE`. Do not overwrite
existing Spec Kit scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/setup-plan.sh --json` from the repo root. Stop if
   the active feature or spec is missing; tell the user to run
   `/speckit.specify` first.
2. Load `SPEC_FILE`, `PLAN_FILE`, `.specify/memory/constitution.md`, and any
   existing docs under `FEATURE_DIR`.
3. Fill `FEATURE_DIR/plan.md` with implementation context, constraints,
   constitution compliance, project structure, and phase gates. Use the user's
   input for stack and architecture choices.
4. Produce supporting docs when relevant:
   `FEATURE_DIR/research.md`, `FEATURE_DIR/data-model.md`,
   `FEATURE_DIR/contracts/`, and `FEATURE_DIR/quickstart.md`.
5. Keep the plan consistent with the spec. Do not add feature scope that the
   spec did not request; send scope changes back to `/speckit.specify`.
6. Run `.specify/scripts/bash/update-agent-context.sh` if present so agent
   context reflects the chosen stack.
7. Report created/updated files, unresolved risks, and readiness for
   `/speckit.tasks`.
