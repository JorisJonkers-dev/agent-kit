# agent-kit

agent-kit sets up the estate's coding agents. `registry/estate-tooling.yaml`
says what each agent surface (Claude Code in every profile, Codex, Vibe)
should hold, and a setup run makes the surface match it.

## Registry entries

**Declared entry**:
An MCP server, marketplace, plugin or hook that `registry/estate-tooling.yaml`
names for a surface. Setup owns it. An entry supplied by a declared plugin
(`plugin:*`) or by `coding_agents:` (the `hindsight` server and its hooks) is
declared through that source.
_Avoid_: managed entry, fleet entry

**Retired record**:
A declared entry with `surfaces: []`, one listed in `retired_plugins:`, or one
with `enabled: false` on a surface. It stays in the registry as history, and
setup removes it from that surface.
_Avoid_: disabled entry, legacy entry

**Undeclared entry**:
Anything present on a surface that the registry does not name. Setup removes
it unless the keep file names it.
_Avoid_: hand-added entry, unknown entry

**Keep file**:
A per-machine list outside the repository, `~/.config/agent-kit/keep.yaml`,
of undeclared entries that setup leaves alone. It is a top-level `keep:` list
of entry names; a missing file is an empty list. Work-profile items live here,
so nothing work-related enters the public registry.
_Avoid_: allowlist, ignore file

**Converge**:
What a setup run does to each surface: it ends with exactly the declared
entries plus the kept ones.
_Avoid_: sync, reconcile

## Estate cleanup

**Old stack**:
The knowledge stack (fleet-infra#231), `ollama-heavy-gateway`, the mem0
leftovers, Basic Memory, and the old external-dns owner id. It does not cover
OpenRouter.
_Avoid_: legacy stack

**Stale record**:
An external-dns ownership TXT record, and the record it owns, whose route is
gone from both `main` and the cluster.
_Avoid_: orphan record
