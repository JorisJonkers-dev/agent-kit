# Implementation Plan: {{FEATURE_NAME}}

**Branch**: `{{FEATURE_NAME}}` | **Date**: {{DATE}} | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/{{FEATURE_NAME}}/spec.md`

## Summary

[Extract from feature spec: primary requirement + technical approach]

## Technical Context

**Language/Version**: [e.g. Kotlin 2.x, TypeScript 5.x or NEEDS CLARIFICATION]
**Primary Dependencies**: [e.g. Spring Boot, Vue, Postgres or NEEDS CLARIFICATION]
**Storage**: [if applicable, e.g. PostgreSQL, Redis, files or N/A]
**Testing**: [e.g. Gradle unit tests, Vitest, Playwright or NEEDS CLARIFICATION]
**Target Platform**: [e.g. k3s, browser, JVM service or NEEDS CLARIFICATION]
**Project Type**: [service/ui/platform/mixed]
**Performance Goals**: [domain-specific target or N/A]
**Constraints**: [domain-specific constraints or N/A]
**Scale/Scope**: [domain-specific scale or N/A]

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [ ] No attribution is introduced in files, comments, commit text, or PR text
- [ ] Claude/Codex parity is preserved for any agent-facing behavior
- [ ] Rendered artifacts are updated by the owning renderer when source changes require it
- [ ] Small stacked PR boundary is clear and unrelated cleanup is excluded
- [ ] Verification command is identified for each touched area

## Project Structure

### Documentation

```text
specs/{{FEATURE_NAME}}/
|-- plan.md
|-- research.md
|-- data-model.md
|-- quickstart.md
|-- contracts/
`-- tasks.md
```

### Source Code

```text
# Fill with the actual paths this feature will touch.
```

**Structure Decision**: [Document the chosen source layout and real paths]

## Phase 0: Outline & Research

1. Extract unknowns from Technical Context into research tasks.
2. Capture existing repo patterns for touched paths.
3. Resolve all NEEDS CLARIFICATION items before design.

**Output**: `research.md`

## Phase 1: Design & Contracts

1. Derive entities from the feature spec and document them in `data-model.md`.
2. Produce or update API/CLI/config contracts in `contracts/`.
3. Write `quickstart.md` with validation steps for the feature.
4. Re-run Constitution Check.

**Output**: `data-model.md`, `contracts/*`, `quickstart.md`

## Phase 2: Task Planning Approach

Describe how `/speckit.tasks` should convert this plan into ordered, independently executable tasks. Do not create `tasks.md` manually during `/speckit.plan`.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
| --- | --- | --- |
| [Only if a constitution gate is intentionally violated] | [reason] | [why simpler option does not work] |

## Progress Tracking

**Phase Status**:

- [ ] Phase 0: Research complete
- [ ] Phase 1: Design complete
- [ ] Phase 2: Task planning approach complete

**Gate Status**:

- [ ] Initial Constitution Check: PASS
- [ ] Post-Design Constitution Check: PASS
- [ ] All NEEDS CLARIFICATION resolved
