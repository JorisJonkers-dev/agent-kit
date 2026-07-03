# Toolchain

source: README.md; pyproject.toml; PORTABILITY.md; council/README.md; council/council.toml
built-at: 2026-07-02T22:50:34Z

This fragment is tracked seed content. Treat it as seed-if-absent and preserve
local edits during regeneration.

## Language And Package Tools

- Python package metadata lives in `pyproject.toml`.
- Required Python version: `>=3.11`.
- Runtime dependency: `PyYAML>=6.0,<7`.
- Development dependencies: `mypy`, `pytest`, `ruff`, and `types-PyYAML`.
- `uv` is the documented local package runner.

## Local Validation

Common checks from `README.md`:

- `uv sync --frozen`
- `uv run python render-agent-kit.py --check`
- `uv run python render-agent-kit.py --doctor`
- `uv run ruff check .`
- `uv run mypy`
- `uv run pytest`
- `uv run python scripts/validate_manifest.py`

Use `uv run python render-agent-kit.py --write` only when intentionally updating
checked-in generated surfaces from `templates/` or `council/`.

## Council Runtime Tools

- Council requires authenticated `claude` and `codex` CLIs.
- Default council config uses `standard` intensity, `claude:opus` planner A,
  `codex:gpt-5.5` planner B, `claude:opus` consolidator, and `claude:sonnet`
  verifier.
- Council supports `plan`, `fanout`, `fleet`, `split`, `config`, and
  `--self-test` workflow from the TypeScript CLI (`node council/council.mjs --self-test`).
