---
name: council
description: Orchestrate a large, decomposable problem across Codex and Claude with triage, cross-model planning, grill, design checkpoint, guarded fan-out, review council, and resumable status/tail. Use for big parallelizable work, not for small or tightly-coupled sequential changes.
---

# council

A cross-model orchestrator for problems too big for one pass. The current driver
is the installed TypeScript CLI, invoked through the `council` launcher or, if
the launcher is not on `PATH`, the skill-local CLI under
`~/.codex/skills/council`. It shells out to `codex exec` and `claude -p` and
targets the repository of the current working directory.

Your job around it is still the human-facing part: clarify the brief, route
small work away from council, present checkpoints, and keep the user in control
of expensive fan-out.

Run it from the project you want council to work on. It installs into
`~/.codex/skills/council` (and `~/.claude/skills/council`) via the agent-kit
installer (`curl .../install.sh | bash`); re-run that to upgrade.

Use it for large work that decomposes into independent parallel pieces and is
worth the spend. Do not use it for small changes, tightly-coupled changes, or
work where every step depends on the last.

## The Loop

1. Clarify. If scope, constraints, definition of done, or repo area are
   ambiguous, ask the user 2-3 targeted questions; otherwise skip. Write the
   result to `brief.md`.
2. Start council with the TypeScript CLI:
   ```bash
   council run --brief brief.md
   ```
   If the shell shim is unavailable, use the installed skill-local CLI:
   ```bash
   node ~/.codex/skills/council/council.mjs run --brief brief.md
   ```
   Use `--estimate` before a run when the user wants the call count and rough
   cost envelope.
3. Triage routing. Council first classifies the brief as `normal-session`,
   `plan-only`, `fleet`, or `full-council`. If it routes to `normal-session`,
   stop the run and handle the task in the current chat. If it routes to
   `plan-only`, present the design without fan-out unless the user asks to
   continue.
4. Context packs. Before model calls, council builds bounded context packs for
   each role: brief, relevant files, repo rules, prior artifacts, task
   boundaries, and review-pack requirements. Treat those packs as the source of
   truth for what each sub-agent was allowed to use.
5. Plan, critique, and grill. Independent Codex and Claude planners produce
   plans, cross-critique rounds refine them, and the grill step stress-tests the
   design for missing dependencies, unsafe parallelism, unclear ownership,
   validation gaps, and rollback/recovery risk.
6. Design checkpoint. Council stops before fan-out with `design.md`,
   `grill.md`, `consolidated_plan.md`, and `tasks.json`. Show the user the
   recommended route, risks, task DAG, affected areas, expected spend, and the
   command printed by the CLI to resume. Do not fan out without approval.
7. Fan out on approval. Workers execute DAG tasks in isolated worktrees. The
   watchdog monitors stalled workers, missing heartbeats, out-of-bounds edits,
   and verification timeouts, then marks tasks for retry or review instead of
   silently continuing.
8. Monitor progress:
   ```bash
   council status --run <run-id>
   council tail --run <run-id> --follow
   ```
   `status` is for a compact run summary; `tail` is for live worker, watchdog,
   review, and GitHub-mirror events.
9. Review-pack gate. Every completed task must produce a review pack with the
   objective, allowed files, changed files, diff summary, verification output,
   residual risks, and follow-up notes. Tasks without a valid review pack do not
   pass the gate.
10. Review council. Reviewers inspect the review packs and diffs, then return
    `accept`, `fix`, or `reject`. Accepted work is reconciled onto the
    integration branch; rejected or fix-required tasks stay out and are reported
    with their reason.
11. GitHub mirroring. When enabled, council mirrors run state, task status,
    review packs, and review-council outcomes to GitHub issues, PR comments, or
    checks. The local run directory remains canonical; GitHub is a visibility
    mirror, not the source of truth.
12. Final checkpoint. Present `report.md`: what merged, what failed, which
    review packs were accepted, which tasks need another pass, and the
    integration branch to inspect. Re-loop into design, retry selected tasks, or
    close the council run.

## Commands

```bash
council run --brief brief.md --preset standard
council run --brief brief.md --estimate
council resume --run <run-id>
council status --run <run-id>
council tail --run <run-id> --follow
council config show
council config set preset thorough
```

`run` owns the normal triage -> design checkpoint -> fan-out -> review council
loop. `resume` continues from the latest checkpoint or interrupted stage. Use
the exact resume command printed by the CLI when a run stops at a gate.

## Config Knobs

Council is driven by a preset plus optional per-role and per-gate overrides in
`council.toml` and project-local `./.council.toml`. Precedence is:
preset defaults < user `council.toml` < project `./.council.toml` < CLI flags.

| preset | rounds | codex effort | worker | max workers | review council |
|---|---:|---|---|---:|---|
| `quick` | 1 | low | claude:haiku | 4 | single reviewer |
| `standard` (default) | 2 | high | claude:haiku | 6 | two reviewers |
| `thorough` | 3 | high | claude:sonnet | 6 | two reviewers + grill retry |
| `max` | 3 | xhigh | claude:sonnet | 8 | three reviewers + grill retry |

Core role knobs:
`triager`, `planner_a`, `planner_b`, `critic_a`, `critic_b`, `griller`,
`designer`, `worker`, `watchdog`, `reviewer`, `verifier`.

Execution knobs:
`preset`, `rounds`, `codex_effort`, `max_workers`, `worker_timeout_s`,
`watchdog_interval_s`, `retry_limit`, `status_tail_lines`.

Gate and visibility knobs:
`design_checkpoint`, `require_context_pack`, `require_review_pack`,
`review_council`, `github_mirror`, `github_mirror_target`.

Context-pack knobs:
`context_pack_budget`, `context_pack_max_files`, `context_pack_include_tests`,
`review_pack_max_bytes`.

Unset role and gate keys inherit from the selected preset. Keep GitHub mirroring
off unless the user asks for it or the repository convention requires it.

## Fleet

Use `fleet` when the task DAG already exists and no plan/grill/design phase is
needed:

```bash
council fleet --tasks tasks.json --agents 'codex:gpt-5.5*3,claude:haiku*2'
council fleet --tasks tasks.json --agents 'codex:gpt-5.5*4' --estimate
```

The same context-pack, watchdog, review-pack, review-council, and GitHub mirror
gates apply.

## split

`split` carves a path out of the current repo into a new GitHub repo with that
path's history preserved, on a throwaway branch; the working branch is
untouched.

```bash
council split --path services/foo --dest myorg/foo --dry-run
council split --path services/foo --dest myorg/foo
```

`--no-push` stops at the local branch; `--visibility public` creates a public
repo. The GitHub App must be installed on the destination owner for the push.

## Phase 2 Roadmap

- Council board: persistent board view for runs, lanes, task state, blockers,
  review decisions, and integration status.
- ACP adapter: expose the same run, status, tail, checkpoint, and review-pack
  workflow through the Agent Client Protocol.
- Self-hosted engine profile: preset family for local or privately hosted model
  endpoints, with the same role, watchdog, and gate semantics.

## Notes

- Both `codex` and `claude` CLIs must be installed and authenticated.
- Runs are resumable; use `council status` to find the latest gate and `council
  resume` to continue.
- Do not edit worker worktrees by hand. Feed fixes back through retry, review
  council, or a new council run.
