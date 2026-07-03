---
name: council-supervise
description: Supervise a single cross-CLI delegate with bounded scope and progress relay. Use when Codex needs one Codex or Claude worker for a scoped task without launching a full council plan or fan-out.
---

# Council Supervise

Use `supervise` for one bounded delegate. It is lighter than a full council run
but still preserves scope, progress, and result capture.

## Chunking Rules

- One supervise call equals one objective with explicit boundaries.
- Include files allowed, files forbidden, commands allowed, and verification.
- Keep dependent follow-up work in the host session until the result returns.
- Do not use supervise for broad ambiguous projects; triage or design first.

## Lens-Building Rules

Build a delegate lens:

- Role: implementer, reviewer, debugger, summarizer, or verifier.
- Objective and non-goals.
- Evidence to inspect before acting.
- Boundaries and validation.
- Required final response format.

## Model Preference Doctrine

- Use the host Codex session for small or tightly-coupled code work.
- Use `claude-worker` for an independent Claude pass on bounded work.
- Use stronger engines only when the lens explains why the cheap worker is
  likely insufficient.
- Prefer cross-family delegation when disagreement or independent synthesis is
  more valuable than raw throughput.

## Interop Surfaces

- Primary command shape: `node ~/.codex/skills/council/council.mjs supervise --engine ...`.
- Codex-side delegate skill: `claude-worker`.
- Preserve quoting and newlines when forwarding the prompt.
- Relay run id, phase transitions, validation output, final result, changed
  files, and caveats to the host session.
