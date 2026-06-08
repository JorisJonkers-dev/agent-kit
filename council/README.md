# council

Cross-model planning + fan-out orchestrator. Two different model families
(Claude + Codex) plan a problem independently, critique each other's plan for two
rounds, a single judge consolidates one plan plus a parallel task DAG, then cheap
worker agents execute the DAG in isolated git worktrees. The human is consulted
at two checkpoints: the clarified brief, and the consolidated plan before fan-out.

This script is the engine; the `council` skill (`.claude` / `.agents`) drives the
human-facing steps around it. Use it for **large, decomposable, parallelizable**
work only — multi-agent runs roughly 15x the tokens of a single chat and *hurts*
small or tightly-coupled tasks.

## Why this shape
Cross-*model* critique beats self-critique (different families make different
mistakes, and a model over-rates its own output); gains are front-loaded so two
rounds is the sweet spot; consolidation synthesises rather than votes; expensive
models plan/critique/judge while cheap models fan out. Full rationale and sources:
`docs/private/council-orchestrator-design.md`.

## Usage

```bash
# stages 1-4: produce a consolidated plan + tasks.json (no execution)
python3 platform/agents/council/council.py plan --brief brief.md

# preview cost (no model calls)
python3 platform/agents/council/council.py plan --estimate

# resume an interrupted run (stages are idempotent on their output files)
python3 platform/agents/council/council.py plan --brief brief.md --run .council/runs/<id>

# pure-function checks, no model calls
python3 platform/agents/council/council.py --self-test
```

`plan` prints the run dir on stdout. Inspect `consolidated_plan.md` and
`tasks.json` there.

```bash
# stages 5-6: execute the task DAG with cheap workers, verify, reconcile
python3 platform/agents/council/council.py fanout --run .council/runs/<id>

# preview the wave/worker plan (no execution)
python3 platform/agents/council/council.py fanout --run .council/runs/<id> --estimate
```

`fanout` topologically sorts `tasks.json` into waves. Within a wave, workers run
concurrently — each in its own `git worktree` off the integration branch — so
they never collide. Each worker (cheap model) implements its task; the
orchestrator commits the worktree, runs the task's `verify`, and an adversarial
verifier (`claude:sonnet`) checks the diff against the objective. Worker commits
are merged onto a `council/<run>/integration` branch in dependency order;
conflicts are left out and reported. **Nothing is merged into your branch** —
`fanout` prints the integration branch name for review. See `report.md` in the
run dir.

```bash
# ad-hoc, engine-agnostic worker pool over any tasks.json (no plan phase)
python3 platform/agents/council/council.py fleet \
  --tasks tasks.json --agents 'codex:gpt-5.5*3,claude:haiku*2'
```

`fleet` round-robins the tasks across the declared pool — workers may be Claude
or Codex — and runs them through the same worktree/verify/reconcile path as
`fanout`. Use it to drive a mixed fleet over an already-decomposed batch.

## split — extract a subtree into a new repo

```bash
# preview only:
python3 platform/agents/council/council.py split \
  --path services/foo --dest myorg/foo --dry-run
# extract (history preserved), create the remote, push:
python3 platform/agents/council/council.py split --path services/foo --dest myorg/foo
```

`split` runs `git subtree split` on a throwaway `council/split/<name>` branch so
the path's history is preserved, creates the destination repo with `gh` (skipped
if it already exists), and pushes. The working branch is untouched. `--no-push`
stops at the local branch; `--visibility public` makes the new repo public. The
GitHub App must be installed on the destination owner for the push to
authenticate.

## Models & intensity

Driven by an intensity preset plus optional per-role overrides in `council.toml`.

| preset | rounds | codex effort | workers | max workers |
|---|---|---|---|---|
| `quick` | 1 | low | haiku | 4 |
| `standard` (default) | 2 | high | haiku | 6 |
| `thorough` | 3 | high | sonnet | 6 |
| `max` | 3 | xhigh | sonnet | 8 |

```bash
council.py plan --brief b.md --intensity thorough     # one-off
council.py config show                                # resolved config + sources
council.py config set intensity thorough              # persist (user-global)
council.py config set planner_b codex:gpt-5.5
council.py config set worker claude:sonnet --project  # persist for THIS repo only
council.py config unset worker                         # back to preset
```

Override flags: `--intensity`, `--rounds`, `--planner-a`, `--planner-b`,
`--consolidator`, `--worker`, `--verifier`, `--codex-effort`, `--max-workers`.
Workers must be `claude:<model>` (codex workers aren't supported yet).

**Config layering** (each overrides the previous):
1. intensity preset
2. user-global `council.toml` (next to the driver — the committed default
   in-repo, or `~/.claude/skills/council/council.toml` when installed globally)
3. per-project `./.council.toml` in the repo you run council from
4. CLI flags

`config set` writes the user-global file by default; `--project` writes the
repo's `./.council.toml`.

## Portability

council operates on **the repository of your current working directory** — run
it from inside whatever project you want it to orchestrate (it resolves the repo
via `git rev-parse --show-toplevel` of the CWD, not the toolkit's own location).
This is what lets a globally installed council work in any project.

## Run layout (`.council/runs/<id>/`, gitignored)
```
brief.md
planA.v1.json planB.v1.json            # stage 1
critique-of-A.r1.md planA.v2.json ...  # stage 2 (per round)
consolidated_plan.md tasks.json        # stage 4
state.json                             # resume marker
```

## Notes
- Sub-invocations run with `KB_AUTO_MCP_DISABLED=1` so council's internal prompts
  never pollute the knowledge base.
- Codex planning/critique uses `model_reasoning_effort=high`, read-only sandbox.
- Both `claude` and `codex` CLIs must be installed and authenticated.
- We don't bundle a full JSON-Schema validator; `validate_tasks` checks the
  fields fan-out relies on and runs a topological sort (cycle / unknown-dep
  detection). The JSON schemas under `schemas/` document the contract.
