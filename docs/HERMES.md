# Making things available to Hermes

Hermes is the in-cluster gateway. It gets its skills and MCP servers from two
ConfigMaps in `fleet-infra`, and both are generated from this repo's registry
so a skill added once reaches the workstation and Hermes together.

## The loop

```bash
# 1. add or change one entry, with surfaces: [..., hermes]
$EDITOR registry/estate-tooling.yaml

# 2. render and check
uv run python scripts/render_registry.py --write
uv run pytest tests/test_registry_render.py

# 3. push into a fleet-infra checkout (writes files, commits nothing)
scripts/sync-hermes-registry.sh ../../platform/fleet-infra

# 4. review, commit, PR, merge in fleet-infra
```

Then — and only then — reconcile. **Flux reads `deploy/production`, which a
workflow publishes from `main`.** Reconciling before that publish lands
silently applies the previous revision, so the ConfigMap you just merged is not
the one the pod reads.

```bash
kubectl -n agents-system rollout restart deploy/hermes
```

## Verifying, and why `Ready` is not evidence

Both init containers **fail soft** by design: a network blip leaves the last
good skill tree in place rather than starting the dashboard with no skills. So
a `Ready` pod is compatible with having vendored nothing.

Read what they say:

```bash
kubectl -n agents-system logs deploy/hermes -c seed-config
kubectl -n agents-system logs deploy/hermes -c sync-skills
```

`sync-skills` counts skills **from disk**, not from what it thought it copied,
and **rejects** any source whose commit does not match its pin. `seed-config`
names any MCP credential that resolved empty, and names the configured
provider's key when that is the missing one.

Then ask Hermes itself, rather than reading the object back:

```bash
kubectl -n agents-system exec deploy/hermes -- hermes skills list
kubectl -n agents-system exec deploy/hermes -- hermes doctor
```

## The three traps

**Pins are commit shas.** `sync-skills` clones `--branch <ref>` and compares
`git rev-parse HEAD`. Almost every upstream tag is annotated, so
`git ls-remote <repo> refs/tags/<tag>` returns the tag *object* and can never
match. Peel it: `git ls-remote <repo> 'refs/tags/<tag>^{}'`. The registry
validator rejects a malformed pin before it can reach the cluster.

**In-cluster MCP servers hit the SSRF block.** Hermes refuses RFC1918
destinations by default and every in-cluster service resolves into
`10.43.0.0/16`. The symptom is a server reporting **zero tools while the hosted
ones work** — that is the block, not the NetworkPolicy, which explicitly allows
them. Start from `hermes doctor`.

**Git wins over the dashboard.** `config.yaml` is reseeded onto the PVC at
every pod start, so anything changed through the dashboard's config editor is
overwritten on the next restart. The previous file is kept as
`config.yaml.superseded`. Sessions, memories and hub-installed skills are
untouched.

## Credentials

A registry entry's `credential:` renders as `@NAME@` in the generated
`mcp_servers:` block. Two more things have to exist for it to resolve:

1. the field in `secret/agents/hermes` (via
   `scripts/ops/set-hermes-secrets.sh` in `fleet-infra`), and
2. a substitution for it in the `seed-config` init script and a matching env
   var on the Deployment.

Miss either and `sed` writes an empty string. The result is a syntactically
valid config whose server fails at the first tool call — which is why
`seed-config` checks each source variable is non-empty and names the server it
breaks, rather than trusting its own exit code.

## What Hermes does not get

- **Claude Code plugins.** They are a Claude Code concept. Where a plugin is
  really a skill wrapper — `caveman`, `drawio`, `kubernetes-skill`,
  `mattpocock-skills` — the registry vendors the upstream skills into Hermes
  from the same source repo, so the surfaces stay in step without pretending
  Hermes can install a plugin.
- **Language servers.** Same reason: the LSP wiring is a plugin declaration.
- **Spec Kit.** Not on any surface.
