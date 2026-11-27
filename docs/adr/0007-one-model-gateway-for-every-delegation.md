# One model gateway for every delegation

Two parts of agent-kit call models from other clients today. The critique
runner has its critic clients (Vibe and Antigravity, one pinned model each, in
`routing.json`), and council has its engines (Claude and Codex). Each has its
own way to build the command, read the output and count the cost. Neither can
list what a client offers or say how much of a plan is left. We now put one
model gateway under both. It knows each client (Vibe, Antigravity, Codex,
Claude Code, Hermes) and lists its client models: live where the client can
list them, from the client's own configuration where it cannot, and from a
declared list as a last resort. It runs a delegation and records every run in
one ledger. Any agent reaches it as a command on `PATH`, so Claude Code is
not the only possible orchestrator.

We rejected growing the critic clients or the council engines into this.
Either one would carry its own context's rules (critique's packet and
read-only proof, council's supervision) into a tool that every delegation
needs. We rejected an MCP server as the first interface. Its tool schemas
load into every session and cost tokens even when nothing is delegated. A
command costs nothing until it is called, and every client has a shell. An
MCP wrapper can come later on top of the command.

## Consequences

- Critique and council move onto the gateway in the last slices of the same
  spec. Critique keeps its safety behaviour through the move: the search
  lockdown, the home's MCP servers switched off and the per-client read-only
  proof (ADR-0001, ADR-0006). ADR-0001 still holds: critics stay direct CLI
  calls, now made through the gateway.
- Critique's ledger becomes the gateway's ledger. Critique stats read from it.
- The family allowlist applies to every delegation, not only to critiques.
  A repository owned by `nedap` sends nothing outside its permitted clients.
- A delegate cannot call the gateway. The gateway marks its delegates'
  environment and refuses calls from inside a delegation, so one workflow
  cannot fan out without limit.

## Details settled by /critique-decisions (2026-11-04)

- **A client cannot run.** When the chosen client model cannot start (signed
  out, plan limit reached, quota gone), the gateway moves on its own to
  another client model of the same cost class that covers the task's
  difficulty tier. It never moves into a costlier class by itself: it reports
  the delegation as unavailable, and the orchestrator decides.
- **No client model covers the tier.** The delegate skill keeps the task in
  the orchestrator or splits it into lower-tier tasks. It asks the owner for a
  Codex grant only when the plan says why the orchestrator is the worse
  choice. A task is never sent quietly to a model below its tier.
- **A batch plan with a failure.** A delegation that fails for good, after
  its retry and its climb, makes the gateway skip only the delegations that
  depend on it. Every staged change already made is kept, and the result
  names each delegation as done, failed or skipped.
- **nedap repositories.** "Sends nothing outside its permitted clients" means
  this: a repository owned by `nedap` may delegate to Codex on the nedap
  account (under a grant) and to Claude Code, and to nothing else, whatever
  its family allowlist entry says. Critique's rule is unchanged: such a
  repository still sends nothing to any critic.
- **Metered-personal spend.** Machine configuration caps metered-personal
  runs per workflow (default $2) and per day (default $5). The gateway
  reserves each run's estimate against both caps, as it does for grants, and
  refuses a run that does not fit either.

## Amended (2026-11-26): the allowlist follows the repository's visibility

The owner's rule: on public repositories every model is allowed and the
allowlist does not apply; on `nedap` repositories nothing is allowed; on
private repositories the allowlist decides. It replaces the `nedap` exception
above and the per-entry `public` flag `critique allow` used to set.

- **nedap repositories.** A repository with any `nedap`-owned remote sends
  nothing to any client: no delegation through the gateway (Codex and Claude
  Code included) and no critic (the work roster included). This holds even
  when the repository is public, and it is decided from the remotes alone,
  before visibility is asked.
- **Public repositories.** A repository GitHub says is public may reach every
  client the machine has, `data_scope: public` routes among them, with code and
  history; its allowlist entry is not read.
- **Private repositories.** A private or internal repository, and one whose
  visibility cannot be read (no `gh`, offline, an API error, a remote not on
  github.com), is held to its allowlist entry as before, and never reaches a
  `data_scope: public` route. Unknown visibility fails closed: it is never
  read as public.
- **Lookup.** Visibility is `gh api repos/<owner>/<repo> --jq .visibility`, run
  by the gateway's `visibility` module, which critique uses too. A known answer
  is cached for fifteen minutes under the cache directory; a failed lookup is
  never cached.
- **The `public` flag.** An entry no longer records one. `critique allow`
  refuses a `public` field, and a flag an older `allow` wrote is ignored and
  dropped on the next write. It could only widen what a private repository
  sends, which the rule no longer allows, and narrowing a public repository
  would contradict "allowlists don't apply".
- **Hooks.** The commit and PR gate and the session-start hook still read only
  the allowlist entry, with no network: they send nothing to a model, and a
  hook must finish well under a second.
