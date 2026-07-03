# Constitution Digest

source: templates/repo/.specify/memory/constitution.md; README.md; PORTABILITY.md
built-at: 2026-07-02T22:50:34Z

This fragment is tracked seed content. Treat it as seed-if-absent and preserve
local edits during regeneration.

## Core Rules

- Human authorship only: no assistant, model, automation, generated-by, or
  co-author attribution in commits, PRs, code, comments, docs, generated files,
  or templates.
- Validate against real repo paths, APIs, config, tooling, and live state when
  relevant; unknown facts must be checked before being used.
- Keep Claude and Codex behavior equivalent across skills, hooks, memory rules,
  installer behavior, commands, and project guidance unless an explicit
  unsupported reason is recorded.
- Edit render-managed files at their source templates or inventory, then run
  the owning renderer and commit generated output with the source change.
- Keep changes scoped to one objective and avoid tangential cleanup, speculative
  abstractions, unrelated refactors, and unnecessary compatibility shims.

## Workflow Expectations

- Start user-visible or cross-cutting changes from a spec with outcomes,
  acceptance criteria, non-goals, and open questions.
- Plan against existing repo patterns and real paths.
- Break work into tasks that preserve small PR boundaries and parallel safety.
- Implement only the assigned task scope and never overwrite unrelated parallel
  edits.
- Run the smallest meaningful validation command for the touched area and state
  any remaining unverified work.

## Governance

The checked-in constitution seed is version `1.0.0`, ratified and last amended
on `2026-06-08`. It overrides ad-hoc agent behavior and should be amended
deliberately when governing workflow changes.
