## What this role cannot enforce

This text comes from the critique pipeline, where the runner — not the role
text — enforces the rules around it. Running the role as a Gold Band profile
loses that machinery, so this section states each guarantee the role relies
on without being able to enforce it: an agent taking the role is bound by
its workflow, not by this text.

- **Read-only proof.** The runner proves every critic ran read-only and
  catches one that wrote. A Gold Band agent reading this profile is bound
  only by the reach its workflow gives it.
- **Packet confinement.** The reviews above read a frozen packet the runner
  picked, and every file, citation and search stays inside it. An agent on
  this path reads whatever its reach allows, which may reach past the
  artifact and its reference files.
- **The quote check.** In the pipeline a quote that appears in no listed
  file discards its finding. Nothing here checks a quote, so check each one
  against the files before acting on a finding.
- **Schema enforcement.** The runner validates every finding a critic
  returns against the finding schema above and discards one that fails
  it. Nothing here checks a finding's shape, so validate each one
  against that schema before acting on it.
- **The refute round.** An accepted critical finding went to one more critic
  of a family that neither raised it nor is the orchestrator's. On this path
  only a workflow that runs the adversarial-verifier profile gets that, and
  nothing runs it on its own.
- **The family allowlist and the nedap refusal.** The runner consults the
  machine's family allowlist entry recorded for the repository's remote:
  a repository with no recorded entry reviews under the personal default
  — every critic family, code but no history — a declined entry or a
  `nedap` remote sends nothing, and a repository with no remote is
  refused. Which family runs this profile is decided by the Gold Band
  workflow alone, and the family allowlist is not enforced on this path.
- **Plan-limit routing.** The runner reads the model gateway's plan limits
  before it starts a critic and routes or drops the critic within them. A
  Gold Band run spends on its own plan; nothing here checks a window.
