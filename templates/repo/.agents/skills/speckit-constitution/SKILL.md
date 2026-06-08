---
name: speckit-constitution
description: Create or update the Spec Kit project constitution.
---

# Speckit Constitution

Use this skill for the `$speckit-constitution` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing templates and minimal helper
scripts under `.specify/scripts/bash/` only when absent:
`check-prerequisites.sh`, `create-new-feature.sh`, `setup-plan.sh`,
`setup-tasks.sh`, and `update-agent-context.sh`. The helpers must support
active feature lookup from `.specify/feature.json`, JSON output, and required
file checks. Do not overwrite existing Spec Kit files.

## Workflow

1. Read `.specify/templates/constitution-template.md` and any existing
   `.specify/memory/constitution.md`.
2. Derive concrete project principles, testing standards, delivery workflow,
   review rules, governance, and amendment rules from the user request.
3. Write `.specify/memory/constitution.md` with no unresolved placeholders.
4. Check the spec, plan, and tasks templates for obvious constitution alignment
   notes, but do not make unrelated edits.
5. Report the constitution path and readiness for `$speckit-specify`.
