# agent-kit

Versioned source for the JorisJonkers-dev agent renderer and installer kit.

The repo owns the checked-in templates, manifest, council bundle, generated
agent surfaces, and `installer/install.sh` artifact that the KB service serves
as `ClassPathResource("installer/install.sh")`.

## Layout

- `render-agent-kit.py`: renderer and doctor CLI.
- `manifest.yaml`: managed path, checksum, parity, and artifact ledger.
- `templates/repo/`: sources for checked-in `.claude`, `.codex`, `.agents`,
  and `.specify` surfaces.
- `templates/installer/`: installer template and partials.
- `templates/runner-runtime/`: sources for portable agent-runner runtime
  packaging artifacts.
- `runner-manifests/`: runtime package artifacts and non-deployable platform
  handoff fixtures.
- `installer/install.sh`: generated serving artifact with `@VERSION@` and
  `@KB_URL@` placeholders.
- `council/`: council driver, prompts, schemas, and default config.

## Render

```bash
uv run python render-agent-kit.py --check
uv run python render-agent-kit.py --write
uv run python render-agent-kit.py --output /tmp/agent-kit-render
uv run python render-agent-kit.py --doctor
```

`--check` and `--doctor` are read-only. Use `--write` only when intentionally
updating checked-in generated surfaces from `templates/` or `council/`.

## Validate

```bash
uv sync --frozen
uv run ruff check .
uv run mypy
uv run pytest
uv run python scripts/validate_manifest.py
uv run python scripts/validate_manifest.py --runtime-selftest
uv run python -m compileall render-agent-kit.py scripts council
bash -n installer/install.sh
```

CI runs default manifest validation without the opt-in runtime credential
self-test, plus `render-agent-kit.py --check`, `--doctor`, and an `--output`
smoke render. The required terminal check is `Pipeline Complete`.

## Install

Install the generated client kit through a KB service that serves this
repository's `installer/install.sh`:

```bash
curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  "${KB_URL}/install.sh" | bash -s -- --agent all --scope user
```

Use `--scope project` with `AGENT_KIT_PROJECT_ROOT` to install repo-local
`.claude` and `.codex` surfaces. Use `--dry-run` to preview and `--uninstall`
to remove managed files. See [PORTABILITY.md](PORTABILITY.md).

### Full agents-system installer

`installer/install-agents.sh` is a thin wrapper that installs the full client
agents system in one shot. It delegates the base install above
(hooks + skills + council + Spec Kit), additionally **registers the
`knowledge` MCP server** for both Claude Code and Codex, and best-effort
installs the kit's Python dependencies (`PyYAML`, `ruff` — council needs
`python3`):

```bash
curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  "${KB_URL}/install-agents.sh" | bash -s -- --scope user
```

It targets all agents by default and honors the same `--scope user|project`,
`--dry-run`, and `--uninstall` flags. `KB_BEARER_TOKEN` is required in the
environment.

## Release

Consumers pin this kit by repository release tag, using the short coordinate
recorded in `manifest.yaml`:

```text
github:JorisJonkers-dev/agent-kit
```

Release Please creates version tags from merged conventional commits. Releases
publish the rendered runtime-home bundle to
`ghcr.io/jorisjonkers-dev/agent-kit/runtime-home` and attach the tarball plus
sha256 file to the GitHub release.
