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
A model from another model family that reviews one artifact and reports
findings. A critic is read-only, sees only its packet, and never sees another
critic's findings or the conversation that produced the artifact.
_Avoid_: reviewer, judge, council member

**Orchestrator**:
The agent the owner is working in, which runs a critique and gives each of
the critics' findings its verdict.

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
of one critique run (one pass of a critique stage over one artifact) reads.
Later edits to the artifact never reach it. At a code stage (ticket review,
feature review) it is a code packet: the diff against the base without its
generated, vendored and lock files (in windows by file when it does not fit
the packet cap), the ticket or tickets, the spec, a repo
map, the context slices around the change, and as much more of the commit
under review as the packet cap allows.
_Avoid_: bundle, context, snapshot

**Repo map**:
The fact a code packet opens with: every source file at the commit under
review with its top-level symbols and their signatures, ranked by how many
other files reference them.
_Avoid_: index, outline

**Context slice**:
Lines of a file that a code packet sends around a change, at their own line
numbers: a changed function or block, a symbol it calls, or a function that
calls it.
_Avoid_: snippet, excerpt, hunk (a hunk is a piece of the diff)

**Finding**:
One defect a critic claims, with evidence a third party can check. A finding
without checkable evidence gets the verdict hint and is never acted on
automatically.

**Verdict**:
The orchestrator's ruling on a finding after checking its evidence: accepted,
rejected, needs-human, or hint (no checkable evidence, so it is reported but
never acted on automatically).
_Avoid_: vote, score

**Critique stage**:
A point in the spec-to-ticket flow where critics run (decisions, plan for a
refactor, spec, tickets, ticket review, feature review).
_Avoid_: step, phase (a phase is a span of the flow, not one stage)

**Review stage**:
A point outside any one feature's flow where critics run on demand over a
repository's decision records or code (ADR review, decision recovery,
architecture review). Its results feed the spec-to-ticket flow but are not
part of it.
_Avoid_: critique stage (that is a point in one feature's flow), audit

**Fact**:
A measurement of a repository at one commit (its imports, cycles, layer
violations, churn, files that change together) that the runner computes
into a packet. Critics cite facts; they never compute or estimate them.
_Avoid_: metric, analysis, signal (a signal chooses depth)

**Recovered decision**:
A decision a critic infers from a repository's code, and from its history
where the family allowlist permits, that no decision record holds. It is a
question for the owner and never counts as a recorded decision until the
owner accepts it and it is written as an ADR.
_Avoid_: recovered ADR, inferred ADR, candidate ADR

**Refactoring plan**:
The ordered steps that move a refactor feature's code toward a target
structure, each small enough to ship on its own, written and critiqued
before the feature's spec.
_Avoid_: migration plan, roadmap

**Depth**:
How many critics a critique stage runs: base (the stage's base roster in
routing, one critic at a cheap stage, two families at the spec stage) or deep
(base plus the stage's deep critics, each only where the family allowlist
permits). Chosen automatically from signals in the artifact unless the user
names it.
_Avoid_: tier, mode, --fast

**Refute round**:
The pass after the orchestrator's verdicts in which each accepted critical
finding goes to one critic from a family that neither raised it nor is the
orchestrator's. That critic may only refute or downgrade the finding, with
evidence; without evidence the verdict stands.
_Avoid_: challenge round, appeal

**Role**:
The job one critic is given in a critique stage (for example `spec-redteam`):
the part of its prompt, after the shared preamble, that says what it reviews
and how. A role never mentions other models or critics.
_Avoid_: persona, agent, prompt

**Lens**:
The angle one critic is told to review from (fidelity, failure modes, ...).
_Avoid_: perspective, persona

**Family allowlist**:
The per-repository list of model families a critique may send content to,
kept per machine outside the repository, so nothing about it is committed.
It is keyed by the repository's remote. A repository with no entry, or with
no remote, sends nothing; a repository owned by the `nedap` GitHub
organisation sends nothing, whatever its entry says. An entry allows
documents only unless its code permission is on; the code stages refuse
while it is off. Its history permission, also off by default, separately
lets a stage send the repository's commit messages and issue text; while it
is off, a stage leaves history out.
_Avoid_: data policy, provider list

**Flow state**:
Where one feature stands in the spec-to-ticket flow: its phase (planning,
implementation or finishing), the runs of each critique stage, each ticket
with its review status, the open needs-human questions, the gate, the flow's
Vibe spend and whether critique was skipped on purpose. It is kept per
feature in the git common directory, so every worktree shares it, and it is
only a cache: it can always be rebuilt from the ledger, the run directories
and the tracker.
_Avoid_: session, progress, checkpoint

## Build tracker

**Build tracker**:
The page that shows the owner where every ticket of a spec stands while it is
built, rendered from one state file per repository and spec. It is a view for
the owner, never a source for rebuilding flow state; the issue tracker stays
that source. "Tracker" alone means the issue tracker.
_Avoid_: dashboard, progress page, status page

**Milestone**:
A named group of a spec's tickets that ends in one closing ticket, such as a
live check. A ticket belongs to the first milestone whose closing ticket
depends on it, directly or through other tickets; a ticket in no milestone is
shown under "Other".
_Avoid_: phase (a phase is a span of the flow), stage
