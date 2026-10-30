# agent-kit

agent-kit sets up the estate's coding agents. `registry/estate-tooling.yaml`
says what each agent surface (Claude Code in every profile, Codex, each Vibe
home, Antigravity) should hold, and a setup run makes the surface match it.

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
it unless the keep file names it — except on Antigravity, where setup leaves
it alone (see **Converge**).
_Avoid_: hand-added entry, unknown entry

**Keep file**:
A per-machine list outside the repository, `~/.config/agent-kit/keep.yaml`,
of undeclared entries that setup leaves alone. It is a top-level `keep:` list
of entry names; a missing file is an empty list. Work-profile items live here,
so nothing work-related enters the public registry.
_Avoid_: allowlist, ignore file

**Converge**:
What a setup run does to each surface: it ends with exactly the declared
entries plus the kept ones — except Antigravity, whose one MCP file setup
merges, keeping every server it does not name except a retired one and
Private Memory.
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

## Memory and launchers

**Shared Memory**:
The memory bank every installed agent and launcher may read and write without
a passphrase.
_Avoid_: default bank, estate memory, memory-api (that is the server, not the concept)

**Private Memory**:
The owner's personal-notes memory bank, reachable only from vibe-private,
after the owner types the passphrase. No keep file can keep it on another
surface.
_Avoid_: personal bank, personal datastore, private bank

**Passphrase gate**:
The interactive prompt the owner must answer at a terminal before Private
Memory is reachable; no agent can answer it.
_Avoid_: password, unlock

**Launcher**:
A named command that starts an agent with a fixed account, memory access and
local state.
_Avoid_: alias, wrapper, profile

**Account suffix**:
A launcher-name suffix (`-personal`, `-study`) naming which vendor account the
launcher signs in with; no suffix means the default, personal account.

**Private suffix**:
The launcher-name suffix `-private`, marking the only launcher behind the
Passphrase gate, the one that reaches Private Memory.

**Vibe home**:
The local state one or more Vibe launchers share: settings, session history,
logs, skills. vibe and vibe-study share one; vibe-private has its own.
_Avoid_: profile, config dir

**Study account**:
The owner's university account at a vendor, used by `-study` launchers.
_Avoid_: student account, school account

## Critique pipeline

**Critic**:
A model from another provider that reviews one artifact and reports findings.
A critic is read-only, sees only its packet, and never sees another critic's
findings or the conversation that produced the artifact.
_Avoid_: reviewer, judge, council member

**Critic client**:
One CLI pinned to one model, through which a critic runs (for example Vibe
with Mistral Large, or Antigravity with Gemini Flash).
_Avoid_: preset, provider, engine

**Model family**:
The vendor whose training produced a model (Mistral, Zhipu, DeepSeek, Google).
Two critics count as independent only when their families differ, and no
critic shares a family with the orchestrator that adjudicates its findings.
_Avoid_: provider (Vibe serves several families)

**Packet**:
The frozen copy of the artifact and its reference material that every critic
of one critique run reads. Later edits to the artifact never reach it.
_Avoid_: bundle, context, snapshot

**Finding**:
One defect a critic claims, with evidence a third party can check.
A claim without checkable evidence is a hint, not a finding.

**Verdict**:
The orchestrator's ruling on a finding after checking its evidence: accepted,
rejected, needs-human, or hint.
_Avoid_: vote, score

**Critique stage**:
A point in the spec-to-ticket flow where critics run (decisions, spec,
tickets, ticket review, feature review).
_Avoid_: step, phase (a phase is a span of the flow, not one stage)

**Depth**:
How many critics a critique stage runs: base (two families) or deep (base
plus every further family the allowlist permits). Chosen automatically from
signals in the artifact unless the user names it.
_Avoid_: tier, mode, --fast

**Lens**:
The angle one critic is told to review from (fidelity, failure modes, ...).
_Avoid_: perspective, persona

**Family allowlist**:
The per-repository list of model families a critique may send content to,
kept per machine outside the repository, so nothing about it is committed.
A repository with no allowlist sends nothing; a repository owned by the
`nedap` GitHub organisation sends nothing, whatever its allowlist says.
_Avoid_: data policy, provider list
