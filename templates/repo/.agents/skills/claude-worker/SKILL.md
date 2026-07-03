---
name: claude-worker
description: Delegate bounded work from Codex to a supervised Claude worker through council.mjs, especially for cross-family review, implementation, summarization, or independent second-pass reasoning where progress must be relayed back to the Codex host.
---

# Claude Worker

Use this as the Codex-side symmetric delegate for work that should be performed
by a supervised Claude worker instead of the current Codex session.

Resolve `<installed-path>` to the installed council directory, normally
`~/.codex/skills/council`, then run from the target repository:

```bash
node <installed-path>/council.mjs supervise --engine claude:haiku ...
```

Use a stronger Claude model only when the task is too ambiguous, high-risk, or
architecture-heavy for a cheap worker.

## Chunking Rules

- Delegate one bounded objective at a time.
- Include allowed files, forbidden files, validation commands, and definition of
  done in the task prompt.
- Split broad work by ownership boundary, feature slice, or independent DAG node.
- Keep dependent work in the host session or run it sequentially after the
  prerequisite result lands.

## Lens-Building Rules

Before launching the worker, write the lens into the prompt:

- Objective: what outcome the worker must produce.
- Scope: files, commands, and repo area the worker may touch.
- Exclusions: what must not be changed or decided.
- Evidence: artifacts the worker must inspect before acting.
- Output: exact summary, changed-file list, tests, and caveats expected.

## Model Preference Doctrine

- Prefer `claude:haiku` for isolated edits, summaries, and mechanical checks.
- Use `claude:sonnet` for multi-file reasoning, user-facing behavior, or tests.
- Use `claude:opus` only for expensive design or final judgment that benefits
  from broad synthesis.
- Prefer cross-family delegation when an independent read is more valuable than
  another pass by the same host model.

## Interop Surfaces

- Primary command: `node <installed-path>/council.mjs supervise --engine ...`.
- Preserve quoting and newlines when forwarding the prompt.
- Relay run ids, phase changes, blocked states, validation output, and final
  changed files back to the Codex host.
- Do not bypass the supervisor for file-writing work; it owns boundaries,
  progress, and result capture.
