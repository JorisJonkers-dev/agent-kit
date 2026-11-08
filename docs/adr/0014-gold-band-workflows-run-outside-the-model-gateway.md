# Gold Band workflows run outside the model gateway

Status: proposed.

ADR-0007 put every delegation on one model gateway: a shared catalogue of
client models, one ledger, per-workflow and per-day caps for the
metered-personal cost class, and one batch rule. Gold Band workflows do not
go through it. A Gold Band run is an orchestration of its own: it drives the
configured agent CLIs itself, starting each managed agent's adapter as a
direct process the way a critic's CLI is started (ADR-0001), and it hands
each node its own packet — its goal, output contract and success condition
(ADR-0006). Wrapping all of that inside one gateway delegation would nest
two orchestrators and take away the run's control of its own loops.

This is a deliberate exception to ADR-0007, and the controls the gateway
carries are replaced, not dropped:

- **Cost.** Each of the three pay-per-token keys (Vibe, Vibe-study, OpenCode
  Go) is cost class metered-personal, and the per-key LiteLLM budgets —
  EUR caps — are authoritative. The gateway's ledger, its per-workflow and
  per-day dollar caps and its batch reservation rules do not apply to Gold
  Band runs; a budget is what refuses a run that does not fit.
- **Failure.** Gold Band's failure edges govern: a node's failure outcome
  routes along its `on: failure` edge or ends the run, and the structural
  limits bound every loop — `max_attempts` for repair loops, `max_rounds`
  for round openings, and the ai-dynamic node's fan-out, depth, parallel
  and invocation limits.
- **Safety.** None of the exception. The family allowlist, the code
  permission and the `nedap` and no-remote refusals still apply to every
  Gold Band workflow of this spec, and they apply twice: at setup, through
  the setup harness with a temporary home, and again at each run start,
  where `gold-band-allowlist exec` runs the same check before any node
  starts, so an entry narrowed or an origin remote changed after setup
  makes the run refuse (`scripts/crates/gold-band-allowlist`, issue #352).

## Consequences

- ADR-0007's ledger caps no longer measure Gold Band spend; the per-key
  EUR budgets do, and they stay the authority to refuse.
- The exception is scoped to Gold Band workflows of this spec. Every other
  delegation — critique, council, a task delegated from an agent-kit
  workflow — still goes through the gateway unchanged.
- A refusal names the workflow's node, the family and the repository entry,
  and nothing is written: a forbidden binding, a `nedap` repository or a
  repository without an `origin` remote stops the run before it starts.
- The family allowlist entry is read again at each run start, never trusted
  from setup; the entry recorded for a repository decides as it stands at
  that moment.
