# Refactoring plans are their own critique stage, ahead of the spec

A refactor feature is planned as a **refactoring plan**: a goal, a target
structure and ordered steps, each of one kind (characterize, tidy, structure,
behaviour, expand, migrate, contract, remove-transitional), each with its
safety net, rollback and blockers. The plan is critiqued at its own critique
stage, `/critique-plan`, which takes the place of the decisions stage for a
refactor: after `/improve-codebase-architecture` has grilled a candidate, the
stage drafts the plan when none exists, critics check it, and only a plan
with no open decisions goes on to `/to-spec`. The owner starts the first
`/critique-plan` run; the plan file it drafts marks the draft as a refactor
from then on. The file stays local until `/to-spec`: under
`.scratch/<feature>/` in a repository with a local-markdown tracker,
otherwise beside the flow state in the git common directory, never committed.

We rejected treating the plan as a spec with extra lenses at the spec stage.
That would have needed no new stage, but a plan's defects (a step that mixes
structure and behaviour, a step with no rollback, an order that leaves main
broken) are cheapest to fix before the spec and tickets are cut from it. The
plan's step graph is also the ticket graph `/to-tickets` will produce, so
checking it first means checking the tickets' shape once.

## Consequences

- The feature flow gains a stage (`plan`) that only refactor drafts reach;
  `/flow` names it in place of `/critique-decisions` when a plan file exists.
- `/to-spec` mostly transcribes the plan: its sections map onto the spec's,
  and file paths appear only inside fact anchors.
- A plan finding that would change the target (modules, interfaces, seams,
  dependency rules) or an ADR's standing is always the owner's question.
