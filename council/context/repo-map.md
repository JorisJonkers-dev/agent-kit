# Repository Map

source: README.md; docs/PORTABILITY.md; manifest.yaml; runner/manifests/README.md; scripts/render_agent_kit.py
built-at: 2026-07-02T22:50:34Z

Edit this fragment under `council/context/`. `scripts/render_agent_kit.py --write`
copies it to `.council/context/` and overwrites any edit made there.

## Top-Level Areas

- `skills/workstation/`: skills setup installs into agent homes.
- `skills/repo/`: repo skills (`claude/` and `codex/` variants), rendered into
  the gitignored `.claude/skills/` and `.agents/skills/`.
- `agents/`: Claude subagent definitions, rendered into `.claude/agents/`.
- `council/`: council driver, prompts, schemas, and default config;
  `council/context/` is this context pack, rendered into `.council/context/`.
- `runner/template/`: source templates for runner runtime artifacts.
- `runner/manifests/`: rendered runtime package artifacts, validation fixtures,
  and platform-blueprints handoff docs.
- `installer/`: `setup-workstation.sh`, generated from
  `registry/estate-tooling.yaml` by `scripts/render_registry.py`, and the
  scripts it sources; `installer/cloud/` holds the cloud environment files.
- `scripts/`: renderers, validators, release helpers and `deploy/`.
- `provenance/`: lock files for adapted upstream sources.
- `registry/`: the tooling registry and the artifacts rendered from it.
- `docs/`: setup, registry, cloud, portability and critique-pipeline docs, and
  the ADRs.
- `.github/`: workflows, the Renovate config and the security policy.
- `manifest.yaml`: managed path, checksum, parity, and artifact ledger.

## Render Sources And Destinations

- `scripts/render_agent_kit.py` renders the repo skills and agents into
  `.claude/` and `.agents/`, this context pack into `.council/context/`
  (`--write` only), and the runtime package.
- `scripts/render_registry.py` renders `installer/setup-workstation.sh` and
  `registry/generated/` from `registry/estate-tooling.yaml`.
- `runner/manifests/runtime/` is rendered from `runner/template/`.

## Boundaries

- Agent-kit owns portable runtime contracts, installer surfaces, hooks, skills,
  commands, and council bundle sources.
- Platform blueprints own cluster realization details such as namespaces, RBAC,
  storage, runner pods, scheduling, network policy, and observability.
