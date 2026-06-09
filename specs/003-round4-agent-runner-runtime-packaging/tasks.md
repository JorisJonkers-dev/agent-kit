# Tasks: Agent Runner Runtime Packaging

## Phase 1: Setup

- [x] T001 [FR-001] Read Round 3 boundary spec and skeleton files.
- [x] T002 [FR-002, FR-003] Inspect renderer, manifest, CI, and validation
  conventions.
- [x] T003 [FR-004, FR-010] Review read-only reference behavior without
  copying deployment manifests or concrete values.

## Phase 2: Runtime Templates

- [x] T004 [FR-002] Add `templates/runner-runtime/` as a renderer template
  root.
- [x] T005 [FR-004] Add portable Containerfile and build env example.
- [x] T006 [FR-005, FR-006] Add portable entrypoint with local self-tests,
  repository semantics, trust setup, and Spec Kit seeding.
- [x] T007 [FR-007] Add Git token helper, gh wrapper, git credential helper,
  and Git MCP wrapper.
- [x] T008 [FR-008] Add Claude JSON and Codex TOML MCP profile placeholders.
- [x] T009 [FR-002] Render runtime templates into `runner-manifests/runtime/`.

## Phase 3: Manifest And Validation

- [x] T010 [FR-003] Add runtime template root, managed paths, and checksums to
  `manifest.yaml`.
- [x] T011 [FR-009] Extend manifest validation for runtime package YAML,
  profile parsing, helper executability, shell syntax, and offline self-tests.
- [x] T012 [FR-004, FR-010] Preserve skeleton guardrails for concrete values and
  deployable Kubernetes resource kinds.
- [x] T018 [FR-011] Gate git credential helper token probing behind
  `--runtime-selftest` or `AGENT_RUNTIME_SELFTEST=1` and cover it with a mocked
  helper unit test.

## Phase 4: Verification

- [x] T013 [SC-001] Run `python3 render-agent-kit.py --check`.
- [x] T014 [SC-002] Run `python3 render-agent-kit.py --doctor`.
- [x] T015 [SC-003] Run `python3 scripts/validate_manifest.py`.
- [x] T016 [SC-004] Run `python3 -m compileall render-agent-kit.py scripts council`.
- [x] T017 [SC-005] Run `ruff check .` if available locally.
- [x] T019 [SC-006] Run `python3 scripts/validate_manifest.py --runtime-selftest`.

## Dependencies

- T001-T003 before runtime templates.
- T004 before rendering runtime artifacts.
- T005-T009 before manifest checksums.
- T010 before validator checksum enforcement.
- T011-T012 before final verification.

## Notes

This task list intentionally excludes image builds, Kubernetes rendering,
Flux/Kustomize validation, secret seeding, and any networked command.
