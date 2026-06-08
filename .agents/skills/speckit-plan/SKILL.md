---
name: speckit-plan
description: Create a technical implementation plan for the active specification.
---

# Speckit Plan

Use this skill for the `$speckit-plan` phase. Treat the user's prompt as
technical preferences and constraints.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/scripts/bash/check-prerequisites.sh`,
`.specify/scripts/bash/setup-plan.sh`, and
`.specify/scripts/bash/update-agent-context.sh` only when absent. The minimal
`setup-plan.sh` must locate the active feature, copy
`.specify/templates/plan-template.md` to `FEATURE_DIR/plan.md` if needed, and
print JSON with `FEATURE_DIR`, `SPEC_FILE`, and `PLAN_FILE`.

## Workflow

1. Run `.specify/scripts/bash/setup-plan.sh --json`. Stop if no active spec
   exists.
2. Read the spec, plan template or existing plan, constitution, and supporting
   docs in `FEATURE_DIR`.
3. Fill `FEATURE_DIR/plan.md` with implementation context, constraints,
   constitution compliance, project structure, and phase gates.
4. Create supporting docs when relevant: `research.md`, `data-model.md`,
   `contracts/`, and `quickstart.md`.
5. Keep scope aligned to the spec; send product-scope changes back to
   `$speckit-specify`.
6. Run `.specify/scripts/bash/update-agent-context.sh` if present.
7. Report updated files, unresolved risks, and readiness for `$speckit-tasks`.
