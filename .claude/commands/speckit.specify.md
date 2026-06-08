---
description: Create or update a feature specification from a natural language description.
---

## User Input

```text
$ARGUMENTS
```

You MUST consider the user input before proceeding. If it is empty, stop and ask
for the feature description.

## Bootstrap

If `.specify` is absent, scaffold the minimal Spec Kit runtime before creating
the spec:

- Create `.specify/memory`, `.specify/templates`, `.specify/scripts/bash`, and
  `specs`.
- Add missing templates only when absent, especially
  `.specify/templates/spec-template.md` and
  `.specify/templates/checklist-template.md`.
- Add missing helper scripts only when absent:
  `.specify/scripts/bash/create-new-feature.sh` must create a single
  `specs/<number>-<short-name>/spec.md`, copy the spec template, write
  `.specify/feature.json`, and print JSON with `FEATURE_DIR` and `SPEC_FILE`.
  `.specify/scripts/bash/check-prerequisites.sh` must locate the active feature
  from `.specify/feature.json`.
- Do not overwrite existing Spec Kit scripts or templates.

## Procedure

1. Generate a concise 2-4 word feature short name from the user description.
   Prefer action-noun names such as `add-user-auth` or `analytics-dashboard`.
2. Run `.specify/scripts/bash/create-new-feature.sh --json "$ARGUMENTS"` if the
   script supports it. Otherwise use the minimal script behavior described in
   Bootstrap. Capture `FEATURE_DIR` and `SPEC_FILE`.
3. Load `.specify/templates/spec-template.md` and, if present,
   `.specify/memory/constitution.md`.
4. Write `FEATURE_DIR/spec.md` from the user's description:
   - Focus on what users need and why.
   - Avoid implementation details, tech stacks, APIs, libraries, and code
     structure.
   - Include user scenarios, functional requirements, success criteria,
     assumptions, edge cases, and key entities when relevant.
   - Use at most three `[NEEDS CLARIFICATION: ...]` markers, only for decisions
     that materially affect scope, security/privacy, or user experience and have
     no reasonable default.
5. Create `FEATURE_DIR/checklists/requirements.md` from the checklist template.
   Validate the spec against completeness, testability, measurable success
   criteria, bounded scope, and absence of implementation details. Iterate the
   spec up to three times for fixable failures.
6. If clarification markers remain, present all questions together with options
   and wait for the user's answers before finalizing.
7. Report `FEATURE_DIR`, `SPEC_FILE`, checklist status, and readiness for
   `/speckit.clarify` or `/speckit.plan`.
