# Two Gold Band instances, split by employer

The owner wants Gold Band workflows on work repositories as well as personal
ones. A single instance would hold both the employer's Codex and the personal
metered keys, and one wrong binding would send employer code to a personal
provider, or bill personal work to the employer. So there are two Gold Band
instances. They follow the launcher naming: the unsuffixed name is work, and
`-personal` is personal.

- **`gold-band`, the work instance.** It holds only the work Claude agent and
  Codex. It runs only on repositories with a `nedap`-owned remote, refuses a
  repository with no remote, and runs only on the work machine. Every run that
  binds Codex needs a live model-gateway grant (ADR-0008). Gold Band itself
  bypasses the gateway.
- **`gold-band-personal`, the personal instance.** It holds every declared
  agent except Codex and the work Claude agent. It refuses `nedap`-owned
  repositories and allows a repository with no remote, so that a new personal
  repository can be bootstrapped. It runs on the Linux workstation and the work
  machine.

The registry declares only the generic shape of the work instance (`claude-work`,
`codex`). Employer-specific settings live in a per-machine overlay beside the
keep file, so nothing about the employer enters the public registry.

## Checks before and during a run

Each Gold Band workflow starts with a run-start shim. The shim is a cheap
node that reads only the working copy and the owner's records, never model
output. Before any model node runs, it checks three things, and any failure
refuses the run:

- the instance's repository rule, against every remote of the repository;
- the family of every bound agent id, against that repository's family
  allowlist entry, including the code and history permissions. The gateway is
  bypassed, so this shim is where ADR-0007's allowlist holds for Gold Band;
- in the work instance, a live grant for every agent id that binds Codex.

Setup runs the same checks when it binds a Gold Band workflow.

**Grants.** Each Gold Band workflow run counts as one gateway Workflow, named
`<gold band workflow>@<repository>@<run id>`. So two runs on one repository
need two grants, each with its own cap. The gateway's ledger never sees
Gold Band spend, so a Gold Band grant's cap is counted in Codex node starts,
not dollars. Every node that binds Codex is preceded by a grant-check shim
node. When the grant has lapsed, by time or by cap, that shim node fails. The
failure routes to the failure edge and a manual check, so a run never
continues on Codex past its grant.

## The work machine

On the work Mac, the personal instance always runs under a second macOS user
account. That account has its own data directory, keychain, keep file and
overlay. Setup runs once in each account. This is the default, not a
fallback. Unix permissions then also keep personal agents from reading the
employer clones in the work account (#396), which the OS write limit
(ADR-0013) does not prevent.

On Linux, where no employer clones exist, a recorded check (the first B0
ticket of the spec) settles whether Gold Band can keep two data directories
apart. Until that check is recorded, setup converges only the instance the
machine already supports, and reports the other one as blocked.

This replaces the earlier rules that Gold Band never touches a work repository
and that Codex is banned from Gold Band. The Codex ban now applies to the
personal instance only. The direct lane (agents started directly, and the
model gateway) is unchanged and stays available on every repository.

## Considered options

- **One instance with a repository guard.** Rejected: one binding mistake
  crosses the employer boundary, and the guard is the only thing that stops it.
- **Work on the direct lane only.** Rejected: work repositories would get no
  sequenced workflows, gates or critique templates.
