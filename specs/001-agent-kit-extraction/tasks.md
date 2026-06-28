# Tasks: Agent Kit Extraction

## Phase 1: Source Import

- [x] T001 [FR-3, FR-5, FR-10, FR-13] Copy the reference kit renderer,
  manifest, installer templates, docs, Spec Kit lock/notice files, and council
  bundle into the self-contained repo layout.
- [x] T002 [FR-18, FR-19] Copy the project constitution seed source into
  `.specify/memory/constitution.md` for installer project-scope seeding.
- [x] T003 [FR-1, FR-2, FR-23, FR-24] Add release metadata files for
  repository-tag versioning with short `github:JorisJonkers-dev/agent-kit`
  coordinates.

Dependencies: T001 before renderer adaptation; T002 before installer generation;
T003 independent of T001/T002.

## Phase 2: Renderer Adaptation

- [x] T004 [FR-5, SC-4] Adapt `render-agent-kit.py` root constants from
  personal-stack paths to the agent-kit repo layout while preserving CLI modes.
- [x] T005 [FR-6, FR-7, FR-8, SC-1, SC-5, SC-6] Ensure `--check`, `--write`,
  and `--output` compare/write the generated repo surfaces and
  `installer/install.sh` deterministically.
- [x] T006 [FR-9, SC-7] Replace personal-stack-only doctor checks with
  self-contained render, manifest, parity, installer, and optional KB checks.
- [x] T007 [FR-13, FR-21, FR-22] Verify installer expansion still embeds the
  council bundle, supports user/project scope, preview, and uninstall.

Dependencies: T004 before T005/T006/T007; T001/T002 before all Phase 2 tasks.

## Phase 3: Manifest And Validation

- [x] T008 [FR-1, FR-2, FR-10, FR-11] Adapt `manifest.yaml` paths, artifact
  metadata, managed paths, and checksums to the new repo layout.
- [x] T009 [FR-10, FR-11, FR-12, FR-14, FR-20, SC-8, SC-9, SC-13, SC-15,
  SC-16] Add `scripts/validate_manifest.py` to validate manifest shape,
  managed coverage, checksums, parity gaps, installer placeholders, and
  secret-like markers.
- [x] T010 [FR-10, FR-13, FR-14, SC-8, SC-9] Run the renderer in write mode and
  update generated `.claude`, `.codex`, `.agents`, `.specify`, and installer
  files from templates.

Dependencies: T008 after T004/T005; T009 after T008; T010 after T004/T008.

## Phase 4: CI And Release

- [x] T011 [FR-15, FR-16, SC-2, SC-3, SC-7] Add `.github/workflows/ci.yml`
  with real lint, render/doctor, manifest validation jobs and terminal
  `Pipeline Complete`.
- [x] T012 [FR-1, FR-23] Add release-please workflow and config files for tag
  releases without package publishing.
- [x] T013 [FR-1, FR-3, FR-5, FR-21, FR-22] Update README and portability docs
  for the new repo layout and verification commands.

Dependencies: T011 after T009/T010; T012 independent; T013 after T004/T010.

## Phase 5: Verification And Landing

- [x] T014 [SC-1, SC-4, SC-5, SC-6, SC-7] Run local renderer check, doctor, and
  output rendering smoke checks.
- [x] T015 [SC-8, SC-9, SC-13, SC-15, SC-16] Run manifest validation, Python
  compilation, shell syntax checks, and Python lint.
- [ ] T016 [FR-15, FR-16] Commit, push `impl/initial`, open a PR, poll
  `Pipeline Complete`, fix at most one CI failure round, and squash-merge when
  green.

Dependencies: T014/T015 after implementation tasks; T016 after local
verification.
