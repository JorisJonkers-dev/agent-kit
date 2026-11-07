# Gold Band port inventory (issue #351)

Port inventory of everything this repository ships to agent surfaces — one row
per entry — ahead of the Gold Band port (parent issue #350). Each row states
what the entry is, where it lives, the single target kind it gets in Gold Band,
the reach level its port needs, and whether the assignment depends on a Gold
Band fact this session could not verify.

## Sources, method and limitations

Entries were gathered by reading the worktree: `skills/workstation/` and
`skills/repo/` (skill directories), `manifest.yaml`, `installer/`
(`setup-workstation.sh`, `vibe-hooks.toml`), `registry/estate-tooling.yaml`
and `registry/generated/`, `agents/`, `runner/manifests/runtime/mcp/`, and the
ADRs under `docs/adr/` (in particular ADR-0002, the hook ADR).

Limitations of this session, stated so no row hides them:

- `gh issue view` was denied by the sandbox approval policy, so issues #351,
  #350, #367, #327 and #363 could not be read. Only the facts relayed in the
  #351 brief were used, and they are treated as verified: **hooks always stay
  outside; Gold Band has no hook system; Opus only in judgment nodes; no
  Codex.**
- `/Users/j.w.jonkers/IDEAProjects/agent-kit/handoff-gold-band/reports/gb-dsl-facts.md`
  lies outside this worktree and access to it was denied. The reach ladder
  (read < read-web < research < write-scoped < write < full, plus
  run-one-command) is therefore applied as its best-effort reading; rows whose
  reach or kind depends on that file, or on the unread specs, say so in the
  last column.
- The worktree is the branch this task ran in; counts below were taken from
  its disk listings.

## Skill directories

Counted on disk: `skills/workstation/` holds **16** skill directories,
`skills/repo/` holds **17**, **33 in total** — matching the 16 + 17 rows
below. (Renamed-away skills — `critique-decisions`, `critique-spec`,
`check-tickets`, `review-ticket`, `review-feature` — are retired records in
the registry, not directories on disk, and get no rows.)

### skills/workstation (16)

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact |
| --- | --- | --- | --- | --- | --- |
| agent-session-bootstrap | `skills/workstation/agent-session-bootstrap/` | Checklist for configuring a Claude Code/Codex session: config layers, MCP servers, memory-file hygiene, parity. | skill set | read | yes — whether client-session configuration guidance has a Gold Band equivalent (specs #350/#367 unread) |
| build-tracker | `skills/workstation/build-tracker/` | Renders each ticket's status for a spec being implemented into a one-page summary with dependency graph, from a per-spec state file. | workflow | write-scoped | yes — bundled runner `build-tracker.mjs` needs a Gold Band execution story (Rust workspace #331) |
| council | `skills/workstation/council/` | Cross-model orchestrator (Claude + Codex): triage, cross-model planning, grill, design checkpoint, guarded fan-out, review council. | stays outside | n/a | no — verified fact: no Codex in Gold Band, and cross-model fan-out is council's core |
| critique | `skills/workstation/critique/` | The critique pipeline's shared bundle: bundled runner `critique.mjs`, prompt preamble, roles, lenses, finding schema, default routing. | workflow | run-one-command | yes — whether a Gold Band template invokes the runner or the Rust workspace (#331) reimplements it |
| critique-stats | `skills/workstation/critique-stats/` | Reports how each critic client performs per stage from the ledger, ending with routing suggestions; writes nothing. | workflow | read | yes — runs the runner's `stats` command (Rust workspace #331) |
| delegate | `skills/workstation/delegate/` | Plan work to hand to other model clients through the model gateway: tier rating, split, cheapest cost class, one batch, verify. | workflow | write | yes — whether Gold Band retains a model gateway to delegate through (ADR-0007; specs unread) |
| flow | `skills/workstation/flow/` | The pipeline's stepper: says where the current feature stands and names the single next step, running a stage on the owner's word. | workflow | run-one-command | yes — runs the runner's `state` command (Rust workspace #331) |
| kb-first | `skills/workstation/kb-first/` | Consult the memory platform before designing or changing behaviour that may depend on prior captures; capture durable lessons near task completion. | skill set | write | yes — whether a skill set gets write reach to the memory MCP server in Gold Band |
| model-gateway | `skills/workstation/model-gateway/` | The model gateway's runner and shipped defaults: client catalogue, difficulty tiers, capabilities, caps and thresholds. | workflow | full | yes — whether delegation survives in Gold Band or judgment nodes replace the gateway; specs unread |
| pr-composer | `skills/workstation/pr-composer/` | Compose a pull request in the repository's house style: reads the PR template, mines merged PRs, gates on explicit confirmation before push/create. | workflow | write | no |
| review-grill | `skills/workstation/review-grill/` | Critique a feature's decisions after grilling and before /to-spec: gap critics list the unasked questions. | workflow | read | no — already Wave 2 (critique stages as templates, #363) |
| review-implementation | `skills/workstation/review-implementation/` | Review one ticket's staged change before its commit: risk triage, one critic on the diff, each bug finding decided by running its failing test. | workflow | run-one-command | no — already Wave 2 (critique stages as templates, #363) |
| review-pr | `skills/workstation/review-pr/` | Review the whole feature diff before the pull request: critics from two other families read spec, tickets and diff; accepted findings become follow-up tickets. | workflow | write-scoped | no — already Wave 2 (critique stages as templates, #363) |
| review-spec | `skills/workstation/review-spec/` | Critique a spec after /to-spec: critics from other families review it read-only and the orchestrator rules on findings. | workflow | read | no — already Wave 2 (critique stages as templates, #363) |
| review-tickets | `skills/workstation/review-tickets/` | Check a spec's tickets after /to-tickets: deterministic ticket checker plus one critic linting structure read-only. | workflow | run-one-command | no — already Wave 2 (critique stages as templates, #363) |
| token-economy | `skills/workstation/token-economy/` | Token-budget discipline: progressive disclosure, bounded recall, narrow MCP profiles, prompt-cache-friendly ordering. | skill set | read | no |

### skills/repo (17)

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact |
| --- | --- | --- | --- | --- | --- |
| add-public-service | `skills/repo/add-public-service/` | Add a new SSO-protected service to the estate stack: the aligned file set across `fleet.yaml`, auth-api and app-ui, plus render and test. | workflow | write-scoped | yes — whether repo/fleet-specific skills are in the port's scope (#350 unread) |
| agent-session-bootstrap | `skills/repo/agent-session-bootstrap/` | Repo-source variant of the session-configuration checklist, rendered per CLI target. | skill set | read | yes — same session-configuration question as the workstation variant |
| council | `skills/repo/council/` | The council skill's repo source (Claude + Codex targets): orchestrate large decomposable work cross-model. | stays outside | n/a | no — verified fact: no Codex |
| council-amend | `skills/repo/council-amend/` | Amend an existing council brief, design, task DAG or run after feedback, failed validation or changed constraints. | stays outside | n/a | no — verified fact: no Codex |
| council-design | `skills/repo/council-design/` | Produce a council-ready design, consolidated plan and parallel task DAG from a clear brief. | stays outside | n/a | no — verified fact: no Codex |
| council-grill | `skills/repo/council-grill/` | Stress-test a plan, patch, task DAG or decision through a council-style adversarial review. | stays outside | n/a | no — verified fact: no Codex |
| council-inject | `skills/repo/council-inject/` | Inject new constraints, evidence or operator guidance into an active council flow. | stays outside | n/a | no — verified fact: no Codex |
| council-run | `skills/repo/council-run/` | Execute an approved council plan or task DAG through supervised workers. | stays outside | n/a | no — verified fact: no Codex |
| council-supervise | `skills/repo/council-supervise/` | Supervise a single cross-CLI delegate with bounded scope and progress relay. | stays outside | n/a | no — verified fact: no Codex |
| council-triage | `skills/repo/council-triage/` | Decide whether work belongs in a normal session, a council split skill, or a full council run. | stays outside | n/a | no — verified fact: no Codex |
| deploy-status | `skills/repo/deploy-status/` | Fetch and summarise the gate status for a homelab-deploy pull request: the four required checks and merge-readiness. | workflow | read | yes — same scope question for repo-specific skills (#350 unread) |
| fleet-change | `skills/repo/fleet-change/` | Edit `platform/inventory/fleet.yaml` safely: render generated Traefik/catalog YAML, run tooling tests, commit the rendered output. | workflow | write-scoped | yes — same scope question (#350 unread) |
| kb-first | `skills/repo/kb-first/` | Repo-source variant of the memory-first consultation and capture skill. | skill set | write | yes — memory write reach in Gold Band (gb-dsl-facts.md unread) |
| open-pr | `skills/repo/open-pr/` | Open a pull request the way this repo requires: assignee, label, impersonal PR-body voice, no assistant attribution. | workflow | write | yes — same scope question (#350 unread) |
| prepare-rollback | `skills/repo/prepare-rollback/` | Prepare a rollback branch pinning one service to a prior artifact version; returns a COMPARE URL, never merges or approves. | workflow | write-scoped | yes — same scope question (#350 unread) |
| run-tests | `skills/repo/run-tests/` | Run the right tests before pushing a change in the fleet monorepo: Kotlin suites, platform tooling, Vue/TS typecheck + lint + vitest. | workflow | run-one-command | yes — same scope question (#350 unread) |
| token-economy | `skills/repo/token-economy/` | Repo-source variant of the token-budget discipline skill. | skill set | read | no |

## Hooks

This repo installs exactly the three critique hooks ADR-0002 allows, merged by
`setup-workstation.sh` into Claude Code's `settings.json` and Codex's
`hooks.json` from `skills/workstation/critique/hooks/`. Verified fact: Gold
Band has no hook system and hooks always stay outside — every hook row is
"stays outside".

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact |
| --- | --- | --- | --- | --- | --- |
| critique-commit-gate | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-commit-gate.sh` | Blocks a risky staged diff before `git commit` until a passing /review-implementation run matches it. | stays outside | n/a | no — verified: Gold Band has no hook system |
| critique-pr-gate | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-pr-gate.sh` | Blocks `gh pr create` or a push outside the feature branch patterns unless /review-pr passed for the head. | stays outside | n/a | no — verified: Gold Band has no hook system |
| critique-session-start | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-session-start.sh` | Prints one line suggesting /flow setup in an opted-in repository whose machine setup is missing or failing. | stays outside | n/a | no — verified: Gold Band has no hook system |
| hindsight recall hooks | `registry/estate-tooling.yaml` (`coding_agents:`), wired by `installer/setup-workstation.sh` (`hindsight_coding_agents`) | The `@vectorize-io/hindsight-coding-agents` plugin wires its hooks and a `hindsight` server into Claude Code and Codex; locked to recall-once at session start. | stays outside | n/a | no — the hooks stay outside (verified); the recall capability itself is a question, below |
| security-guidance plugin hooks | `registry/estate-tooling.yaml` (`plugins: security-guidance`) | The plugin ships its own PreToolUse and Stop hooks for security review of generated code; the plugin is declared here, its hooks come with it. | stays outside | n/a | no — verified: Gold Band has no hook system |

Not installed (found, recorded so nothing is invented): the retired
knowledge-recall hooks (edit recall, git-commit capture, session digest,
user-prompt recall) are purged from Claude and Codex homes on every setup run;
`installer/vibe-hooks.toml` declares no Vibe hooks today and setup removes the
old `agent-kit-retain-private` hook. Neither gets a row.

## Roles

The critique role files are the six under `skills/workstation/critique/roles/`
(pinned in `manifest.yaml`'s `critique.files`; there is no separate repo copy).
Role-like files elsewhere are listed too, so every one is in exactly one row.

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact |
| --- | --- | --- | --- | --- | --- |
| diff-reviewer | `skills/workstation/critique/roles/diff-reviewer.md` | Reviews one ticket's staged diff for bugs a third party can check. | role | read | yes — whether roles are part of the Wave 2 stage templates (#363) or a separate wave |
| feature-reviewer | `skills/workstation/critique/roles/feature-reviewer.md` | Reviews the whole feature diff against its spec and tickets. | role | read | yes — same #363 question |
| gap-critic | `skills/workstation/critique/roles/gap-critic.md` | Lists the questions nobody asked about a feature's decisions. | role | read | yes — same #363 question |
| refuter | `skills/workstation/critique/roles/refuter.md` | In the refute round, may only refute or downgrade an accepted critical finding, with evidence. | role | read | yes — whether the refute round is a Gold Band judgment node (Opus-only, per the #327 fact) or a critic role |
| spec-redteam | `skills/workstation/critique/roles/spec-redteam.md` | Redteams a spec for failure modes and omissions. | role | read | yes — same #363 question |
| ticket-linter | `skills/workstation/critique/roles/ticket-linter.md` | Lints a spec's ticket structure read-only. | role | read | yes — same #363 question |
| council role prompts | `council/prompts/` (mirrored byte-for-byte at `skills/workstation/council/prompts/`, pinned in `manifest.yaml` `council.files`) | Role prompts for council members: baseline, designer, planner, critic, reviewers (acceptance/adversarial/edgecase), consolidators, triage judge, verifier, worker, and companions. | stays outside | n/a | no — verified fact: no Codex; council does not port |
| Vibe critic agents | `registry/generated/vibe/agents/critic-mistral-large.toml`, `critic-mistral-small.toml`, `critic-glm.toml`, `critic-deepseek-flash.toml` (from `registry/estate-tooling.yaml` `vibe_critic_agents:`) | Read-only critic client agents for the critique pipeline, each pinning a model and its prices so caps fire. | role | read | yes — whether Gold Band renders its own critic-client agent configs or replaces critic clients with judgment nodes (#350/#367 unread) |
| Vibe delegate agents | `registry/generated/vibe/agents/delegate-readonly.toml`, `delegate-write.toml` (from `vibe_delegate_agents:`) | Read-only and worktree-write delegate agents of the model gateway, with no shell, no MCP, nothing outside scope. | role | read | yes — whether a Gold Band delegate role exists without the model gateway (ADR-0007; specs unread) |
| codex-med | `agents/codex-med.md` (rendered to `.claude/agents/codex-med.md`) | Claude Code subagent supervising a Codex gpt-5.5 delegate at medium reasoning effort behind the model gateway and grants. | stays outside | n/a | no — verified fact: no Codex |
| codex-xhigh | `agents/codex-xhigh.md` (rendered to `.claude/agents/codex-xhigh.md`) | Same supervisor at high reasoning effort for difficult reasoning or deep review. | stays outside | n/a | no — verified fact: no Codex |

Not found on disk: the review-stage skills ADR-0003 names (`/critique-adrs`,
`/recover-decisions`, `/review-architecture`) have no skill directories in
this worktree; they exist only as ADR-0003's design record and get no rows.

## MCP servers

Declared in `registry/estate-tooling.yaml` (`mcp_servers:`, and `hindsight`
via `coding_agents:`), plus the profiles the runner image declares in
`runner/manifests/runtime/mcp/` (Claude JSON and Codex TOML variants).
Retired and disabled records are listed and marked.

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact |
| --- | --- | --- | --- | --- | --- |
| memory-api | `registry/estate-tooling.yaml` (`mcp_servers:`) | Hindsight long-term memory for the shared bank: explicit read/write/search knowledge tools; workstation, Hermes and runner surfaces. | MCP server | write | no |
| memory-private | `registry/estate-tooling.yaml` (`mcp_servers:`) | Private Memory, the personal Hindsight bank, only in vibe-private behind the passphrase gate. | stays outside | n/a | yes — whether Gold Band has (or should have) a passphrase-gated private surface (specs unread) |
| memory-mcp | `registry/estate-tooling.yaml` (`mcp_servers:`) | Retired record: Basic Memory over the knowledge-vault; `surfaces: []`, registered nowhere. | stays outside | n/a | no |
| n8n | `registry/estate-tooling.yaml` | n8n workflows through the instance-level MCP server, OAuth per client. | MCP server | write | no |
| n8n-mcp | `registry/estate-tooling.yaml` | n8n node docs and schemas, docs-only, no API key. | MCP server | read | no |
| kubernetes | `registry/estate-tooling.yaml` | Read-only cluster diagnostics through the server's own ClusterRole; workstation reaches it via a port-forward. | MCP server | read | no |
| github | `registry/estate-tooling.yaml` (`mcp_servers:`, Hermes/runner; the `github` Claude Code plugin on the workstation) | GitHub issues, branches, pull requests; delete and label tools excluded to stop invented labels. | MCP server | write | no |
| grimoire | `registry/estate-tooling.yaml` | Grimoire D&D 5e table companion via its MCP server, an account-scoped token. | MCP server | write-scoped | no |
| outline | `registry/estate-tooling.yaml` | Outline wiki at notes.jorisjonkers.dev, reached as a separate Outline user with that user's visibility. | MCP server | write | no |
| vuetify | `registry/estate-tooling.yaml` | Vuetify component API; disabled (`enabled: false`) on every surface today. | stays outside | n/a | no |
| playwright | `registry/estate-tooling.yaml` | Headless Chromium browser automation via playwright-mcp. | MCP server | full | no |
| drawio | `registry/estate-tooling.yaml` | Native .drawio diagram authoring and export via drawio-mcp. | MCP server | write-scoped | no |
| overleaf | `registry/estate-tooling.yaml` | Self-hosted Overleaf: pull, push, compile, review comments via olcli-mcp. | MCP server | write | no |
| hindsight | `registry/estate-tooling.yaml` (`coding_agents:`), registered in every Claude profile and Codex by `installer/setup-workstation.sh` | Hindsight recall-inject memory server for Claude Code and Codex: one recall at a session's first prompt, saves are deliberate. | MCP server | read | yes — whether Gold Band is a supported harness for hindsight coding agents (its hooks already stay outside) |
| git-host | `runner/manifests/runtime/mcp/claude-mcp-servers.*.json`, `codex-mcp-servers.*.toml` (all profiles), via `bin/gh-mcp-wrapper` | GitHub MCP for the runner image through the wrapper and its gh-app token, minted at session start. | MCP server | write | no |
| code-intel | `runner/manifests/runtime/mcp/*code-intel*`, `*cluster*`, `*full-diagnostic*` (serena; also a registry `clis:` container entry) | LSP-backed semantic code intelligence (serena), no-memories mode. | MCP server | read | no |
| frontend-docs | `runner/manifests/runtime/mcp/claude-mcp-servers.full-diagnostic.json`, `...cluster.json`, cod equivalents | Frontend docs server via a build-time `@FRONTEND_DOCS_MCP_URL@` placeholder. | MCP server | read | no |
| ui-docs | same runner profiles | UI docs server via a build-time `@UI_DOCS_MCP_URL@` placeholder. | MCP server | read | no |
| cluster | same runner profiles (plus `claude-mcp-servers.cluster.json`) | Cluster MCP via a build-time `@CLUSTER_MCP_URL@` placeholder. | MCP server | read | no |

## Check script

Not written. The #351 check script would be new Rust code, and per the issue
it waits on the Rust workspace from #331; this issue delivers the inventory
document only. When #331 lands, the check script should verify each row here
against what the port actually shipped (one entry per row, exactly one kind,
declared reach within the ladder, and the 33-skill-directory count).

## Proposed wave order after Wave 2

Wave 2 (#363) already covers the critique stages as Gold Band templates:
`review-grill`, `review-spec`, `review-tickets`, `review-implementation`,
`review-pr` (workflow, read … run-one-command … write-scoped as in the table).
After it:

1. **Wave 3 — Roles and shared prompt parts.** The six critique role files,
   the lenses, the preamble, the output contract and the triage prompt — only
   the parts not already folded into the #363 stage templates (question 2
   below). The Opus-only-in-judgment-nodes fact decides where the verdict and
   refute passes land.
2. **Wave 4 — MCP servers.** Order by what depends on them: `memory-api` and
   the GitHub surfaces (`github`/`git-host`) first because the critique and
   flow waves read and write through them, then the read-only servers
   (`kubernetes`, `n8n-mcp`, `code-intel`, the runner docs/cluster profiles),
   then the writing ones (`outline`, `grimoire`, `drawio`, `overleaf`, `n8n`,
   `playwright`).
3. **Wave 5 — Guidance skill sets.** `kb-first` (after Wave 4's memory-api),
   `token-economy`, and `agent-session-bootstrap` if the owner keeps its
   subject in scope.
4. **Wave 6 — Runner-bound workflows.** `critique` bundle (if not subsumed
   by #363), `flow`, `critique-stats`, `build-tracker`, then `model-gateway`
   and `delegate` (both predicated on the delegation question below), then
   `pr-composer`. These are gated on the Rust workspace (#331), since the
   bundled `.mjs` runners' Gold Band replacement is that workspace.
5. **Not ported.** Everything marked "stays outside" — the three critique
   hooks, the plugin-supplied hooks, the whole council family (workstation
   and repo), `codex-med`/`codex-xhigh`, `memory-private`, `memory-mcp`,
   `vuetify`, and the council role prompts.

The repo-specific workflows (`add-public-service`, `deploy-status`,
`fleet-change`, `open-pr`, `prepare-rollback`, `run-tests`) are placed by
question 5, and deliberately not scheduled until it is answered.

## Questions that need the owner

1. Reach ladder semantics could not be verified this session (`gb-dsl-facts.md`
   and specs #350/#327/#367 unread; `gh` denied by the sandbox). Confirm or
   correct the reach levels assigned per row.
2. Are the six critic roles and the lenses inside the Wave 2 stage templates
   (#363), or do they need their own Wave 3?
3. Runner strategy: are the bundled Node runners (`critique.mjs`,
   `build-tracker.mjs`, `model-gateway.mjs`) reimplemented in the Rust
   workspace (#331), with templates invoking them (hence run-one-command), or
   does Gold Band keep invoking the Node bundles?
4. With all hooks staying outside, what enforces the commit gate and PR gate
   in Gold Band — a gate/verdict template, or are the gates advisory there?
5. Are repo-specific skills (fleet/deploy operations: `add-public-service`,
   `deploy-status`, `fleet-change`, `open-pr`, `prepare-rollback`,
   `run-tests`) in the port's scope, or do they stay with the surface CLIs?
6. Does Gold Band keep the model gateway and delegation (ADR-0007), or do its
   judgment nodes (Opus-only) replace cross-client delegation — deciding the
   fate of `model-gateway`, `delegate`, the Vibe delegate agents and the
   critic-client agents?
7. Are `memory-private` and the `hindsight` recall server in the Gold Band
   port at all (passphrase gate; harness support)? Their hooks are verified
   outside; the recall capability's port is unclear in the unread specs.
8. Do the review stages ADR-0003 names (`/critique-adrs`,
   `/recover-decisions`, `/review-architecture`) belong to this port from
   another issue? They have no skill directories on disk, so they are absent
   from this inventory.
