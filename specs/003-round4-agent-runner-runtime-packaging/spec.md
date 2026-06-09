# Agent Runner Runtime Packaging

## Overview

Round 4 implements the `agent-kit` side of the agent-runner boundary defined in
Round 3. The feature turns the runtime package contract into rendered,
validated artifacts while keeping cluster deployment realization in
`platform-blueprints`.

`agent-kit` now owns portable runtime build inputs, entrypoint behavior,
installation checks, repository input semantics, Git token helper interfaces,
and MCP profile placeholders. The artifacts are rendered by
`render-agent-kit.py` from templates and pinned in `manifest.yaml`.

This feature deliberately does not render Kubernetes, Flux, Helm, Kustomize, or
secret material. Consumer-specific domains, hosts, namespaces, image
repositories, endpoints, storage, schedules, and network rules remain
parameters supplied outside this repository.

## User Scenarios

### User Story 1 - Build Runtime Image Inputs (Priority: P1)

A maintainer can render a portable runtime package that a consumer image build
pipeline can use without inheriting personal deployment values.

**Independent Test**: Run the renderer and inspect
`runner-manifests/runtime/Containerfile`, `build.env.example`, and
`runtime-package.yaml`; all concrete image, endpoint, host, and path choices are
parameters or placeholders.

**Acceptance Scenarios**:

1. **Given** the checked-in templates, **When** `python3 render-agent-kit.py
   --check` runs, **Then** rendered runtime artifacts match the templates.
2. **Given** a consumer image pipeline, **When** it reads the runtime package,
   **Then** it receives a container build input and required executable list
   without a concrete base image or image repository.

### User Story 2 - Runtime Entrypoint Contract (Priority: P1)

An agent runner can use the rendered entrypoint to validate installed kit
markers, derive repository allow-lists, seed Spec Kit into cloned repositories,
and configure Claude/Codex MCP profiles.

**Independent Test**: Run the local entrypoint self-tests through
`scripts/validate_manifest.py`; no network, credential helper token, or
container build is required for default validation. Runtime-only credential
helper token probing is opt-in with `--runtime-selftest` or
`AGENT_RUNTIME_SELFTEST=1`.

**Acceptance Scenarios**:

1. **Given** primary and additional repository inputs, **When** the `repo-allow`
   self-test runs, **Then** it emits only the allowed repository slugs.
2. **Given** a workspace with a git repository and an SDD seed source, **When**
   the `speckit-seed` self-test runs, **Then** the repo receives `.specify`
   seed files and a marker file without overwriting changed files.
3. **Given** Claude and Codex homes, **When** the `agent-kit-manifest`
   self-test runs, **Then** missing or stale markers are warnings rather than
   fatal startup errors.

### User Story 3 - MCP Profile Placeholders (Priority: P2)

A platform blueprint can mount MCP profile files selected by
`AGENT_MCP_PROFILE`, while endpoint and bearer values come from runtime
environment variables.

**Independent Test**: Validate that every declared Claude profile parses as
JSON, every Codex profile parses as TOML, and profile contents contain
placeholder tokens rather than concrete vendor or cluster URLs.

**Acceptance Scenarios**:

1. **Given** the `minimal` profile, **When** the entrypoint starts, **Then**
   Claude and Codex receive knowledge and git-host MCP definitions.
2. **Given** an optional profile such as `cluster` or `frontend`, **When** the
   platform supplies endpoint env vars, **Then** the entrypoint substitutes
   those values at runtime without checking them into the repo.

### User Story 4 - Deployment Boundary Preservation (Priority: P2)

A platform maintainer can still use the handoff fixture as a skeleton without
finding deployable Kubernetes manifests in `agent-kit`.

**Independent Test**: Run manifest validation and confirm deployment resource
kinds and Kubernetes API versions remain rejected in the handoff fixtures.

## Functional Requirements

- **FR-001**: The feature MUST create a Spec Kit directory containing
  `spec.md`, `plan.md`, and `tasks.md`.
- **FR-002**: Runtime package artifacts MUST be rendered from templates by
  `render-agent-kit.py`.
- **FR-003**: `manifest.yaml` MUST include the runtime template root, runtime
  managed paths, and checksum pins for rendered runtime files.
- **FR-004**: Runtime image build inputs MUST avoid concrete base images,
  image repositories, hostnames, domains, namespaces, endpoints, secret paths,
  and platform paths.
- **FR-005**: The entrypoint MUST provide self-tests for kit marker checks,
  repository allow-list derivation, repository directory naming, and Spec Kit
  seeding.
- **FR-006**: Repository semantics MUST support a primary repository plus
  optional additional repositories using `url[#branch]` entries.
- **FR-007**: Git token helper contracts MUST be represented as portable
  helper scripts with endpoint URL, bearer, host, and cache settings supplied
  by environment variables.
- **FR-008**: MCP profiles MUST be rendered as Claude JSON and Codex TOML files
  with endpoint and bearer placeholders only.
- **FR-009**: Default validation MUST parse runtime package YAML, MCP JSON, and
  MCP TOML, and MUST run offline local shell syntax/self-test checks without
  requiring a configured git credential helper or preseeded repository token.
- **FR-011**: Runtime credential helper token probing MUST remain available as
  an explicit runtime self-test selected with `--runtime-selftest` or
  `AGENT_RUNTIME_SELFTEST=1`.
- **FR-010**: Cluster deployment concerns MUST remain contract-only:
  namespace, RBAC, storage, scheduling, network policy, secret projection,
  Flux/Kustomize/Helm manifests, and Kubernetes resources are out of scope.

## Key Entities

- **AgentRuntimePackage**: Rendered YAML contract describing build inputs,
  runtime entrypoint, repository semantics, token helper paths, MCP profiles,
  and platform handoff responsibilities.
- **Runtime image build input**: Containerfile and environment example that
  package scripts and profiles into a consumer-selected base image.
- **Entrypoint self-test**: Local shell mode selected by
  `AGENT_RUNNER_ENTRYPOINT_SELF_TEST` to validate runtime behavior without
  launching the final workload.
- **MCP profile placeholder**: Claude JSON or Codex TOML profile containing
  placeholder tokens such as `@KNOWLEDGE_MCP_URL@`.
- **Platform handoff fixture**: Non-deployable values skeleton consumed by a
  future `platform-blueprints` renderer.

## Success Criteria

- **SC-001**: `python3 render-agent-kit.py --check` passes.
- **SC-002**: `python3 render-agent-kit.py --doctor` passes without requiring
  live KB access.
- **SC-003**: `python3 scripts/validate_manifest.py` validates runtime package
  checksums, profile parsing, shell syntax, offline self-tests, placeholders,
  and non-deployable handoff fixtures without requiring a credential helper
  token.
- **SC-004**: `python3 -m compileall render-agent-kit.py scripts council`
  passes.
- **SC-005**: `ruff check .` passes where the local ruff executable is
  available.
- **SC-006**: `python3 scripts/validate_manifest.py --runtime-selftest` keeps
  the preseeded git credential helper probe available for runtime-oriented
  validation.

## Assumptions

- The consumer image build pipeline supplies the base image and installs agent
  CLIs and optional MCP server executables.
- The platform blueprint supplies concrete endpoint, bearer, namespace,
  storage, scheduling, and secret projection values.
- Existing agent-kit installer marker names may be overridden with
  `AGENT_KIT_VERSION_MARKER` if a consumer adopts a different marker.

## Out of Scope

- Building or publishing a runtime image.
- Running Docker, Kubernetes, Gradle, npm, Nix, kubeconform, or networked
  validation in this sandbox.
- Rendering Kubernetes Deployments, Pods, CronJobs, Services, PVCs, RBAC,
  NetworkPolicies, Flux Kustomizations, Helm releases, or secret resources.
- Checking in personal credentials, concrete endpoints, consumer namespaces,
  image prefixes, cluster DNS, storage classes, queue names, or host paths.
