# Agents write grants after asking the owner

Codex runs on a nedap API account, so every Codex delegation costs nedap
money. The model gateway refuses a metered-employer client unless a grant
covers the workflow. A grant names one workflow and a dollar cap. It ends
when the workflow ends, after 24 hours, or when the cap is spent, whichever
comes first. The agent asks the owner for the grant and writes it itself when
the owner says yes.

We rejected a grant command that only works in an interactive terminal,
which the owner would have to run by hand. It would make a self-granting
agent impossible. It would also stop every workflow that needs Codex until
the owner types a command, and the owner judged that friction not worth it.
So the check rests on agent discipline: the gateway enforces that a grant
exists and that its cap holds, not who wrote it.

## Consequences

- Each grant records the question the owner answered and when, so a grant
  written without asking stands out in an audit of the grant file.
- If an agent is ever found writing a grant without asking, revisit this
  decision and move grant creation behind an interactive confirmation.

## Details settled by /critique-decisions (2026-11-04)

- **The cap holds under parallel runs.** Before a Codex delegation starts,
  the gateway reserves its estimated cost against the grant, and refuses the
  run if the reservation does not fit. Where the client supports a hard limit,
  the run gets one equal to its reservation. When the run ends, its actual
  spend replaces the reservation.
- **What a workflow is.** The workflow is a name the orchestrator passes on
  every gateway call, such as a spec or a batch plan. It ends when the
  orchestrator ends it, or after 24 hours. A grant that has lapsed needs the
  owner asked again; the agent never renews one itself.
- **Nobody to ask.** A headless run that wants Codex and has no grant is
  refused, and the task is handled as when no client model covers its tier:
  the orchestrator keeps it or splits it.
