# Harness matrix (#856)

Every delegation role (#855) runs on every harness the kit supports, and a repeatable proof says which cells
work. Each cell is `model-gateway run --profile <role> --harness <harness>` on a trivial fixture
(`Reply with the single word ok.`, read-only, no tools), so the proof exercises the path a delegation takes:
the profile's models cut by the data policy, the harness choice, the launch and the result parse.

`pi` is every Pi route (the default of a role); `pi-opencode-go` is OpenCode Go through Pi; `claude`, `codex`,
`agy`, `vibe` and `hermes` name themselves.

## The matrix on the owner's machine

A cell is `pass` (the run answered), `blocked` (it cannot run here for a stated reason, which is a missing
prerequisite and not a bug) or `fail` (it should work and does not). Blocked cells are listed, never hidden.

<!-- matrix:start -->
| role | pi | pi-opencode-go | claude | codex | agy | vibe | hermes |
|---|---|---|---|---|---|---|---|
| research | pass | pass | pass | blocked [1] | pass | pass | blocked [2] |
| implement | pass | pass | pass | blocked [1] | pass | pass | blocked [2] |
| critique | pass | pass | pass | blocked [1] | pass | pass | blocked [2] |
| red-team | pass | pass | pass | blocked [1] | pass | pass | blocked [2] |
| refute | pass | pass | pass | blocked [1] | pass | pass | blocked [2] |
| triage | pass | pass | pass | blocked [1] | pass | pass | blocked [2] |

[1] codex: metered-employer: needs a repo whose allowlist names openai, and a grant for the harness-matrix workflow (model-gateway grant write)

[2] hermes: no model configured: hermes takes its model from its own config, so no profile lists one
<!-- matrix:end -->

The same file, `docs/model-gateway/harness-matrix.json`, is what the proof judges against; a Rust test keeps
the table above equal to it. The table is what the cells are expected to do, not a record of the last run.

### Last live run (2026-11-16, the `research` row only)

| harness | result | model | latency | cost |
|---|---|---|---|---|
| claude | pass | claude-haiku-5-5 | 11 s | $0.006 |
| agy | pass | gemini-3.8-flash-low | 14 s | $0 |
| vibe | pass | mistral-small-latest | 7 s | $0.001 |
| hermes | blocked | none | 2 s | $0 |
| pi | **fail** | pi-vibe:ministral-8b | 4 s | $0 |
| pi-opencode-go | **fail** | pi-opencode-go:qwen3.8-flash | 4 s | $0 |

The two Pi cells fail on this machine today, with or without a profile: the launched Pi exits 1 with `Unknown
provider "litellm-vibe"` (and an `EACCES` on `~/.pi/agent/settings.json.lock`), so it is not reading the per-run
home the gateway built. That is a defect of the Pi launch, not of the matrix, and the baseline keeps expecting
a pass so the live proof stays red until it is fixed. The other roles and the codex cells were not run live.

## Running it

```sh
# headless proof: the harnesses are stub programs on PATH, nothing is spent. CI runs it as the
# `matrix-stub` scenario of the model-gateway tests (cargo test -p model-gateway).
model-gateway matrix --json '{"repo": "/path/to/fixture/repo"}'

# live proof on the owner's machine: the real clients, a few cents. Narrow it with roles and harnesses.
model-gateway matrix --json '{"live": true}'
model-gateway matrix --json '{"live": true, "roles": ["red-team"], "harnesses": ["pi", "claude"]}'
```

The command prints one object: `status` (`ok` or `regressed`), `cells` (each with `role`, `harness`, `status`,
`model`, `exit`, `latency_ms`, `cost_usd` and `error`), `regressions` and the `table`. It exits 1 when a cell
does not do what the baseline says: a cell that should pass and does not, and also a blocked cell that now
passes (the baseline is stale, so edit `harness-matrix.json` and the table above). The stub proof expects every
cell to pass except hermes; the live proof expects the blocked cells listed above.

## Why the blocked cells are blocked

- **hermes** takes its model from its own configuration, not from the gateway's catalogue, so no profile lists
  one. Configure a model for it and add it to the profiles to unblock the column.
- **agy** passes on a machine where it is signed in. On one where it is not, its runs fail with the sign-in
  error, which the proof reports as blocked (and, as the baseline expects a pass, as a regression to resolve by
  signing in).
- **codex** is metered-employer: it is reached only for a repository whose allowlist names `openai`, and a
  grant for the `harness-matrix` workflow must exist (`model-gateway grant write`). Pass a `repo` that has
  both to run the codex column live.
