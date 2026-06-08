---
description: Generate a focused quality checklist for the active specification.
---

## User Input

```text
$ARGUMENTS
```

Use the input as the checklist domain or focus. If empty, infer the most useful
quality checklist from the spec.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add
`.specify/templates/checklist-template.md` and
`.specify/scripts/bash/check-prerequisites.sh` only when missing. The script
must locate the active feature from `.specify/feature.json` and print JSON with
`FEATURE_DIR`. Do not overwrite existing Spec Kit scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json` from the repo root.
   Stop if no active feature or spec exists.
2. Load `FEATURE_DIR/spec.md`, `FEATURE_DIR/plan.md` if present, and
   `.specify/memory/constitution.md` if present.
3. Create `FEATURE_DIR/checklists/<focus>.md` using
   `.specify/templates/checklist-template.md` when available. Choose a concise
   file name from the focus, such as `security.md`, `accessibility.md`, or
   `requirements.md`.
4. Write checklist items as validation questions for requirements completeness,
   clarity, consistency, and acceptance readiness. Do not turn the checklist
   into implementation tasks.
5. If the user asked to validate, mark items complete only when the current
   artifacts satisfy them; otherwise leave them unchecked for review.
6. Report the checklist path and any high-risk gaps found while generating it.
