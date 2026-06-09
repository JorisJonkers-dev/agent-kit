# Tasks: Agent Runner Manifest Boundary

## Phase 1: Setup

- [x] T001 [FR-001] Read the round-3 assignment and reference analysis files.
- [x] T002 [FR-001] Inspect existing Spec Kit structure and validation
  conventions in this repo.

## Phase 2: Boundary Design

- [x] T003 [FR-002, FR-003] Define ownership boundaries for runtime packaging
  versus deployment blueprint responsibilities in `spec.md`.
- [x] T004 [FR-006, FR-007, FR-008] Record design-first constraints and
  forbidden extraction values in `spec.md` and `plan.md`.

## Phase 3: Skeleton Contracts

- [x] T005 [FR-002, FR-004, FR-009] Add
  `runner-manifests/runtime-package.schema.json`.
- [x] T006 [FR-002, FR-003] Add `runner-manifests/README.md` and
  `runner-manifests/platform-blueprints-handoff.md`.
- [x] T007 [FR-005, FR-006, FR-007, FR-008] Add placeholder-only fixtures under
  `runner-manifests/fixtures/`.

## Phase 4: Validation

- [x] T008 [FR-010, FR-011] Extend `scripts/validate_manifest.py` to parse the
  skeleton and reject concrete deployment values or deployable Kubernetes
  resource kinds.
- [x] T009 [FR-010, FR-011] Run local Python validation, render checks, compile,
  and lint where available.

## Dependencies

- T001 and T002 before T003.
- T003 and T004 before skeleton files.
- T005 through T007 before validator extension.
- T008 before final validation.

## Notes

This task list intentionally excludes production manifest generation,
Kustomize/Helm output, kubeconform, and any networked build or deployment
command.
