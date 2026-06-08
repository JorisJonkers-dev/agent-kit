---
name: speckit-clarify
description: Clarify underspecified feature requirements before technical planning.
---

# Speckit Clarify

Use this skill for the `$speckit-clarify` phase.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must read
`.specify/feature.json`, print JSON with `FEATURE_DIR`, and honor required-file
checks.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json`. Stop if no active
   feature or `spec.md` exists.
2. Read `FEATURE_DIR/spec.md` and the constitution if present.
3. Identify planning-blocking ambiguities in scope, actors, permissions, data,
   compliance, failure behavior, and user experience.
4. Ask up to five concise questions, using multiple choice where possible.
5. Add a dated `## Clarifications` section to the spec, record answers, and
   revise affected requirements.
6. Report clarification count and readiness for `$speckit-plan`.
