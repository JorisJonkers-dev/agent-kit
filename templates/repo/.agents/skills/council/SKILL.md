---
name: council
description: Orchestrate a large, decomposable problem across two model families — Codex and Claude plan it independently, cross-critique for two rounds, a judge consolidates one plan plus a parallel task DAG, then cheap workers fan out execution in isolated worktrees. Use for big parallelizable work, not for small or tightly-coupled sequential changes.
---

# council

A cross-model orchestrator for problems too big for one pass. The driver is
`~/.codex/skills/council/council.py` (stdlib-only; shells out to `codex exec`
and `claude -p`); inside the personal-stack repo it also lives at
`platform/agents/council/council.py`. Around it, you handle the human-facing
part: clarify the brief and present the two checkpoints.

Run it from the project you want council to work on — it targets the repository
of the current working directory. It installs into `~/.codex/skills/council`
(and `~/.claude/skills/council`) via the agent-kit installer
(`curl …/install.sh | bash`); re-run that to upgrade.

Use it for large work that decomposes into independent parallel pieces and is
worth the spend (multi-agent uses roughly 15x the tokens of a single chat). Do
not use it for small or tightly-coupled, sequential work — handle those in a
normal session.

Loop:

1. Clarify. If scope, constraints, definition of done, or repo area are
   ambiguous, ask the user 2-3 targeted questions; otherwise skip. Write the
   result to `brief.md`.
2. Plan:
   ```bash
   python3 ~/.codex/skills/council/council.py plan --brief brief.md
   ```
   Two independent plans, two cross-critique rounds, one consolidation. Prints
   the run dir. Use `--estimate` for the call count first.
3. Checkpoint 1: show the user `consolidated_plan.md` and the `tasks.json` DAG
   from the run dir. Get approval or edits before any fan-out spend.
4. Fan out, on approval:
   ```bash
   python3 ~/.codex/skills/council/council.py fanout --run <run-dir>
   ```
   Cheap workers execute the DAG in isolated worktrees; results land on a
   `council/<run>/integration` branch. The working branch is untouched.
5. Checkpoint 2: present `report.md` — what merged, what failed, the integration
   branch to review. Collect feedback; re-loop into `plan` or re-run tasks.

## Controlling models & intensity

Council is driven by an intensity preset plus optional per-role overrides in
`platform/agents/council/council.toml`. Manage it conversationally — translate
"use thorough intensity" or "switch planners to sonnet" into a flag or a
`config` command.

| preset | rounds | codex effort | workers | max workers |
|---|---|---|---|---|
| quick | 1 | low | haiku | 4 |
| standard (default) | 2 | high | haiku | 6 |
| thorough | 3 | high | sonnet | 6 |
| max | 3 | xhigh | sonnet | 8 |

```bash
# one-off:
python3 ~/.codex/skills/council/council.py plan --brief brief.md --intensity thorough
python3 ~/.codex/skills/council/council.py fanout --run <dir> --worker claude:sonnet
# persist (edits council.toml):
python3 ~/.codex/skills/council/council.py config show
python3 ~/.codex/skills/council/council.py config set intensity thorough
python3 ~/.codex/skills/council/council.py config set planner_b codex:gpt-5.5
```

Override flags: `--intensity`, `--rounds`, `--planner-a`, `--planner-b`,
`--consolidator`, `--worker`, `--verifier`, `--codex-effort`, `--max-workers`.
Precedence: preset < council.toml < CLI flag. Every role, workers included, can
be `codex:<model>` or `claude:<model>`, so a run can mix engines.

## Fleet — ad-hoc, engine-agnostic worker pool

`fleet` runs an existing task DAG against a declared pool of agents on either
CLI, with no plan phase. Point it at any `tasks.json` and a pool; tasks are
round-robined across the pool, then verified and reconciled like `fanout`.

```bash
python3 ~/.codex/skills/council/council.py fleet \
  --tasks tasks.json --agents 'codex:gpt-5.5*3,claude:haiku*2'
python3 ~/.codex/skills/council/council.py fleet \
  --tasks tasks.json --agents 'codex:gpt-5.5*4' --estimate
```

The `--agents` spec is comma-separated `cli:model[*count]` entries (count
defaults to 1) — a mixed Codex+Claude fleet over an already-decomposed batch.

## split — extract a subtree into a new repo

`split` carves a path out of the current repo into a new GitHub repo with that
path's history preserved (`git subtree split`), on a throwaway
`council/split/<name>` branch; the working branch is untouched.

```bash
python3 ~/.codex/skills/council/council.py split \
  --path services/foo --dest myorg/foo --dry-run
python3 ~/.codex/skills/council/council.py split --path services/foo --dest myorg/foo
```

`--no-push` stops at the local branch; `--visibility public` for a public repo.
The GitHub App must be installed on the destination owner for the push.

Notes:

- Both `codex` and `claude` CLIs must be authenticated.
- Runs are resumable: stages are idempotent on their output files; re-run with
  `--run <dir>`.
- Full design: `docs/private/council-orchestrator-design.md`.
