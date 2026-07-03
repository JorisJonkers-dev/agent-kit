---
name: council-run
description: Execute an approved council plan or task DAG through supervised workers. Use when Claude has an accepted design and needs to fan out bounded work, monitor progress, merge results, and report verification outcomes.
---

# Council Run

Use `run` only after the plan or task DAG has been approved. This skill is for
execution control, progress relay, and result reporting.

## Chunking Rules

- Run only tasks with explicit ids, boundaries, dependencies, and validation.
- Execute independent tasks in parallel waves; keep dependent tasks sequential.
- Do not split or broaden approved tasks during execution; amend the plan first.
- Stop a worker that crosses boundaries or needs missing context.

## Lens-Building Rules

Build an execution lens for each wave:

- Task ids and dependency state.
- Allowed files and commands.
- Expected artifacts and validation commands.
- Merge or integration expectations.
- Failure policy: retry, amend, or escalate.

## Model Preference Doctrine

- Use cheap workers for bounded implementation tasks.
- Use Codex medium for routine code changes and Codex xhigh for complex code or
  hard debugging.
- Use Claude haiku for simple text, summaries, and scoped edits; use sonnet for
  cross-file behavior or tests.
- Keep final verification on a model strong enough to catch integration errors.

## Interop Surfaces

- Primary command shape: `node ~/.claude/skills/council/council.mjs run ...`.
- Use `codex-med` or `codex-xhigh` as supervised worker wrappers from Claude.
- Relay run ids, active wave, worker status, failures, validation output, and
  final report.
- If the DAG must change, pause and route to `council-amend`.
