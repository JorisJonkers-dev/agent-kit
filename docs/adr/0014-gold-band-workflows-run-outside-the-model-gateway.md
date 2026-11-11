# Gold Band workflows run outside the model gateway

This amends ADR-0007. It does not replace it: every other delegation still
goes through the gateway.

ADR-0007 put every delegation on one model gateway: a shared catalogue of
client models, one ledger, per-workflow and per-day caps for the
metered-personal cost class, and one batch rule. Gold Band workflows do not
go through it. A Gold Band run is an orchestration of its own. It drives the
configured agent CLIs itself, starting each managed agent's adapter as a
direct process the way a critic's CLI is started (ADR-0001), and it hands
each node its own packet: its goal, output contract and success condition
(ADR-0006). Wrapping all of that inside one gateway delegation would nest
two orchestrators and take away the run's control of its own loops.

So Gold Band runs outside the gateway, on purpose. The controls the gateway
carries are replaced, not dropped:

- **Cost.** Each of the three pay-per-token keys (Vibe, Vibe-study, OpenCode
  Go) is cost class metered-personal, and the per-key LiteLLM budgets, which
  are EUR caps, are authoritative. The gateway's ledger, its per-workflow and
  per-day dollar caps and its batch reservation rules do not apply to Gold
  Band runs; a budget is what refuses a run that does not fit. The one
  metered-employer client, Codex, is not budgeted by LiteLLM: it is held by a
  grant (below).
- **Failure.** Gold Band's failure edges govern. A node's failure outcome
  routes along its `on: failure` edge or ends the run, and the structural
  limits bound every loop: `max_attempts` for repair loops, `max_rounds` for
  round openings, and the ai-dynamic node's fan-out, depth, parallel and
  invocation limits.
- **Safety.** Safety is not part of the exception. The family allowlist, the
  code permission and the instance's repository rule (ADR-0011) still apply
  to every Gold Band workflow, and they are checked twice. Setup checks them
  when it binds a workflow, through the setup harness with a temporary home.
  The run-start shim, `gold-band-run-start`, checks them again at every run
  start, in the first node of every workflow, before any model node runs. It
  reads the repository's remotes, the allowlist entry, the families file and
  the workflow fresh each time and never trusts what setup recorded, so an
  entry narrowed or a remote changed after setup makes the run refuse. On a
  `nedap` repository the entry is not consulted: only Codex under a grant and
  the work Claude agent are allowed, whatever an entry says (ADR-0007).
- **Grants.** In the work instance, a run that binds Codex needs a live grant
  (ADR-0008) for this run. The run is one gateway workflow, named
  `<gold band workflow>@<repository>@<run id>`, and the shim reads the
  gateway's own grants record to find the grant. The grant's cap is counted in
  Codex node starts, not dollars, because the gateway's ledger never sees Gold
  Band spend. A refused run says which grant is missing and how to ask for it.
  A grant check before each Codex node counts its starts against the cap
  (ADR-0011).

## Consequences

- ADR-0007's ledger caps no longer measure Gold Band spend. The per-key EUR
  budgets do, and they stay the authority to refuse.
- The exception is scoped to Gold Band workflows. Every other delegation
  (critique, council, a task delegated from an agent-kit workflow) still goes
  through the gateway unchanged.
- A refusal names the node, the family and the repository entry, or the
  missing grant, and writes a report beside the run (the gate report format,
  shared with the gate shim). A forbidden binding, a repository of the wrong
  instance or a missing grant stops the run before any model node starts.
- The allowlist entry is read again at each run start. The entry recorded for
  a repository decides as it stands at that moment.

## Considered options

- **Route Gold Band runs through the gateway.** Rejected: two orchestrators
  nested, and the run loses control of its loops.
- **Check safety at setup only.** Rejected: an entry narrowed, a remote
  changed or a grant lapsed after setup would go unnoticed until the next
  setup run, and the gateway that would otherwise catch it is bypassed.
