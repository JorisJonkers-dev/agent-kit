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

## Amended (2026-11-04): the plan limit replaces the per-run $5 Vibe cap

The per-flow dollar cap is gone. Vibe is a subsidised plan counted by plan
limit, not a metered API billed by the token, so a dollar figure the runner
invented was never the thing that stopped it: the plan's own windows are.
Routing no longer carries `budget.vibe_flow_cap_usd`, and neither the review
stages nor the feature flows hold a cap of their own.

What replaces it: before a stage plans a Vibe critic, the runner asks the
model gateway for Vibe's plan limit — the windows this machine's
`clients.vibe.plan_limits` declares, with what the machine-wide ledger says is
spent in each. At or above the gateway's warning ratio (80%) the stage's
summary carries the gateway's warning and the stage runs. At 100% of any
window Vibe counts as unable to start, so a Vibe critic is dropped with the
same `quota` drop reason Antigravity's reserve uses, and the stage runs with
the critics left. A fallback retry is checked again, so a run that spends the
plan does not retry into it.

Consequences:

- A review stage still draws on its own flow key, but the flow key no longer
  bounds its spend: the plan is the machine's, shared with every delegation
  and every other critique.
- The runner's ledger records each critic run in the model gateway's ledger,
  so the gateway counts a critic run the way it counts a delegation and the
  plan windows see it.
- A machine routing override that still names `budget.vibe_flow_cap_usd` is
  ignored, and critique's setup output tells the owner to delete it.
