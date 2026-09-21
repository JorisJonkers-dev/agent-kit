# Memory: replacing the knowledge base

## Superseded

The mem0 plan below was never built. The estate stood up **Hindsight**
(recall/retain, knowledge tools) and **Basic Memory** (shared Markdown
vault) in-cluster instead — see agent-kit#41/#42 and
`registry/estate-tooling.yaml`'s `memory-api` and `memory-mcp` entries,
which replaced the `memory` entry this document originally shipped
(now `surfaces: []`, kept as a record only). The rest of this file is
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

- The bespoke KB was three moving parts (`knowledge-api`,
  `knowledge-ingest-worker`, `lightrag`, plus `ollama` for embeddings) that the
  estate maintained itself, for a capability that is now off-the-shelf.
- Hermes already speaks to a memory provider natively (`memory.provider`), so
  the gateway needs configuration rather than code.
- Self-hosted keeps the estate's posture: no third-party service holding
  session content.

The alternative considered was honcho, which is what Hermes' config currently
names. Honcho is the better fit for cross-session *user modelling*; mem0 is the
better fit for what the KB was actually used for — durable lessons and
decisions retrieved on demand. mem0 also has the simpler self-hosted shape.

## Why the dashboard says "unavailable"

Hermes' Runtime provider plugins page reports `honcho` as **unavailable** with
"Install provider dependencies — Python dependencies". The provider's client
libraries are not in the image, and the image is pinned by tag and digest, so
there is nothing to install at runtime without a writable venv on the PVC.

That is a symptom of the same gap: no memory backend is actually wired.
Switching the provider name alone does not fix it — the dependency has to be
present *and* a backend has to answer.

## Bring-up, in order

1. **Stand up the service in `fleet-infra`**, as `cluster/flux/apps/memory/`:
   the mem0 server, its Postgres with pgvector in the `data` namespace, a
   Vault-backed API key through VSO, and a default-deny NetworkPolicy.
2. **Open the Hermes SSRF allowlist for it.** Hermes refuses RFC1918
   destinations by default and the service resolves into `10.43.0.0/16`. The
   failure shape is the server reporting **zero tools while the hosted ones
   work** — not a NetworkPolicy error, and not visible in the object. Start
   from `hermes doctor`.
3. **Install the provider dependency** into Hermes' writable PVC venv from an
   init container, or accept the MCP server alone and leave
   `memory.memory_enabled: false`. The MCP path needs no provider plugin at
   all, which is the reason to prefer it.
4. **Point Hermes at it**: `memory.provider` and its base URL in
   `config-configmap.yaml`, with the key as a `@MEMORY_API_KEY@` placeholder
   the `seed-config` init container substitutes. Add the field to
   `secret/agents/hermes` and to that script's substitution list — miss the
   second and `sed` writes an empty string into a valid-looking config.
5. **Flip `enabled: true`** in `registry/estate-tooling.yaml`, re-render, and
   sync (`scripts/sync-hermes-registry.sh`).
6. **Verify by reading a value back**, not by a Ready condition: write a
   memory, then retrieve it from a second session.

## Decommissioning the old KB

Not part of the bring-up, and deliberately last. **Update (agent-kit#40):**
`install.sh` and `install-agents.sh` are retired; `setup-workstation.sh`
(rendered from `registry/estate-tooling.yaml`) is the one installer now, and
it is published from the public static host described below
(`https://assets.jorisjonkers.dev/setup-workstation.sh`; agent-kit#35), not
served by `knowledge-api`. The blocker this section originally described is
resolved. The paragraph below is kept for the reasoning, not as a live plan:

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
- `knowledge-api` can now be decommissioned outright (fleet-infra#231); it is
  no longer the install transport.
