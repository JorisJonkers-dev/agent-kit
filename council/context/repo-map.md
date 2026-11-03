# Repository Map

source: README.md; PORTABILITY.md; manifest.yaml; runner-manifests/README.md; render-agent-kit.py
built-at: 2026-07-02T22:50:34Z

Edit this fragment under `council/context/`. `render-agent-kit.py --write`
copies it to `.council/context/` and overwrites any edit made there.

## Top-Level Areas

- `skills/repo/`: repo skills (`claude/` and `codex/` variants), rendered into
  the gitignored `.claude/skills/` and `.agents/skills/`.
- `agents/`: Claude subagent definitions, rendered into `.claude/agents/`.
- `council/context/`: this context pack, rendered into `.council/context/`.
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
- `runner-manifests/runtime/` is rendered from `templates/runner-runtime/`.

## Boundaries

- Agent-kit owns portable runtime contracts, installer surfaces, hooks, skills,
  commands, and council bundle sources.
- Platform blueprints own cluster realization details such as namespaces, RBAC,
  storage, runner pods, scheduling, network policy, and observability.
