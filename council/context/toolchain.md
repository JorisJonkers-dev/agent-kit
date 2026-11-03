# Toolchain

source: README.md; pyproject.toml; docs/PORTABILITY.md; council/README.md; council/council.toml
built-at: 2026-07-02T22:50:34Z

Edit this fragment under `council/context/`. `scripts/render_agent_kit.py --write`
copies it to `.council/context/` and overwrites any edit made there.

## Language And Package Tools

- Python package metadata lives in `pyproject.toml`.
- Required Python version: `>=3.11`.
- Runtime dependency: `PyYAML>=6.0,<7`.
- Development dependencies: `mypy`, `pytest`, `ruff`, and `types-PyYAML`.
- `uv` is the documented local package runner.

## Local Validation

Common checks from `README.md`:

- `uv sync --frozen`
- `uv run python scripts/render_agent_kit.py --check`
- `uv run python scripts/render_agent_kit.py --doctor`
- `uv run ruff check .`
- `uv run mypy`
- `uv run pytest`
- `uv run python scripts/validate_manifest.py`

Run `uv run python scripts/render_agent_kit.py --write` after a checkout or a pull. It
renders the gitignored `.claude/`, `.agents/` and `.council/context/` from
`skills/repo/`, `agents/` and `council/context/`, and the committed
`runner/manifests/runtime/` from `runner/template/`.

## Council Runtime Tools

- Council requires authenticated `claude` and `codex` CLIs.
- Default council config uses `standard` intensity, `claude:opus` planner A,
  `codex:gpt-5.5` planner B, `claude:opus` consolidator, and `claude:sonnet`
  verifier.
- Council supports `plan`, `fanout`, `fleet`, `split`, `config`, and
  `--self-test` workflow from the TypeScript CLI (`node council/council.mjs --self-test`).
