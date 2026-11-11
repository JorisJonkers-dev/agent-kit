# The Gold Band headless proof

Every [Gold Band workflow](../CONTEXT.md) checked into
`installer/gold-band/workflows/` is run headless against a stub agent before
any real run, and a real run is refused when that proof is missing, skipped or
failed (spec #392 user stories 72 and 79; issue #447). The harness is the
`gold-band-proof` crate in the cargo workspace (std plus `serde_json`).

## What a proof does

`gold-band-proof prove --workflow <FILE>` builds a throw-away Gold Band home in
a temporary directory:

* a `settings.json` that declares **every agent id the workflow names** (node
  `provider`s and `modelBindings` agent ids) with the harness itself, run as
  `gold-band-proof stub-acp`, as its command. The stub is a small ACP agent
  that answers every prompt with a fixed findings document, so no model and no
  network is spent;
* a minimal profile file for each profile the workflow's nodes name (Gold Band
  refuses a node whose profile is missing);
* a workspace that is a git repository with one empty commit and a
  `requirement.md`.

It then runs `gold-band run --workflow <FILE> --requirement-file requirement.md`
once per scenario and compares the exit code with what the scenario expects. It
also checks that the printed run state agrees with the exit code (Gold Band's
mapping: completed and success is 0, completed otherwise 1, paused 2).

| Scenario | Stub verdict | Expected exit | Added when |
| --- | --- | --- | --- |
| `clean` | empty findings | 0 | always |
| `needs-human` | one major finding | 2 (paused at the manual check) | the workflow has a `manual_check` node |

More scenarios come from `--scenario NAME:VERDICT:EXIT` (verdicts: `clean`,
`needs-human`, `fail`; `fail` makes the stub answer a prompt with a protocol
error, which Gold Band pauses as `runtime-abnormal`).

## The proof record

`prove` writes `<record-dir>/<workflow id>.proof.json` (default record dir
`$XDG_STATE_HOME/agent-kit/gold-band/proofs`, else
`~/.local/state/agent-kit/gold-band/proofs`): the workflow id and the SHA-256
of its file, the Gold Band commit (from `--gold-band-commit`, else the build
record setup writes at `~/.local/state/agent-kit/gold-band/build.json`), each
scenario's expected and actual exit code and run state, and an outcome of
`passed`, `failed` or `skipped`.

Without a Gold Band binary (`--gold-band` or `GOLD_BAND_BIN`) `prove` does not
fail: it prints `gold-band-proof: skipped: no Gold Band binary supplied ...`,
records the proof as `skipped` with that reason, and exits 0. A skipped proof
is never a passing one.

Exit status of `prove`: `0` passed or skipped, `1` a scenario did not behave as
expected, `2` bad usage or an unreadable workflow.

## Refusing an unproven run

`gold-band-proof run --workflow <FILE> -- <gold-band run arguments>` is how a
real run of a checked-in workflow starts. It first checks the record and
refuses, with exit **5** and the reason on stderr, when:

1. no record exists for the workflow;
2. the proof was skipped (the recorded reason is printed);
3. the proof failed (the first scenario that did not pass is named);
4. the workflow file changed since the proof (hash differs);
5. the installed Gold Band build (the build record's commit) is not the one the
   proof ran on;
6. the record claims a pass but holds no scenarios or a failing one.

Otherwise it runs `gold-band run --workflow <FILE> <arguments>` and returns
Gold Band's own exit code. `gold-band-proof check --workflow <FILE>` is the same
check without starting a run (exit 0 or 5).

## CI

The `Gold Band Headless Proof` job runs on Linux and macOS. It builds the
harness, fetches the Gold Band binary the repository supplies (repository
variables `GOLD_BAND_BIN_URL_LINUX` / `GOLD_BAND_BIN_URL_MACOS` with the
matching `..._SHA256_...`), and runs `prove` over every workflow in
`installer/gold-band/workflows/`. When no binary is supplied for an OS the job
prints the skip reason and passes without proving anything.

## Not covered

The stub answers with a findings document, so the proof checks structure and
routing (the spec-stage template). A workflow whose nodes need other output
(for example Workflow B's gates) adds its own scenarios and, when the findings
shape does not fit, its own stub reply in the ticket that ports it.
