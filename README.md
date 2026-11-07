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
uv run python scripts/render_agent_kit.py --check
uv run python scripts/render_agent_kit.py --doctor
uv run ruff check .
uv run mypy
uv run pytest
uv run python scripts/validate_manifest.py
uv run python scripts/render_registry.py --check
cargo test --manifest-path scripts/Cargo.toml
```

Python dependencies resolve with a seven-day window: `[tool.uv] exclude-newer = "7 days"`
in `pyproject.toml` refuses any release uploaded in the last week. The value is relative, so
the window moves forward at every `uv lock`, and `uv.lock` records it as `exclude-newer-span`.
`uv run --no-project --with ...` calls do not read `pyproject.toml`, so they pass
`--exclude-newer "7 days"` themselves. uv 0.9.17 is the oldest that applies the window, and
older uv ignores it without a word, so the floor is `required-version` in `pyproject.toml`
(current uv refuses to run below it) and the install paths pin at or above it.
`tests/test_uv_cooldown.py` proves a younger release is not resolved.

Run `uv run python scripts/render_agent_kit.py --write` after a checkout or a pull. It
renders the gitignored `.claude/`, `.agents/` and `.council/context/` from
`skills/repo/`, `agents/` and `council/context/`, and the committed
`runner/manifests/runtime/` from `runner/template/`. Use
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
- [docs/RUST.md](docs/RUST.md): the Rust workspace, the 7-day crate cooldown, the pre-commit hook, the pull-request gate and the security-fix waiver.
- [docs/REGISTRY.md](docs/REGISTRY.md): the one file to edit to add a skill,
  plugin, CLI or MCP server to every surface at once.
- [docs/MEMORY.md](docs/MEMORY.md): what replaces the retired knowledge base.
- [docs/CLOUD.md](docs/CLOUD.md): the same setup in a Claude Code cloud
  environment, from the two files in `installer/cloud/`.
- [skills/workstation/critique/README.md](skills/workstation/critique/README.md): the critique
  pipeline for its user: what runs when, depth, budget, the gates, the kill
  switch and how to retune routing.
- [docs/critique-pipeline/integration.md](docs/critique-pipeline/integration.md):
  what the critic CLIs were shown to do, where the critique pipeline lands in
  agent-kit, and where it departs from its brief.

The estate ships **four agent hooks**: the critique pipeline's commit gate,
PR gate and session-start line
([ADR-0002](docs/adr/0002-exactly-three-critique-hooks.md)), and the
projects-layout guard, which keeps clones and worktrees under
`~/Projects/<owner>/` ([ADR-0009](docs/adr/0009-projects-layout-guard.md)).
`setup-workstation.sh` merges them by name into Claude Code's `settings.json`
and Codex's `hooks.json`, leaving your own hooks alone, and purges the retired
knowledge hooks from a machine that still has them, on every run.

## Layout

- `registry/estate-tooling.yaml`: the tooling registry (hand-edited).
- `installer/setup-workstation.sh`, `registry/generated/`: generated from
  it by `scripts/render_registry.py`. Never hand-edit these.
- `skills/workstation/`: skills setup installs into agent homes.
- `skills/repo/`: repo skills, one folder per skill with a `claude/` and a
  `codex/` variant, rendered into `.claude/skills/` and `.agents/skills/`.
- `agents/`: Claude subagent definitions, rendered into `.claude/agents/`.
- `council/`: council driver, prompts, schemas, default config, and this
  repository's council context (`council/context/`, rendered into
  `.council/context/`).
- `runner/template/`: source templates for the runner runtime;
  `runner/manifests/`: the rendered runtime package, its schema and validation
  fixtures.
- `installer/`: `setup-workstation.sh` and `setup-container.sh`, generated as
  above, plus the hand-written `cloud-session.sh` they source and the
  commands setup puts on `PATH` (such as `kubernetes-mcp-readonly`). `installer/cloud/` holds the setup script and environment
  variables pasted into a Claude Code cloud environment.
- `scripts/`: the renderers (`render_agent_kit.py`, `render_registry.py`),
  validators, release helpers, the `deploy/` helpers the deploy skills use,
  and the Rust workspace (`Cargo.toml`, its one std-only crate) behind the
  7-day crate cooldown ([docs/RUST.md](docs/RUST.md)).
- `provenance/`: lock files recording the upstream sources adapted here.
- `docs/`: setup, registry, cloud, portability and critique-pipeline docs, and
  the ADRs.
- `.github/`: workflows, the Renovate config and the security policy.
- `.claude/`, `.agents/`, `.council/`: gitignored local output and agent
  state (worktrees, council runs). Nothing in them is tracked.
- `manifest.yaml`: managed path, checksum, parity, and artifact ledger.

## Install

From a checkout:

```bash
./installer/setup-workstation.sh
```

Or fetched over HTTP; outside a checkout it fetches the kit bundle of the
same release and runs that:

```bash
curl -fsSL https://github.com/JorisJonkers-dev/agent-kit/releases/latest/download/setup-workstation.sh | bash
```

`--uninstall` removes what a machine set up by the retired `install.sh` /
`install-agents.sh` wrote (base skills, Spec Kit commands/skills, their MCP
entries), purges the retired knowledge hooks and removes the critique hooks;
a later run without it installs them again. See
[docs/PORTABILITY.md](docs/PORTABILITY.md).

## Links

- [Organization profile](https://github.com/JorisJonkers-dev)
- [Security policy](https://github.com/JorisJonkers-dev/.github/security/policy)
- [Changelog](CHANGELOG.md)
- [License](LICENSE)

Copyright (c) Joris Jonkers. Source available for viewing only; use, copying,
modification, redistribution, deployment, or reuse is not licensed. See
[LICENSE](LICENSE).
