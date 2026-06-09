# Technical Plan: Agent Runner Manifest Boundary

## Implementation Context

This round is design-first only. The implementation creates a bounded Spec Kit
feature plus non-deployable skeleton contracts under `runner-manifests/`.
Existing renderer-managed files stay untouched. The only executable change is a
small extension to the Python manifest validator so CI rejects concrete
deployment values in the skeleton.

## Chosen Technology

- Markdown for the boundary spec and handoff documentation.
- JSON Schema as the descriptive runtime package contract because it is easy to
  parse with the existing CI JSON validation and does not imply a Kubernetes
  resource.
- YAML placeholder fixtures because later platform work is expected to consume
  declarative values, while this repo must not ship production manifests.
- Python stdlib plus PyYAML in `scripts/validate_manifest.py`, matching the
  existing validation style.

## Architecture

`agent-kit` owns a descriptive runtime package contract:

- Supported agent surfaces and parity expectations.
- Agent CLI home/config paths as consumer-provided runtime values.
- Entrypoint environment contract and self-check names.
- Repository input environment contract.
- GitHub token helper command/env contract.
- MCP profile placeholder contract.
- Agent-kit installer/version marker and Spec Kit seed contract.

`platform-blueprints` owns all cluster realization:

- Namespace and service-account creation.
- RBAC and service-account token policy.
- PVC or other credential persistence.
- Secret projection and rotation.
- Bootstrap, refresh, and installer schedules.
- Runner Pod factory templates.
- Node placement, host mounts, security context, resources, and network policy.
- Internal service discovery for optional MCP sidecars.
- Rendered Kubernetes manifests.

No file in this feature is intended to be applied by `kubectl`, Flux, Helm, or
Kustomize.

## File Layout

```text
runner-manifests/
|-- README.md
|-- runtime-package.schema.json
|-- platform-blueprints-handoff.md
`-- fixtures/
    |-- runtime-package.placeholder.yaml
    `-- platform-blueprint-values.placeholder.yaml

specs/002-round3-agent-runner-manifests/
|-- spec.md
|-- plan.md
`-- tasks.md
```

## FR Traceability

| Requirement | Design Element |
| --- | --- |
| FR-001 | New `specs/002-round3-agent-runner-manifests/` files. |
| FR-002 | `runner-manifests/README.md` runtime ownership section. |
| FR-003 | `runner-manifests/platform-blueprints-handoff.md`. |
| FR-004 | `runner-manifests/runtime-package.schema.json`. |
| FR-005 | `runner-manifests/fixtures/platform-blueprint-values.placeholder.yaml`. |
| FR-006 | Validator rejects production Kubernetes kinds in skeleton fixtures. |
| FR-007 | Validator rejects concrete deployment-value patterns in round-3 paths. |
| FR-008 | Runtime schema and fixtures use placeholder MCP URL and bearer env fields. |
| FR-009 | Runtime schema models supported agents and parity policy. |
| FR-010 | `scripts/validate_manifest.py` parses skeleton JSON/YAML. |
| FR-011 | `scripts/validate_manifest.py` rejects deployable Kubernetes kinds. |

## Constitution Check

- [x] No attribution is introduced in files, comments, commit text, or PR text
- [x] Claude/Codex parity is preserved for agent-facing behavior
- [x] Rendered artifacts are not hand-edited
- [x] Small stacked PR boundary is clear and unrelated cleanup is excluded
- [x] Verification command is identified for each touched area

## Validation Plan

- `python3 scripts/validate_manifest.py`
- `python3 render-agent-kit.py --check`
- `python3 render-agent-kit.py --doctor`
- `python3 -m compileall render-agent-kit.py scripts council`
- `ruff check .`

Networked build, Kubernetes schema validation, and deployment rendering are
intentionally not run in this sandbox and are out of scope for the
design-first skeleton.

## Deviations From Reference

- Concrete namespaces, node labels, hostnames, domains, image repositories,
  Vault paths, and endpoint URLs are replaced with placeholders.
- Kubernetes resource files are not copied or templated.
- The cluster-side handoff is an intent fixture, not an API promise or renderer.

## Progress Tracking

- [x] Phase 0: Reference review complete
- [x] Phase 1: Design complete
- [x] Phase 2: Task planning complete
- [x] Phase 3: Skeleton files complete
- [x] Phase 4: Local validation complete
