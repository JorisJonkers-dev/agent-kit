# Technical Plan: Agent Runner Runtime Packaging

## Implementation Context

Round 4 converts the Round 3 agent-kit boundary from descriptive skeleton into
rendered runtime package artifacts. The implementation stays within the
existing Python renderer, manifest ledger, and validator conventions.

## Chosen Technology

- Python stdlib and PyYAML validation, matching `scripts/validate_manifest.py`.
- Rendered shell scripts for entrypoint and token helpers.
- JSON for Claude MCP server profiles.
- TOML for Codex MCP server profiles, parsed with Python 3.11 `tomllib`.
- YAML for the rendered `AgentRuntimePackage`.
- A Containerfile as build input only; no container build is run in this repo.

## Architecture

`templates/runner-runtime/` is the new source of truth for runtime artifacts.
`render-agent-kit.py` copies that template tree into
`runner-manifests/runtime/` and preserves executable modes.

The rendered runtime package includes:

- Containerfile and build env example with consumer-supplied image parameters.
- Entrypoint with local self-tests, repository allow-listing, Spec Kit seeding,
  Claude/Codex trust setup, and MCP profile merge behavior.
- Git token helper, gh wrapper, git credential helper, and Git MCP wrapper.
- Claude JSON and Codex TOML MCP profiles for minimal, frontend, cluster,
  code-intel, and full-diagnostic modes.
- `runtime-package.yaml` tying those files together as a portable contract.

The platform side remains only:

- `runner-manifests/platform-blueprints-handoff.md`
- `runner-manifests/fixtures/platform-blueprint-values.placeholder.yaml`

No deployment manifests are produced.

## File Layout

```text
templates/runner-runtime/
|-- Containerfile
|-- build.env.example
|-- runtime-package.yaml
|-- entrypoint.sh
|-- bin/
|   |-- agent-github-token
|   |-- gh-wrapper
|   |-- git-credential-agent-gh-app
|   `-- gh-mcp-wrapper
`-- mcp/
    |-- claude-mcp-servers.*.json
    `-- codex-mcp-servers.*.toml

runner-manifests/runtime/
`-- rendered copy of templates/runner-runtime/

specs/003-round4-agent-runner-runtime-packaging/
|-- spec.md
|-- plan.md
`-- tasks.md
```

## FR Traceability

| Requirement | Design Element |
| --- | --- |
| FR-001 | New `specs/003-round4-agent-runner-runtime-packaging/` files. |
| FR-002 | `render-agent-kit.py` runtime template root. |
| FR-003 | `manifest.yaml` renderer and `agent_runner_runtime` sections. |
| FR-004 | `Containerfile`, `build.env.example`, runtime validator guardrails. |
| FR-005 | `entrypoint.sh` self-test modes and validator execution. |
| FR-006 | `entrypoint.sh` `REPO_URL`, `REPO_BRANCH`, `REPO_URLS` handling. |
| FR-007 | Runtime `bin/` helper scripts. |
| FR-008 | Runtime `mcp/` profiles and placeholder validation. |
| FR-009 | `scripts/validate_manifest.py` default offline runtime artifact validation. |
| FR-010 | Round 3 handoff fixture validation remains non-deployable. |
| FR-011 | `scripts/validate_manifest.py --runtime-selftest` opt-in credential helper token probe. |

## Validation Plan

- `python3 render-agent-kit.py --check`
- `python3 render-agent-kit.py --doctor`
- `python3 scripts/validate_manifest.py`
- `python3 scripts/validate_manifest.py --runtime-selftest` for runtime-only credential helper token probing
- `python3 -m compileall render-agent-kit.py scripts council`
- `ruff check .`
- `bash -n` on rendered runtime shell scripts through the validator

Networked builds and Kubernetes validators are intentionally not run in this
sandbox.

## Constitution Check

- [x] No attribution is introduced in files, comments, commit text, or PR text
- [x] Claude/Codex parity is preserved for runtime packaging
- [x] Rendered artifacts are generated from templates
- [x] Deployment manifests stay out of agent-kit
- [x] Verification command is identified for each touched area

## Progress Tracking

- [x] Phase 0: Boundary and reference review
- [x] Phase 1: Runtime artifact design
- [x] Phase 2: Renderer and template implementation
- [x] Phase 3: Manifest and validator integration
- [x] Phase 4: Local non-network validation
