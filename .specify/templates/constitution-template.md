# Project Constitution

## Core Principles

### I. Outcome-First Specifications

Every feature begins with a specification that describes user-visible outcomes,
acceptance scenarios, non-goals, and success criteria before implementation
details. Ambiguity must be marked explicitly with `NEEDS CLARIFICATION`.

### II. Plan Before Implementation

Implementation work starts only after the plan identifies real project paths,
dependencies, validation commands, rollback considerations, and risks. Plans
must prefer established local patterns over new abstractions.

### III. Tests and Validation Are Mandatory

Each feature defines the smallest meaningful verification command before work
begins. Changes are not complete until those checks pass or the remaining gap is
documented with the exact reason validation could not run.

### IV. Small, Reviewable Changes

Tasks and PRs must be independently reviewable, revertable, and scoped to one
behavioral objective. Unrelated cleanup, broad refactors, and speculative
flexibility are not allowed inside feature work.

### V. Durable Context Stays Current

Specifications, plans, tasks, and durable project memory must reflect decisions
that affect future work. Do not leave important behavior only in chat logs,
temporary notes, or uncommitted local state.

## Workflow

1. `/speckit.specify` creates or updates `specs/<feature>/spec.md`.
2. `/speckit.plan` creates `plan.md` and supporting design artifacts.
3. `/speckit.tasks` creates `tasks.md` from the approved plan.
4. Implementation follows tasks in dependency order, with tests close to the
   behavior being changed.
5. Completion requires validation evidence and any relevant documentation
   updates.

## Governance

This constitution overrides informal conventions. Changes to these principles
must be reviewed deliberately, with downstream templates and instructions
updated in the same change.

**Version**: 1.0.0
**Ratified**: {{DATE}}
**Last Amended**: {{DATE}}
