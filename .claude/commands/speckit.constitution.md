---
description: Create or update the Spec Kit project constitution.
---

## User Input

```text
$ARGUMENTS
```

You MUST consider the user input before proceeding.

## Bootstrap

If `.specify` is absent, scaffold the minimal Spec Kit runtime before changing
the constitution:

- Create `.specify/memory`, `.specify/templates`, `.specify/scripts/bash`, and
  `specs`.
- Add missing template files only when absent:
  `.specify/templates/constitution-template.md`,
  `.specify/templates/spec-template.md`,
  `.specify/templates/plan-template.md`,
  `.specify/templates/tasks-template.md`, and
  `.specify/templates/checklist-template.md`.
- Add missing helper scripts only when absent:
  `.specify/scripts/bash/check-prerequisites.sh`,
  `.specify/scripts/bash/create-new-feature.sh`,
  `.specify/scripts/bash/setup-plan.sh`,
  `.specify/scripts/bash/setup-tasks.sh`, and
  `.specify/scripts/bash/update-agent-context.sh`.
- The minimal scripts must support the core paths used by these commands:
  active feature lookup from `.specify/feature.json`, JSON output, and required
  file checks. Do not overwrite existing Spec Kit scripts or templates.

## Procedure

1. Load `.specify/templates/constitution-template.md` if present and load the
   current `.specify/memory/constitution.md` if it already exists.
2. Extract governing principles, testing expectations, delivery workflow, and
   review rules from the user input. If updating an existing constitution,
   preserve valid principles unless the user explicitly replaces them.
3. Write `.specify/memory/constitution.md` with concrete principles, rationale,
   governance, amendment rules, and version/date metadata. Do not leave template
   placeholders unresolved.
4. Review `.specify/templates/spec-template.md`,
   `.specify/templates/plan-template.md`, and
   `.specify/templates/tasks-template.md` for obvious constitution references.
   Note any template updates that should be made by a separate command if they
   are outside this command's requested scope.
5. Report the constitution path, whether it was created or updated, and the
   next recommended phase (`/speckit.specify`).
