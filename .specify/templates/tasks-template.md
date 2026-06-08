# Tasks: {{FEATURE_NAME}}

**Input**: Design documents from `/specs/{{FEATURE_NAME}}/`
**Prerequisites**: plan.md (required), research.md, data-model.md, contracts/

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel with other tasks because it touches different files
- **[Story]**: User story label, for example US1, US2, US3
- Include exact file paths in descriptions

## Phase 1: Setup

- [ ] T001 Create or verify project structure for this feature
- [ ] T002 Identify the smallest validation command for touched area

## Phase 2: Foundational

- [ ] T003 Implement shared models/configuration needed by all stories
- [ ] T004 Add or update base tests for cross-story behavior

## Phase 3: User Story 1 (Priority: P1)

**Goal**: [Brief value delivered by this story]

**Independent Test**: [How to verify only this story]

- [ ] T005 [US1] Implement [specific behavior] in [path]
- [ ] T006 [US1] Add focused tests in [path]

## Phase 4: User Story 2 (Priority: P2)

**Goal**: [Brief value delivered by this story]

**Independent Test**: [How to verify only this story]

- [ ] T007 [P] [US2] Implement [specific behavior] in [path]
- [ ] T008 [P] [US2] Add focused tests in [path]

## Phase 5: User Story 3 (Priority: P3)

**Goal**: [Brief value delivered by this story]

**Independent Test**: [How to verify only this story]

- [ ] T009 [P] [US3] Implement [specific behavior] in [path]
- [ ] T010 [P] [US3] Add focused tests in [path]

## Phase 6: Polish

- [ ] T011 Run the validation command identified in plan.md
- [ ] T012 Update docs or runbooks affected by this feature

## Dependencies

- Setup before foundational work
- Foundational work before user stories
- User stories may proceed in priority order, unless marked independent and parallel
- Polish after desired stories are complete

## Parallel Example

```text
T007 [P] [US2] ...
T009 [P] [US3] ...
```
