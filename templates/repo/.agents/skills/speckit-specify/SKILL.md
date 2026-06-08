---
name: speckit-specify
description: Create or update a feature specification from a natural language description.
---

# Speckit Specify

Use this skill for the `$speckit-specify` phase. The user's prompt is the
feature description; if it is empty, ask for the description before proceeding.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add missing
`.specify/templates/spec-template.md`,
`.specify/templates/checklist-template.md`,
`.specify/scripts/bash/create-new-feature.sh`, and
`.specify/scripts/bash/check-prerequisites.sh` only when absent. The feature
script must create one `specs/<number>-<short-name>/spec.md`, copy the spec
template, write `.specify/feature.json`, and print JSON with `FEATURE_DIR` and
`SPEC_FILE`.

## Workflow

1. Generate a concise 2-4 word feature short name.
2. Run `.specify/scripts/bash/create-new-feature.sh --json "<description>"` or
   the minimal equivalent from Bootstrap.
3. Read `.specify/templates/spec-template.md` and
   `.specify/memory/constitution.md` if present.
4. Write `FEATURE_DIR/spec.md` focused on what users need and why. Avoid
   implementation details. Include user scenarios, functional requirements,
   success criteria, assumptions, edge cases, and key entities when relevant.
5. Use at most three `[NEEDS CLARIFICATION: ...]` markers for critical unknowns
   with no reasonable default.
6. Create `FEATURE_DIR/checklists/requirements.md`, validate the spec, and
   iterate up to three times for fixable gaps.
7. Report `FEATURE_DIR`, `SPEC_FILE`, checklist status, and readiness for
   `$speckit-clarify` or `$speckit-plan`.
