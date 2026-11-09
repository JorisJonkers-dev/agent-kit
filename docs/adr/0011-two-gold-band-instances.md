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
  binds Codex needs a live model-gateway grant (ADR-0008) for that
  repository. A shim checks the grant when the run starts. Gold Band itself
  bypasses the gateway.
- **`gold-band-personal`, the personal instance.** It holds every declared
  agent except Codex and the work Claude agent. It refuses `nedap`-owned
  repositories and allows a repository with no remote, so that a new personal
  repository can be bootstrapped. It runs on the Linux workstation and the work
  machine.

The registry declares only the generic shape of the work instance (`claude-work`,
`codex`). Employer-specific settings live in a per-machine overlay beside the
keep file, so nothing about the employer enters the public registry. When both
instances run on one Mac and Gold Band cannot keep two data directories apart,
the personal instance runs under a second macOS user account.

This replaces the earlier rules that Gold Band never touches a work repository
and that Codex is banned from Gold Band. The Codex ban now applies to the
personal instance only. The direct lane (agents started directly, and the
model gateway) is unchanged and stays available on every repository.

## Considered options

- **One instance with a repository guard.** Rejected: one binding mistake
  crosses the employer boundary, and the guard is the only thing that stops it.
- **Work on the direct lane only.** Rejected: work repositories would get no
  sequenced workflows, gates or critique templates.
