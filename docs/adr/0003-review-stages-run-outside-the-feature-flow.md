# Repository-level reviews are review stages outside the feature flow

ADR review, decision recovery and architecture review look at a whole
repository's decision records or code, not at one feature's spec, tickets or
diff. We run them as **review stages**: each has its own skill
(`/critique-adrs`, `/recover-decisions`, `/review-architecture`), is started on
demand, and is a flow of its own keyed by its run id, so it draws on its own
$5 Vibe cap. They are not added to the feature flow's stages, and `/flow`
reaches them through a separate entry.

We rejected adding them to the feature flow. The flow model is built around
one feature: its stages are a fixed record keyed by the feature's spec or
draft, and its second round, its "do not start" line and its stats pruning all
assume a spec. A repository review has no spec and no point in a feature's
life where it is due. Its results reach the feature flow as input instead: a
review's accepted findings become candidates for
`/improve-codebase-architecture`, and decisions the owner accepts become ADRs
that later critiques read.

## Consequences

- The runner needs a flow key and a spend key per review run, and per-stage
  branches wherever it assumes a spec (second round, report, stats).
- `/flow` cannot tell the owner that a review is due; reviews happen only when
  someone asks for one.
- Review stages get no hook, consistent with ADR-0002.
