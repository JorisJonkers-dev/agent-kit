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
| role | pi | pi-opencode-go | claude | codex | agy | vibe | hermes | opencode |
|---|---|---|---|---|---|---|---|---|
| research | pass | pass | pass | blocked [1] | pass | blocked [2] | pass | pass |
| implement | pass | pass | pass | blocked [1] | pass | blocked [2] | pass | pass |
| critique | pass | pass | pass | blocked [1] | pass | blocked [2] | pass | pass |
| red-team | pass | pass | pass | blocked [1] | pass | blocked [2] | pass | pass |
| refute | pass | pass | pass | blocked [1] | pass | blocked [2] | pass | pass |
| triage | pass | pass | pass | blocked [1] | pass | blocked [2] | pass | pass |

[1] codex: metered-employer: needs a repo whose allowlist names openai, and a grant for the harness-matrix workflow (model-gateway grant write)

[2] vibe: explicit only: no profile lists a vibe model, so a profile run never picks the Vibe CLI; a run that names client vibe still runs
<!-- matrix:end -->

The same file, `docs/model-gateway/harness-matrix.json`, is what the proof judges against; a Rust test keeps
the table above equal to it. The table is what the cells are expected to do, not a record of the last run.

### Last live run (2026-11-16, Pi cells, every role)

All twelve `pi` and `pi-opencode-go` cells pass live (`model-gateway matrix --json '{"live": true,
"harnesses": ["pi", "pi-opencode-go"]}'`), each in 4 to 9 s at $0:

| harness | roles | models used |
|---|---|---|
| pi | all six pass | pi-vibe:ministral-8b, deepseek-v4.1-flash, glm-5.3 |
| pi-opencode-go | all six pass | pi-opencode-go:qwen3.8-flash, kimi-k2.7-code |

The `research` row for claude, agy and vibe passed earlier the same day. The codex cells
and the non-Pi harnesses of the other roles were not re-run.

### What broke Pi (#870), and why it stays fixed

`agent-launcher confine` clears the environment down to its own short list, plus each variable the caller names
with `--env`. The gateway set `PI_CODING_AGENT_DIR` and `TMPDIR` on the launcher process but never named them,
so Pi did not receive them, fell back to the real `~/.pi/agent` (no `litellm-*` provider there, hence
`Unknown provider`) and tried to take `settings.json.lock` in a directory the confinement keeps read-only
(`EACCES`). The launch now passes `--env PI_CODING_AGENT_DIR --env TMPDIR`, and a test fails if either is missing.

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
cell to pass; the live proof expects the blocked cells listed above.

## Why the blocked cells are blocked

- **hermes** runs the profile's Pi-route models: `--harness hermes` takes the cheapest model the profile
  leaves on a Pi route for any data (`pi-vibe`, `pi-vibe-study`, `pi-opencode-go`, `pi-local`, never the
  public-only or free-only routes) and runs it as `hermes -m <model> --provider litellm-<route>`, the provider
  setup writes into `~/.hermes/config.yaml`. The profile check holds the run to that Pi entry, and every role
  names hermes in `unenforced_clients`, since the gateway cannot enforce a tool scope on it.
- **vibe** is explicit only: the Vibe CLI spends the owner's Mistral account, so no profile lists a `vibe:` model
  and `--harness vibe` finds none. A run that names `"client": "vibe"` and a model still runs. The Vibe plans stay
  reachable through Pi as `pi-vibe` and `pi-vibe-study`.
- **agy** passes on a machine where it is signed in. On one where it is not, its runs fail with the sign-in
  error, which the proof reports as blocked (and, as the baseline expects a pass, as a regression to resolve by
  signing in).
- **codex** is metered-employer: it is reached only for a repository whose allowlist names `openai`, and a
  grant for the `harness-matrix` workflow must exist (`model-gateway grant write`). Pass a `repo` that has
  both to run the codex column live.
