# personal-stack Constitution

## Core Principles

### I. Human Authorship and No Attribution

All repository work is authored solely by the human driver. Do not add
`Co-Authored-By` trailers, generated-by footers, assistant names, model names,
or automation-attribution text to commits, PRs, code comments, docs, generated
files, or templates.

### II. Validate Against Reality

Claims about paths, APIs, config, cluster state, or tooling must be checked
against the real codebase and, where relevant, live state. If a fact is unknown,
search the repo, inspect the source, or run the narrowest safe command before
designing around it. Do not invent secret paths, resource names, commands, or
contracts.

### III. Claude/Codex Parity

Agent-facing behavior must stay equivalent across Claude and Codex surfaces.
Any skill, hook, memory rule, installer behavior, command, or project guidance
added for one agent must get the matching surface for the other in the same
branch, unless an explicit unsupported reason is recorded.

### IV. Render and Validate Discipline

Render-managed files are edited only at their source templates or inventory.
After touching a render source, run the owning renderer and commit the rendered
output with the source change. Run the smallest meaningful validation command
for the touched area, and state exactly what remains unverified if a check
cannot run.

### V. Small Stacked PRs

Every change should be reviewable, revertable, and scoped to one objective.
Prefer small stacked PRs over broad bundles. Avoid tangential cleanup,
speculative abstractions, unrelated refactors, and compatibility shims when a
direct local-pattern change is available.

## Required Workflow

1. Start from a spec for user-visible or cross-cutting changes. The spec must
   describe outcomes, acceptance criteria, non-goals, and open questions.
2. Plan against existing repo patterns and real paths. Surface architectural
   limitations before implementation begins.
3. Break work into tasks that preserve small PR boundaries and parallel safety.
4. Implement only the task scope. Never revert or overwrite unrelated parallel
   edits.
5. Validate with the smallest meaningful command for the touched area:
   `./gradlew :services:<service>:test` for Kotlin services,
   `./gradlew :platform:tooling:test` for platform tooling, and
   `npm run typecheck && npm run lint && npm run test` inside Vue UIs.
6. Capture durable lessons or decisions in the knowledge base when they affect
   future repo behavior, without storing secrets, raw transcripts, or full
   diffs.

## Render-Managed Boundaries

- `platform/inventory/fleet.yaml` is the source of truth for public service
  routing, catalog, placement, exposure, and access intent.
- Generated Traefik routes, catalog ConfigMaps, agent-kit mirrors, and installer
  artifacts must not be hand-edited.
- `.specify/memory/constitution.md` is committed and hand-edited for this repo.
  The generic `.specify/templates/constitution-template.md` is only the
  render-managed starter for future seed installs.

## Governance

This constitution overrides ad-hoc agent behavior. Amend it deliberately when
the governing workflow changes, and update `AGENTS.md`, `CLAUDE.md`, skills, or
templates in the same branch when parity requires it.

**Version**: 1.0.0
**Ratified**: 2026-06-08
**Last Amended**: 2026-06-08
