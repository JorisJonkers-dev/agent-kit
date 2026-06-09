# Agent Runner Manifest Boundary

## Overview

Round 3 defines the boundary for portable agent runner manifests. This feature
is design-first only: it documents the contract between agent runtime packaging
owned by `agent-kit` and cluster deployment blueprints owned by
`platform-blueprints`, then adds skeleton files and fixtures that are
deliberately non-deployable.

The goal is to prevent future extraction work from copying a consumer cluster's
agent manifests, credentials, namespaces, endpoints, image names, storage
assumptions, or node placement rules into this repository. `agent-kit` owns the
agent-facing runtime contract: CLI parity, homes, entrypoint expectations, kit
installation checks, MCP profile placeholders, repository input semantics, and
Spec Kit seeding. `platform-blueprints` owns the cluster shape: namespaces,
RBAC, PVCs, secret projection, schedules, node placement, network policy,
optional Docker socket access, observability hooks, and rendered Kubernetes
objects.

## User Scenarios

### User Story 1 - Boundary Review (Priority: P1)

A maintainer reviews the round-3 extraction boundary and can tell whether a
future change belongs in `agent-kit` runtime packaging or in a deployment
blueprint repository.

**Why this priority**: This prevents ownership drift before any manifest
renderer is implemented.

**Independent Test**: Read the spec and skeleton README and classify a runtime
entrypoint change, a PVC storage-class choice, and an MCP endpoint value without
ambiguity.

**Acceptance Scenarios**:

1. **Given** a change to CLI home paths, entrypoint self-checks, or Spec Kit
   seeding, **When** a maintainer consults the boundary, **Then** the change is
   assigned to `agent-kit`.
2. **Given** a change to namespace, service account, PVC, secret projection,
   scheduling, or network policy, **When** a maintainer consults the boundary,
   **Then** the change is assigned to `platform-blueprints`.

### User Story 2 - Placeholder Fixture Authoring (Priority: P2)

A platform maintainer can start designing a deployment blueprint from a
placeholder fixture without receiving production manifests or inherited
consumer-specific values.

**Why this priority**: The next extraction step needs stable handoff inputs, not
copied manifests.

**Independent Test**: Inspect each fixture and confirm it contains only
placeholder values and no applicable Kubernetes workload resources.

**Acceptance Scenarios**:

1. **Given** the fixture directory, **When** a maintainer opens the files,
   **Then** the files describe intent using placeholders rather than concrete
   domains, namespaces, image prefixes, IPs, storage classes, or secret paths.
2. **Given** the fixture directory, **When** validation runs, **Then** Kubernetes
   production resource kinds such as Pods, Deployments, CronJobs, Services,
   PVCs, NetworkPolicies, Roles, and ServiceAccounts are rejected.

### User Story 3 - Runtime Contract Validation (Priority: P3)

A kit maintainer can run the existing local validation command and catch
accidental personal or deployable values added to the skeleton.

**Why this priority**: Design-first artifacts still need guardrails because the
reference material contains concrete deployment details.

**Independent Test**: Run the manifest validator and confirm it parses skeleton
JSON/YAML and rejects forbidden concrete value patterns.

**Acceptance Scenarios**:

1. **Given** checked-in skeleton files, **When** the existing manifest
   validation command runs, **Then** JSON/YAML syntax and design-first guardrails
   pass.
2. **Given** a skeleton file containing a concrete domain, IP, namespace, image
   prefix, or deployable Kubernetes resource kind, **When** validation runs,
   **Then** the command exits non-zero.

## Functional Requirements

- **FR-001**: The feature MUST create a spec-kit feature directory with
  `spec.md`, `plan.md`, and `tasks.md`.
- **FR-002**: The boundary documentation MUST define `agent-kit` ownership for
  runtime packaging, CLI parity, entrypoint contracts, agent homes, MCP profile
  placeholders, repository input semantics, GitHub token helper interfaces,
  agent-kit installer/version checks, and Spec Kit seeding.
- **FR-003**: The boundary documentation MUST define `platform-blueprints`
  ownership for namespaces, RBAC, service accounts, PVCs, secret projection,
  schedules, node placement, host mounts, network policy, service discovery,
  observability, and rendered Kubernetes manifests.
- **FR-004**: The skeleton MUST include a runtime package contract that is
  descriptive and non-deployable.
- **FR-005**: The skeleton MUST include a platform handoff fixture that
  describes blueprint inputs using placeholders only.
- **FR-006**: The skeleton MUST NOT include working Kubernetes deployment
  manifests, Kustomizations, Helm releases, or renderer implementation.
- **FR-007**: The skeleton MUST NOT include personal credentials, personal
  domains, concrete namespaces, concrete hostnames, IP addresses, concrete image
  prefixes, concrete Vault paths, queue names, or KB endpoint URLs.
- **FR-008**: The skeleton MUST represent MCP and knowledge integration through
  placeholder URL and bearer-token environment contracts only.
- **FR-009**: The skeleton MUST keep Claude and Codex as first-class runtime
  surfaces, and any future agent-specific gap MUST require an explicit
  unsupported reason.
- **FR-010**: Validation MUST parse the new JSON/YAML skeleton files and reject
  forbidden concrete deployment values in the round-3 skeleton paths.
- **FR-011**: Validation MUST reject deployable Kubernetes resource kinds in the
  skeleton fixture directory.

## Key Entities

- **AgentRuntimePackage**: A non-deployable contract for the runtime image and
  entrypoint surface owned by `agent-kit`.
- **AgentRunnerDeploymentIntent**: A non-deployable placeholder fixture for the
  cluster inputs a deployment blueprint must later render.
- **Runtime surface**: Agent-facing behavior such as CLI homes, version checks,
  MCP profile merge semantics, repository cloning inputs, credential helper
  contracts, and SDD seeding.
- **Cluster surface**: Deployment-facing behavior such as namespace, scheduling,
  storage, secrets, RBAC, network, CronJobs, services, and observability.
- **MCP profile placeholder**: A profile entry that names an integration point
  without baking endpoint URLs or bearer values into the kit.

## Success Criteria

- **SC-001**: The new spec directory contains `spec.md`, `plan.md`, and
  `tasks.md` in the same style as the existing specs.
- **SC-002**: A reviewer can classify at least ten listed responsibilities into
  `agent-kit` versus `platform-blueprints` from the boundary docs.
- **SC-003**: Skeleton fixtures contain no concrete domains, hostnames, IPs,
  namespaces, image prefixes, secret paths, queue names, or endpoint URLs.
- **SC-004**: Skeleton fixtures contain no Kubernetes production resource kind
  that could be applied as a deployment.
- **SC-005**: `python scripts/validate_manifest.py` passes with the skeletons in
  place.
- **SC-006**: `python -m compileall render-agent-kit.py scripts council` passes.
- **SC-007**: Existing render checks remain green because no render-managed
  surfaces are edited by hand.

## Assumptions

- Deployment manifest rendering will be implemented later in
  `platform-blueprints`, after the platform pack boundary is ready.
- `agent-kit` may later ship reusable runtime scripts or image templates, but
  this round ships only contracts and placeholders.
- Consumers will provide their own endpoint, namespace, image, storage,
  scheduling, and secret values.

## Edge Cases

- A future maintainer wants to add a Kubernetes manifest directly to
  `agent-kit`.
- A future maintainer wants to make an MCP profile point at a concrete service
  URL.
- A runtime image needs a new CLI or tool that exists only for one agent.
- A platform blueprint wants optional Docker socket access for build/test
  workflows.
- A consumer uses RWX storage rather than node-pinned RWO credentials.
- A consumer does not enable a GitHub App token helper and relies on read-only
  clone credentials.

## Out of Scope

- Copying or adapting production Kubernetes manifests.
- Implementing a deployment renderer.
- Selecting namespaces, storage classes, node labels, domains, hostnames,
  image repositories, secret paths, or KB endpoint URLs.
- Moving platform-blueprint files into this repository.
- Changing the existing agent-kit renderer or installer behavior beyond adding
  validation for the new design-first skeleton.
