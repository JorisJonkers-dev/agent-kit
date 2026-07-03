# Repository Map

source: README.md; PORTABILITY.md; manifest.yaml; runner-manifests/README.md; render-agent-kit.py
built-at: 2026-07-02T22:50:34Z

This fragment is tracked seed content. Treat it as seed-if-absent and preserve
local edits during regeneration.

## Top-Level Areas

- `templates/repo/`: source templates for `.claude`, `.codex`, `.agents`, and
  project Spec Kit seed payload.
- `templates/installer/`: installer templates and partials.
- `templates/runner-runtime/`: source templates for runner runtime artifacts.
- `installer/`: rendered install scripts served by the KB service.
- `runner-manifests/`: rendered runtime package artifacts, validation fixtures,
  and platform-blueprints handoff docs.
- `council/`: council driver, prompts, schemas, and default config.
- `scripts/`: repository validation helpers.
- `manifest.yaml`: managed path, checksum, parity, and artifact ledger.

## Render Sources And Destinations

- `render-agent-kit.py` renders repo templates, runtime templates, and installer
  templates.
- `templates/repo/.specify/memory/constitution.md` seeds project-scope
  constitutions and is preserved when the destination already exists.
- `runner-manifests/runtime/` is rendered from `templates/runner-runtime/`.

## Boundaries

- Agent-kit owns portable runtime contracts, installer surfaces, hooks, skills,
  commands, and council bundle sources.
- Platform blueprints own cluster realization details such as namespaces, RBAC,
  storage, runner pods, scheduling, network policy, and observability.
