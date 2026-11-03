---
name: council-design
description: Produce or refine a council-ready design, consolidated plan, and parallel task DAG. Use when Claude needs to turn a clear brief into bounded chunks with dependencies, validation, and model assignments before execution.
---

# Council Design

Use `design` when the brief is clear enough to plan but execution has not been
approved. The output should be a reviewable plan and DAG, not file edits.

## Chunking Rules

- Chunk by ownership boundary, feature slice, or independently verifiable task.
- Keep task prompts self-contained: objective, files, constraints, tests, and
  expected output.
- Make dependencies explicit; no worker should need another worker's unstated
  context.
- Prefer fewer, larger coherent chunks over many tiny tasks that create merge
  overhead.

## Lens-Building Rules

Build the design lens before drafting tasks:

- User outcome and non-goals.
- Repository areas and existing patterns to inspect.
- Risk and validation requirements per area.
- Parallelism boundaries and dependency order.
- Integration strategy and rollback or amendment path.

## Model Preference Doctrine

- Prefer stronger reasoning for DAG shape, dependency calls, and validation
  design.
- Prefer Codex high or xhigh for codebase-heavy decomposition.
- Prefer Claude sonnet or opus for product, architecture, and task clarity.
- Use cheap workers only after the plan is approved and chunks are bounded.

## Interop Surfaces

- Primary command shape: `node ~/.claude/skills/council/council.mjs design ...`.
- Use `council-grill` to challenge the design before `council-run`.
- Use `codex-xhigh` for an independent code-focused design pass when needed.
- Return a concise plan, DAG tasks, dependencies, validation commands, and open
  questions.
