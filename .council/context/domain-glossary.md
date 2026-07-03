# Domain Glossary

source: README.md; PORTABILITY.md; manifest.yaml; council/README.md; runner-manifests/README.md
built-at: 2026-07-02T22:50:34Z

This fragment is tracked seed content. Treat it as seed-if-absent and preserve
local edits during regeneration.

- Agent kit: versioned repository that owns templates, generated agent
  surfaces, installer artifacts, council bundle, and runtime manifests.
- Agent surface: files installed for a specific agent client, including Claude
  commands, Claude/Codex skills, hooks, and settings.
- Council: cross-model planning and fan-out orchestrator that plans with Claude
  and Codex, critiques, consolidates, then executes workers in isolated
  worktrees.
- Fanout: council execution phase that topologically sorts task DAG waves,
  dispatches workers, verifies results, and reconciles onto an integration
  branch.
- Fleet: ad-hoc council worker pool over an already decomposed `tasks.json`.
- Spec Kit seed: project `.specify` scaffold, including the hand-edited
  constitution seed and render-managed templates/scripts.
- Render-managed file: checked-in output whose source is a template or
  inventory entry; edit the source and rerender the output.
- Runtime home: published OCI package containing portable agent runner runtime
  home artifacts.
- Runner manifest: agent-kit-owned runtime package contract and placeholder
  handoff material for deployment blueprints.
- KB service: service that serves installer artifacts and provides knowledge
  tooling used by hooks and agent workflows.
- Parity: requirement that shared agent behavior remains equivalent across
  Claude and Codex surfaces or records an explicit unsupported reason.
