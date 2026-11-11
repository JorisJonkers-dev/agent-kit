# Lens: evidence anchoring

Check that each step stands on evidence the packet holds — a fact, an ADR or a
prerequisite — and that the Target does not contradict what the project has
already decided. Each check fixes the severity of its findings, given in
brackets.

- `unanchored-step` (major): Look for a step whose Anchor names a fact row, an
  ADR or a prerequisite that says nothing about what the step does. The
  checker already refuses an anchor that does not exist; this is an anchor
  that exists and does not support the step. Quote the anchor and the step
  side by side.
- `stale-hotspot` (minor): Look for a step ordered ahead of others on a
  hotspot the facts show is no longer one: its churn or co-change in `facts/`
  is lower than that of rows the plan leaves for later. Give the two numbers.
- `adr-contradiction` (critical): Look for a Target or a step that contradicts
  an accepted ADR in `references/`, with no step or open decision that
  supersedes it. Quote the step and the ADR.
- `closes-import-cycle` (major): Look for a Target that makes two modules
  import each other, or closes a cycle through a third. Trace the imports with
  file and line from `tree/`.
- `premature-service-split` (major): Look for a split into services, processes
  or packages deployed apart where an in-process seam would meet the Goal.
- `single-adapter-seam` (minor): Look for a port or interface the plan adds
  with one adapter and no second planned: the seam buys nothing yet.
- `deletion-without-decision` (major): Look for a step that deletes code and
  does not name the decision (an ADR, a glossary term, a recorded reason) the
  deleted code implements; the code may be there for a reason nobody wrote
  down.

The bracketed severity is the finding's: write it, and the runner sets it
whatever you write. A plan whose every step stands on evidence the packet
holds is no finding; prefer a few findings you can quote.
