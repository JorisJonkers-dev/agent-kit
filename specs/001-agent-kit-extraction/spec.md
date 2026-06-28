# Agent Kit Extraction

## Overview

JorisJonkers-dev/agent-kit is the versioned source for a pinned renderer and installer kit used by downstream repositories. The kit supplies agent-facing surfaces: skills, hooks, council orchestration assets, KB hooks, settings, and Spec Kit seeds. The purpose is to replace copied or repo-local surface ownership with a reproducible artifact that downstream repositories can pin, render, verify, and update through normal dependency management.

The extraction preserves the user-facing contracts currently represented by the reference renderer, manifest, templates, council assets, and installer route. The compatibility baseline is the read-only reference set under `/workspace/personal-stack/platform/agents/kit`, `/workspace/personal-stack/platform/agents/council`, and `services/knowledge-api` installer controller files. Downstream repositories must be able to render `.claude`, `.codex`, and `.agents` content from an exact kit version, detect drift in CI, and receive installer content served by the running KB service without embedding secrets.

Distribution intent: this repository is consumed by personal-stack and optionally website as a versioned artifact with short coordinates. Coordinates must not produce doubled plugin-marker names. Consumers pin versions through Renovate-managed dependency updates. personal-stack remains continuously auto-deployed and is not treated as a versioned release artifact.

## User Scenarios

1. A consumer maintainer pins JorisJonkers-dev/agent-kit to a released version, renders repo-local `.claude`, `.codex`, and `.agents` surfaces, and commits the generated result with provenance that identifies the kit version.
2. A consumer CI run compares committed surfaces with the pinned kit render and fails when any generated skill, hook, setting, council asset, KB hook, or installer entrypoint differs.
3. A kit maintainer changes a managed surface, updates the manifest, and verifies that all supported agent surfaces stay in parity or that any intentional gap has an explicit unsupported reason.
4. A runtime operator downloads `/install.sh` from the KB service and receives a shell script whose version and KB URL match the running service while containing no bearer token or other secret.
5. A downstream repository receives a Renovate update for a newer kit version and can review the generated surface diff without changing personal-stack release semantics.

## Functional Requirements (FR-n)

- FR-1: The kit must define a versioned artifact identity for JorisJonkers-dev/agent-kit that downstream repositories can pin with short coordinates.
- FR-2: Artifact coordinates must avoid doubled plugin-marker names in consumer metadata, generated paths, and dependency update labels.
- FR-3: Downstream repositories must render `.claude`, `.codex`, and `.agents` surfaces from a declared pinned kit version rather than from an unpinned local copy.
- FR-4: Rendered surfaces must include or reference enough provenance for a reviewer and CI to identify the exact kit version that produced them.
- FR-5: The renderer contract must preserve the public modes `--check`, `--write`, `--output`, and `--doctor`.
- FR-6: `--check` must report drift or missing managed files without modifying the consumer repository.
- FR-7: `--write` must update managed consumer surfaces to match the pinned kit version.
- FR-8: `--output` must render the pinned kit into a caller-selected directory without requiring changes to the consumer repository.
- FR-9: `--doctor` must remain read-only and report static render drift, manifest version state, parity state, and KB reachability state when configured.
- FR-10: The kit manifest must pin every managed skill, hook, hook setting, council asset, KB hook, Spec Kit seed, and installer entrypoint that belongs to the kit contract.
- FR-11: The manifest must record checksums or equivalent stable integrity signals for managed files so CI can detect unreviewed drift.
- FR-12: Shared capabilities must maintain parity across all supported agent surfaces unless the manifest records an explicit unsupported reason for a specific gap.
- FR-13: Council orchestration assets must be represented as a first-class managed bundle covering the driver, prompts, schemas, default configuration, and user-facing skill entrypoints.
- FR-14: KB hooks and KB-oriented skills must be represented as managed kit surfaces and must be available to every supported agent surface unless explicitly unsupported.
- FR-15: Consumer CI must fail with a non-zero status when committed generated surfaces do not match the pinned kit version.
- FR-16: Consumer CI must pass when committed generated surfaces, manifest entries, and the pinned kit version agree.
- FR-17: The KB service installer route must preserve the `/install.sh` path and return shell-script content suitable for direct download or shell piping.
- FR-18: The installer serving contract must load the script as a `ClassPathResource` named `installer/install.sh`.
- FR-19: The installer serving contract must substitute `@VERSION@` and `@KB_URL@` at request time so the served script reflects the running service version and configured KB URL.
- FR-20: The served installer script must not contain bearer tokens, deploy keys, raw transcripts, or other secrets.
- FR-21: The installer contract must support installing managed surfaces into user scope and project scope.
- FR-22: The installer contract must support preview and removal flows for managed surfaces.
- FR-23: personal-stack must consume the kit through a pinned version while remaining continuously auto-deployed and outside the kit release versioning model.
- FR-24: website consumption must be possible through the same versioned artifact contract when that repository opts into the kit.

## Success Criteria (SC-n, measurable)

- SC-1: Given a consumer repository pinned to version N, two renders from version N produce byte-for-byte identical managed surfaces.
- SC-2: Given one edited managed file in a consumer repository, the CI drift check exits non-zero and identifies at least one changed path.
- SC-3: Given generated surfaces that match the pinned version, the CI drift check exits zero.
- SC-4: Renderer help or contract tests list `--check`, `--write`, `--output`, and `--doctor` exactly once each.
- SC-5: `--check` completes without changing the consumer worktree in both passing and failing cases.
- SC-6: `--output` can render to an empty temporary directory and leaves the consumer worktree unchanged.
- SC-7: `--doctor` exits non-zero for static drift and reports a warning or failure for unavailable KB reachability according to its configured strictness.
- SC-8: The manifest accounts for 100 percent of managed skills, hooks, settings, council assets, KB hooks, Spec Kit seeds, and installer entrypoints.
- SC-9: Every shared capability has entries for all supported agent surfaces, or exactly one explicit unsupported reason is present for each intentional gap.
- SC-10: A request to `/install.sh` returns HTTP 200 with a shell-script content type.
- SC-11: The `/install.sh` response contains no unresolved `@VERSION@` or `@KB_URL@` placeholders.
- SC-12: The `/install.sh` response contains the configured KB URL and a service version value for the running service.
- SC-13: A served installer body scan finds zero bearer tokens, deploy keys, or raw transcript markers.
- SC-14: A Renovate update can identify a newer kit version for a consumer pin without requiring personal-stack to publish a release version.
- SC-15: Consumer metadata using short coordinates contains no repeated plugin-marker segment.

## Assumptions

- The reference files under personal-stack describe the current compatibility contract but are not the long-term source location after extraction.
- personal-stack remains a continuously deployed consumer and does not become a versioned release artifact.
- website is an optional consumer and should not require a separate distribution model.
- The KB service remains the runtime owner of the public installer route.
- The exact artifact registry or package format can be chosen later as long as the short-coordinate and Renovate-pinned version requirements are met.

## Edge Cases

- A consumer pins a kit version that is unavailable or withdrawn.
- A consumer has generated files from one kit version but metadata pins another.
- A managed capability is intentionally unsupported for one agent surface.
- A local user has existing unmanaged files at paths that the installer wants to manage.
- A served installer contains an unresolved placeholder.
- A consumer uses dependency metadata that would duplicate a plugin-marker segment.
- The KB service has no configured version value at request time.
- CI runs in an environment without live KB access.

## Key Entities

- Kit artifact: The released JorisJonkers-dev/agent-kit package that contains managed surfaces and metadata.
- Pinned version: The immutable version selected by a consumer repository.
- Short coordinate: The compact dependency coordinate used by consumers and Renovate.
- Consumer repository: A repository such as personal-stack or website that renders managed surfaces from the kit.
- Agent surface: A generated destination family such as `.claude`, `.codex`, or `.agents`.
- Renderer contract: The user-facing render and verification modes preserved by the kit.
- Manifest: The parity, provenance, and integrity ledger for managed surfaces.
- Installer route: The KB service route that serves `/install.sh`.
- Installer script: The shell content loaded from `ClassPathResource` and templated with runtime values.
- Council bundle: The managed orchestration driver, prompts, schemas, configuration, and skill entrypoints.
- KB hook bundle: The managed hooks and skills that connect agent sessions to KB recall and capture workflows.
- Drift check: The CI-facing comparison between committed consumer surfaces and the pinned kit render.

## Out of Scope

- Implementing renderer, installer, manifest, packaging, or CI code in this specification.
- Changing KB service authentication, storage, recall behavior, or capture behavior beyond the installer route contract.
- Versioning personal-stack as a release artifact.
- Replacing the KB service, KB vault, or council orchestration model.
- Defining the final registry, package manager, or release automation mechanics beyond the required versioned artifact contract.
- Requiring website to consume the kit before a separate adoption decision.
- Managing secrets or credentials inside the kit artifact or generated surfaces.
