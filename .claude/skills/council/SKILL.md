---
name: council
description: Orchestrate a large, decomposable problem across two model families — Claude and Codex plan it independently, cross-critique for two rounds, a judge consolidates one plan plus a parallel task DAG, then cheap workers fan out execution in isolated worktrees. Use for big parallelizable work; NOT for small or tightly-coupled/sequential changes (use a normal session for those).
---

# council

A cross-model orchestrator for problems too big for one pass. The driver is
`~/.claude/skills/council/council.py` (stdlib-only; shells out to `claude -p` and
`codex exec`); inside the personal-stack repo it also lives at
`platform/agents/council/council.py`. Your job around it is the human-facing
part: clarify the brief and present the two checkpoints.

**Run it from the project you want council to work on** — it targets the
repository of the current working directory, so a globally installed council
orchestrates whatever repo you're in. It installs into `~/.claude/skills/council`
(and `~/.codex/skills/council`) via the agent-kit installer
(`curl …/install.sh | bash`); re-run that to upgrade.

## When to use / when not

- **Use it** when the work is large, decomposes into independent parallel
  pieces, and is worth the spend (multi-agent runs roughly 15x the tokens of a
  single chat).
- **Do not use it** for small changes, or for tightly-coupled / sequential work
  where every step depends on the last — multi-agent *hurts* there. Just do
  those in a normal session.

## The loop

1. **Clarify (you → user).** Read the request. If scope, constraints,
   definition-of-done, or the repo area are ambiguous, ask 2-3 targeted
   questions with `AskUserQuestion`. Skip if already clear. Write the result to
   a `brief.md`.
2. **Plan (council.py).** Run the `plan` phase — two independent plans, two
   cross-critique rounds, one consolidation:
   ```bash
   python3 ~/.claude/skills/council/council.py plan --brief brief.md
   ```
   It prints the run dir. (`--estimate` first if you want the call count.)
3. **Checkpoint 1 (you → user).** Show the user `consolidated_plan.md` and the
   `tasks.json` DAG from the run dir. Let them approve or edit before any fan-out
   spend. This is the cheap gate before the expensive wave.
4. **Fan out (council.py).** On approval:
   ```bash
   python3 ~/.claude/skills/council/council.py fanout --run <run-dir>
   ```
   Cheap workers execute the DAG in isolated worktrees; results land on a
   `council/<run>/integration` branch. Your working branch is untouched.
5. **Checkpoint 2 (you → user).** Present `report.md`: what merged, what failed,
   the integration branch to review. Collect feedback; re-loop into `plan` or
   re-run specific tasks as needed.

## Controlling models & intensity

Council is driven by an **intensity preset** plus optional **per-role
overrides**, stored in `platform/agents/council/council.toml`. Manage it
conversationally — when the user says "use thorough intensity" or "switch the
planners to sonnet", translate to a flag or a `config` command:

| preset | rounds | codex effort | workers | max workers |
|---|---|---|---|---|
| `quick` | 1 | low | haiku | 4 |
| `standard` (default) | 2 | high | haiku | 6 |
| `thorough` | 3 | high | sonnet | 6 |
| `max` | 3 | xhigh | sonnet | 8 |

```bash
# one-off for this run:
python3 ~/.claude/skills/council/council.py plan --brief brief.md --intensity thorough
python3 ~/.claude/skills/council/council.py fanout --run <dir> --worker claude:sonnet

# persist defaults (edits council.toml):
python3 ~/.claude/skills/council/council.py config show
python3 ~/.claude/skills/council/council.py config set intensity thorough
python3 ~/.claude/skills/council/council.py config set planner_b codex:gpt-5.5
python3 ~/.claude/skills/council/council.py config unset worker
```

Per-role override flags: `--intensity`, `--rounds`, `--planner-a`, `--planner-b`,
`--consolidator`, `--worker`, `--verifier`, `--codex-effort`, `--max-workers`.
Precedence: intensity preset < `council.toml` < CLI flag. Every role — workers
included — can be `claude:<model>` or `codex:<model>`, so a run can mix engines.

## Fleet — ad-hoc, engine-agnostic worker pool

`fleet` runs an existing task DAG against a declared pool of agents on either
CLI, skipping the plan phase. Point it at any `tasks.json`, give it a pool, and
tasks are round-robined across the pool, then verified and reconciled exactly
like `fanout` (isolated worktrees, integration branch, your branch untouched).

```bash
python3 ~/.claude/skills/council/council.py fleet \
  --tasks tasks.json --agents 'codex:gpt-5.5*3,claude:haiku*2'
python3 ~/.claude/skills/council/council.py fleet \
  --tasks tasks.json --agents 'claude:haiku*4' --estimate
```

The `--agents` spec is comma-separated `cli:model[*count]` entries (count
defaults to 1). Use it to drive a mixed Claude+Codex fleet over a big,
already-decomposed batch — e.g. cross-agent cleanups managed from either CLI.

## split — extract a subtree into a new repo

`split` carves a path out of the current repo into a brand-new GitHub repo with
that path's history preserved (`git subtree split`). It works on a throwaway
`council/split/<name>` branch and never touches your working branch.

```bash
# preview the exact commands, touch nothing:
python3 ~/.claude/skills/council/council.py split \
  --path services/foo --dest myorg/foo --dry-run
# extract, create the (private) remote, push:
python3 ~/.claude/skills/council/council.py split --path services/foo --dest myorg/foo
# extract to a local branch only; push it yourself later:
python3 ~/.claude/skills/council/council.py split --path services/foo --dest myorg/foo --no-push
```

`--visibility public` makes the new repo public. After extracting, it prints how
to optionally replace the in-repo copy with a submodule in a separate change.
The GitHub App must be installed on the destination owner for the push to
authenticate.

## Notes

- Both `claude` and `codex` CLIs must be authenticated.
- Runs are resumable: stages are idempotent on their output files; re-run with
  `--run <dir>` to continue.
- Full design and rationale: `docs/private/council-orchestrator-design.md`.
