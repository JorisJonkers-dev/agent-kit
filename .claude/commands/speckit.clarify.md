---
description: Clarify underspecified feature requirements before technical planning.
---

## User Input

```text
$ARGUMENTS
```

Use the input as clarification focus if provided.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must read
`.specify/feature.json`, print JSON with `FEATURE_DIR` and available docs, and
honor required-file checks. Do not overwrite existing scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json` from the repo root.
   Stop if no active feature or `spec.md` is found; tell the user to run
   `/speckit.specify` first.
2. Load `FEATURE_DIR/spec.md` and `.specify/memory/constitution.md` if present.
3. Identify ambiguities that block planning. Prioritize scope, actor and
   permission boundaries, data lifecycle, compliance, failure behavior, and user
   experience. Ignore implementation choices unless the spec already leaked
   them into requirements.
4. Ask up to five concise questions. Prefer multiple choice when the tradeoff is
   bounded; allow custom answers when needed. Ask only questions whose answers
   would materially change the spec or plan.
5. Update `FEATURE_DIR/spec.md` with a `## Clarifications` section dated today.
   Record each question and answer, then revise affected requirements so the
   body is unambiguous.
6. Report how many clarifications were added and whether the feature is ready
   for `/speckit.plan`.
