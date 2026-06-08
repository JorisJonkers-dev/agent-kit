---
name: speckit-checklist
description: Generate a focused quality checklist for the active specification.
---

# Speckit Checklist

Use this skill for the `$speckit-checklist` phase. Treat the user's prompt as
the checklist focus; infer a useful focus if none is provided.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add
`.specify/templates/checklist-template.md` and
`.specify/scripts/bash/check-prerequisites.sh` only when missing.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json`. Stop if no active
   feature or spec exists.
2. Read the active spec, plan if present, and constitution if present.
3. Create `FEATURE_DIR/checklists/<focus>.md` from the checklist template when
   available.
4. Write validation questions for requirements completeness, clarity,
   consistency, and acceptance readiness. Do not write implementation tasks.
5. Mark items complete only when the user asked for validation and the current
   artifacts satisfy the item.
6. Report the checklist path and high-risk gaps.
