# agent-kit

Versioned source for the JorisJonkers-dev agent renderer, installer kit, and
portable runner runtime templates.

## What It Is

`agent-kit` owns the checked-in agent templates, generated Claude/Codex surfaces,
council bundle, installer artifacts, and runner runtime manifests used by
JorisJonkers-dev automation.

## Local Use

```bash
uv sync --frozen
uv run python render-agent-kit.py --check
uv run python render-agent-kit.py --doctor
uv run ruff check .
uv run mypy
uv run pytest
uv run python scripts/validate_manifest.py
uv run python scripts/render_registry.py --check
```

Use `uv run python render-agent-kit.py --write` only when intentionally updating
checked-in generated surfaces from `templates/` or `council/`. Use
`scripts/build-runtime-package.sh v0.0.0-local dist` to build the runtime-home
bundle locally.

## Workstation Setup

```bash
./installer/setup-workstation.sh --check     # report only
./installer/setup-workstation.sh             # install/upgrade everything
```

Claude Code and Codex at their latest releases, every plugin, the language
servers those plugins drive, the MCP fleet, and the first-party skills. The
script is generated from `registry/estate-tooling.yaml` -- see
[docs/SETUP.md](docs/SETUP.md).

## Docs

- [docs/SETUP.md](docs/SETUP.md): workstation setup, credentials, exit codes.
- [docs/REGISTRY.md](docs/REGISTRY.md): the one file to edit to add a skill,
  plugin, CLI or MCP server to every surface at once.
- [docs/HERMES.md](docs/HERMES.md): getting a registry entry onto the
  in-cluster gateway, and verifying it landed.
- [docs/MEMORY.md](docs/MEMORY.md): what replaces the retired knowledge base.

The estate ships **no agent hooks**. `install-agents.sh` purges the retired
knowledge hooks from a machine that still has them.

## Layout

- `registry/estate-tooling.yaml`: the tooling registry (hand-edited).
- `installer/setup-workstation.sh`, `registry/generated/hermes/`: generated from
  it by `scripts/render_registry.py`. Never hand-edit these.
- `skills/`: first-party multi-file skills copied onto a surface.
- `templates/repo/`: source templates for `.claude`, `.codex`, `.agents`, and
  project Spec Kit seed payload.
- `templates/installer/`: installer templates and partials.
- `templates/runner-runtime/`: source templates for runner runtime artifacts.
- `installer/`: rendered install scripts served by the KB service.
- `runner-manifests/`: rendered runtime package artifacts and validation
  fixtures.
- `council/`: council driver, prompts, schemas, and default config.
- `manifest.yaml`: managed path, checksum, parity, and artifact ledger.

## Install

Install the rendered kit through a KB service that serves this repository's
installer artifacts:

```bash
curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  "${KB_URL}/install-agents.sh" | bash -s -- --scope user
```

Use `--scope project` with `AGENT_KIT_PROJECT_ROOT` for repo-local agent
surfaces. See [PORTABILITY.md](PORTABILITY.md).

## Links

- [Organization profile](https://github.com/JorisJonkers-dev)
- [Security policy](https://github.com/JorisJonkers-dev/.github/security/policy)
- [Changelog](CHANGELOG.md)
- [License](LICENSE)

Copyright (c) Joris Jonkers. Source available for viewing only; use, copying,
modification, redistribution, deployment, or reuse is not licensed. See
[LICENSE](LICENSE).
