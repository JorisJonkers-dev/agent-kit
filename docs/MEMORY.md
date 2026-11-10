# Memory: replacing the knowledge base

## Superseded

The mem0 plan below was never built. The estate stood up **Hindsight**
(recall/retain, knowledge tools) and **Basic Memory** (shared Markdown
vault) in-cluster instead — see agent-kit#41/#42 and
`registry/estate-tooling.yaml`'s `memory-api` and `memory-mcp` entries,
which replaced the `memory` entry this document originally shipped.
That entry is gone from the registry; setup removes a leftover `memory`
server as undeclared. Basic Memory has since been
retired too: `memory-mcp` is `surfaces: []` and the service is parked, so
Hindsight (`memory-api`, Shared Memory, everywhere; `memory-private`, Private Memory, in `vibe-private` only) is the agent memory. The rest of this file is
history: it explains why the KB was retired and records the reasoning
that led to mem0 being picked over honcho, which is still accurate, but
the "Bring-up" and "Decision" sections describe a service that does not
exist. Do not follow them.

## Where this stood

The estate's own knowledge base is being retired. What is **done**:

- Every knowledge hook is gone from the kit, and `setup-workstation.sh`
  purges them from a machine that still has them (agent-kit#40 moved this
  off the retired `install-agents.sh`).
- `manifest.yaml` fails validation if a `hooks:` or `settings:` section
  reappears.
- The `knowledge` MCP server sits in the registry with `surfaces: []` — a
  record, registered nowhere.

What is **not yet done**: the replacement service does not exist. The registry
carries a `memory` MCP entry with `enabled: false` for exactly that reason.
Registering a URL that answers nothing would give every surface a server that
fails at the first tool call, which is worse than having no memory.

Flip `enabled: true` in the same change that lands the Deployment, never
before.

## The decision

Self-host **mem0** in-cluster rather than keep building a bespoke knowledge
service. The reasoning:

- The bespoke KB was three moving parts (its API, an ingest worker,
  `lightrag`, plus `ollama` for embeddings) that the
  estate maintained itself, for a capability that is now off-the-shelf.
- Self-hosted keeps the estate's posture: no third-party service holding
  session content.

The alternative considered was honcho. Honcho is the better fit for cross-session *user modelling*; mem0 is the
better fit for what the KB was actually used for — durable lessons and
decisions retrieved on demand. mem0 also has the simpler self-hosted shape.

## Bring-up (retired)

The mem0 bring-up steps targeted the in-cluster Hermes gateway, which is
retired (estate#316), and mem0 itself was replaced by Hindsight. They are
gone; the current stack is described above.

## Decommissioning the old KB

Not part of the bring-up, and deliberately last. **Update (agent-kit#40):**
`install.sh` and `install-agents.sh` are retired; `setup-workstation.sh`
(rendered from `registry/estate-tooling.yaml`) is the one installer now, and
it is published as a GitHub release asset
(`https://github.com/JorisJonkers-dev/agent-kit/releases/latest/download/setup-workstation.sh`),
not served by the knowledge service. It was first published from the public
static host described below (agent-kit#35); that host,
`assets.jorisjonkers.dev`, is legacy now that the in-cluster
`garage-asset-sync` CronJob is being retired, and can at most become a
redirect to GitHub, owned outside this repo. The blocker this section
originally described is resolved. The paragraph below is kept for the reasoning, not as a live plan:

The knowledge service used to serve the installer (`install.sh` /
`install-agents.sh` were fetched from it with a bearer token), so it could not
be switched off until those artifacts were published somewhere else. The plan
was a public, auth-free static host — a Garage bucket behind the public edge,
since the installer is a public-shaped GET and the bearer token bought nothing
once the content was not secret.

Remaining follow-up now that the transport dependency is gone:

- **Revoke the KB bearer token** on any machine that ran the old installer. It
  sits in plaintext in `~/.claude.json` under the `knowledge` MCP entry.
  Unregistering the server does not revoke the token.
- The knowledge service is decommissioned (fleet-infra#231).
