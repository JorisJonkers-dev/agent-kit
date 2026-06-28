# Agent Runner Manifest Skeleton

This directory is the round-3 design-first handoff for agent runner manifests.
It is not a deployment pack. Files here describe the boundary between
`agent-kit` runtime packaging and a future `platform-blueprints` deployment
blueprint.

## Owned By Agent Kit

`agent-kit` owns the runtime contract that must be true inside an agent runner
image or entrypoint:

- Supported agents and parity policy for Claude, Codex, shell, and future
  agent kinds.
- CLI home/config paths as runtime inputs, not cluster defaults.
- Entrypoint environment names and self-check names.
- Version marker checks for installed agent-kit surfaces.
- Repository input semantics for one primary repo plus optional extra repos.
- GitHub token helper command and environment contract, without credentials.
- MCP profile merge semantics using placeholder URLs and bearer-token env names.
- Spec Kit seed source, marker file, and update policy.
- Required tools the runtime image must provide for normal agent workflows.
- Published runtime-home artifact metadata for the versioned agent-kit bundle.

## Owned By Platform Blueprints

`platform-blueprints` owns every cluster realization detail:

- Namespace, labels, service accounts, RBAC, and service-account token policy.
- Credential persistence through PVCs or an equivalent store.
- Secret projection, rotation, and source paths.
- Interactive bootstrap, refresh probes, kit-install schedules, and cleanup.
- Runner Pod templates, resources, security context, host mounts, and optional
  Docker socket access.
- Scheduling, affinity, tolerations, node labels, storage class, and topology.
- Network policy, egress destinations, service discovery, and observability.
- Rendered Kubernetes, Flux, Helm, or Kustomize manifests.

## Files

- `runtime-package.schema.json`: descriptive schema for an
  `AgentRuntimePackage` contract.
- `runtime/`: rendered portable runtime package artifacts owned by
  `agent-kit`: image build input, entrypoint, token helpers, and MCP profile
  placeholders.
- `fixtures/runtime-package.placeholder.yaml`: placeholder runtime package
  fixture for agent-kit-owned inputs.
- `platform-blueprints-handoff.md`: blueprint-side responsibilities and open
  renderer inputs.
- `fixtures/platform-blueprint-values.placeholder.yaml`: placeholder deployment
  intent fixture for platform-blueprints. This is not a Kubernetes manifest.

## Design-First Guardrails

Placeholder fixtures must stay placeholder-only. Do not add concrete domains,
hostnames, IP addresses, namespaces, image prefixes, secret paths, queue names,
or endpoint URLs to fixtures. Do not add Kubernetes workload resources in this
directory. The local manifest validator enforces those constraints.
