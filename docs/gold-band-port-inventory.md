# Gold Band port inventory (issue #351)

Port inventory of everything this repository ships to agent surfaces — one row
per entry — ahead of the Gold Band port (parent issue #350). Each row states
what the entry is, where it lives, the single target kind it gets in Gold Band,
the reach level its port needs, and whether the assignment depends on a
Gold Band fact this session could not verify. The owner answered the open
questions on 2026-11-09; the decisions are recorded in "## Owner decisions",
and the waves they imply in "## Waves".

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

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact | Target |
| --- | --- | --- | --- | --- | --- | --- |
| agent-session-bootstrap | `skills/workstation/agent-session-bootstrap/` | Checklist for configuring a Claude Code/Codex session: config layers, MCP servers, memory-file hygiene, parity. | skill set | read | yes — whether client-session configuration guidance has a Gold Band equivalent (specs #350/#367 unread) | - |
| build-tracker | `skills/workstation/build-tracker/` | Renders each ticket's status for a spec being implemented into a one-page summary with dependency graph, from a per-spec state file. | workflow | write-scoped | no — owner decision 3, 2026-11-09 (#374): `build-tracker.mjs` is reimplemented in the Rust workspace (#331) and templates call the crates | - |
| council | `skills/workstation/council/` | Cross-model orchestrator (Claude + Codex): triage, cross-model planning, grill, design checkpoint, guarded fan-out, review council. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| critique | `skills/workstation/critique/` | The critique pipeline's shared bundle: bundled runner `critique.mjs`, prompt preamble, roles, lenses, finding schema, default routing. | workflow | run-one-command | no — owner decision 3, 2026-11-09 (#374): `critique.mjs` is reimplemented in the Rust workspace (#331) and templates call the crates | - |
| critique-stats | `skills/workstation/critique-stats/` | Reports how each critic client performs per stage from the ledger, ending with routing suggestions; writes nothing. | workflow | read | no — owner decision 3, 2026-11-09 (#374): the runner's `stats` command is reimplemented in the Rust workspace (#331) | - |
| delegate | `skills/workstation/delegate/` | Plan work to hand to other model clients through the model gateway: tier rating, split, cheapest cost class, one batch, verify. | stays outside | n/a | no — owner decision 2, 2026-11-09 (#374): Gold Band nodes bind an agent and a model per node, so `model-gateway` and `delegate` stay CLI-only and are not ported | - |
| flow | `skills/workstation/flow/` | The pipeline's stepper: says where the current feature stands and names the single next step, running a stage on the owner's word. | workflow | run-one-command | no — owner decision 3, 2026-11-09 (#374): the runner's `state` command is reimplemented in the Rust workspace (#331), so the stepper becomes a template that calls the crate | - |
| kb-first | `skills/workstation/kb-first/` | Consult the memory platform before designing or changing behaviour that may depend on prior captures; capture durable lessons near task completion. | skill set | write | yes — whether a skill set gets write reach to the memory MCP server in Gold Band | - |
| model-gateway | `skills/workstation/model-gateway/` | The model gateway's runner and shipped defaults: client catalogue, difficulty tiers, capabilities, caps and thresholds. | stays outside | n/a | no — owner decision 2, 2026-11-09 (#374): stays CLI-only and unported; decision 3 reimplements `model-gateway.mjs` in the Rust workspace (#331) only where a workflow needs the crate | - |
| pr-composer | `skills/workstation/pr-composer/` | Compose a pull request in the repository's house style: reads the PR template, mines merged PRs, gates on explicit confirmation before push/create. | workflow | write | yes — whether PR composition ports as a workflow with write reach is unverified | - |
| review-grill | `skills/workstation/review-grill/` | Critique a feature's decisions after grilling and before /to-spec: gap critics list the unasked questions. | workflow | read | no — #367 Implementation Decisions, Waves: the critique stages are Wave 2 (#363) templates | - |
| review-implementation | `skills/workstation/review-implementation/` | Review one ticket's staged change before its commit: risk triage, one critic on the diff, each bug finding decided by running its failing test. | workflow | run-one-command | no — #367 Implementation Decisions, Waves: the critique stages are Wave 2 (#363) templates | - |
| review-pr | `skills/workstation/review-pr/` | Review the whole feature diff before the pull request: critics from two other families read spec, tickets and diff; accepted findings become follow-up tickets. | workflow | write-scoped | no — #367 Implementation Decisions, Waves: the critique stages are Wave 2 (#363) templates | - |
| review-spec | `skills/workstation/review-spec/` | Critique a spec after /to-spec: critics from other families review it read-only and the orchestrator rules on findings. | workflow | read | no — #367 Implementation Decisions, Waves: the critique stages are Wave 2 (#363) templates | - |
| review-tickets | `skills/workstation/review-tickets/` | Check a spec's tickets after /to-tickets: deterministic ticket checker plus one critic linting structure read-only. | workflow | run-one-command | no — #367 Implementation Decisions, Waves: the critique stages are Wave 2 (#363) templates | - |
| token-economy | `skills/workstation/token-economy/` | Token-budget discipline: progressive disclosure, bounded recall, narrow MCP profiles, prompt-cache-friendly ordering. | skill set | read | yes — whether a guidance skill set ports with read reach is unverified | - |

### skills/repo (17)

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact | Target |
| --- | --- | --- | --- | --- | --- | --- |
| add-public-service | `skills/repo/add-public-service/` | Add a new SSO-protected service to the estate stack: the aligned file set across `fleet.yaml`, auth-api and app-ui, plus render and test. | workflow | write-scoped | no — owner decision 1, 2026-11-09 (#374): becomes a Gold Band workflow, scoped to this repository, in Wave 9 (## Waves below) | - |
| agent-session-bootstrap | `skills/repo/agent-session-bootstrap/` | Repo-source variant of the session-configuration checklist, rendered per CLI target. | skill set | read | yes — same session-configuration question as the workstation variant | - |
| council | `skills/repo/council/` | The council skill's repo source (Claude + Codex targets): orchestrate large decomposable work cross-model. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-amend | `skills/repo/council-amend/` | Amend an existing council brief, design, task DAG or run after feedback, failed validation or changed constraints. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-design | `skills/repo/council-design/` | Produce a council-ready design, consolidated plan and parallel task DAG from a clear brief. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-grill | `skills/repo/council-grill/` | Stress-test a plan, patch, task DAG or decision through a council-style adversarial review. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-inject | `skills/repo/council-inject/` | Inject new constraints, evidence or operator guidance into an active council flow. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-run | `skills/repo/council-run/` | Execute an approved council plan or task DAG through supervised workers. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-supervise | `skills/repo/council-supervise/` | Supervise a single cross-CLI delegate with bounded scope and progress relay. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| council-triage | `skills/repo/council-triage/` | Decide whether work belongs in a normal session, a council split skill, or a full council run. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| deploy-status | `skills/repo/deploy-status/` | Fetch and summarise the gate status for a homelab-deploy pull request: the four required checks and merge-readiness. | workflow | read | no — owner decision 1, 2026-11-09 (#374): becomes a Gold Band workflow, scoped to this repository, in Wave 9 (## Waves below) | - |
| fleet-change | `skills/repo/fleet-change/` | Edit `platform/inventory/fleet.yaml` safely: render generated Traefik/catalog YAML, run tooling tests, commit the rendered output. | workflow | write-scoped | no — owner decision 1, 2026-11-09 (#374): becomes a Gold Band workflow, scoped to this repository, in Wave 9 (## Waves below) | - |
| kb-first | `skills/repo/kb-first/` | Repo-source variant of the memory-first consultation and capture skill. | skill set | write | yes — memory write reach in Gold Band (gb-dsl-facts.md unread) | - |
| open-pr | `skills/repo/open-pr/` | Open a pull request the way this repo requires: assignee, label, impersonal PR-body voice, no assistant attribution. | workflow | write | no — owner decision 1, 2026-11-09 (#374): becomes a Gold Band workflow, scoped to this repository, in Wave 9 (## Waves below) | - |
| prepare-rollback | `skills/repo/prepare-rollback/` | Prepare a rollback branch pinning one service to a prior artifact version; returns a COMPARE URL, never merges or approves. | workflow | write-scoped | no — owner decision 1, 2026-11-09 (#374): becomes a Gold Band workflow, scoped to this repository, in Wave 9 (## Waves below) | - |
| run-tests | `skills/repo/run-tests/` | Run the right tests before pushing a change in the fleet monorepo: Kotlin suites, platform tooling, Vue/TS typecheck + lint + vitest. | workflow | run-one-command | no — owner decision 1, 2026-11-09 (#374): becomes a Gold Band workflow, scoped to this repository, in Wave 9 (## Waves below) | - |
| token-economy | `skills/repo/token-economy/` | Repo-source variant of the token-budget discipline skill. | skill set | read | yes — whether a guidance skill set ports with read reach is unverified | - |

## Hooks

This repo installs exactly the three critique hooks ADR-0002 allows, merged by
`setup-workstation.sh` into Claude Code's `settings.json` and Codex's
`hooks.json` from `skills/workstation/critique/hooks/`. Verified fact: Gold
Band has no hook system and hooks always stay outside — every hook row is
"stays outside".

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact | Target |
| --- | --- | --- | --- | --- | --- | --- |
| critique-commit-gate | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-commit-gate.sh` | Blocks a risky staged diff before `git commit` until a passing /review-implementation run matches it. | stays outside | n/a | no — #367 Implementation Decisions, Stays outside: hooks have no Gold Band equivalent; owner decision 4, 2026-11-09 (#374): the commit gate stays git-side, and a gate-report node (#353) adds visibility only | - |
| critique-pr-gate | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-pr-gate.sh` | Blocks `gh pr create` or a push outside the feature branch patterns unless /review-pr passed for the head. | stays outside | n/a | no — #367 Implementation Decisions, Stays outside: hooks have no Gold Band equivalent; owner decision 4, 2026-11-09 (#374): the PR gate stays GitHub-side, and a gate-report node (#353) adds visibility only | - |
| critique-session-start | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-session-start.sh` | Prints one line suggesting /flow setup in an opted-in repository whose machine setup is missing or failing. | stays outside | n/a | no — #367 Implementation Decisions, Stays outside: hooks have no Gold Band equivalent | - |
| hindsight recall hooks | `registry/estate-tooling.yaml` (`coding_agents:`), wired by `installer/setup-workstation.sh` (`hindsight_coding_agents`) | The `@vectorize-io/hindsight-coding-agents` plugin wires its hooks and a `hindsight` server into Claude Code and Codex; locked to recall-once at session start. | stays outside | n/a | no — #367 Implementation Decisions, Stays outside: hooks have no Gold Band equivalent; owner decision 5, 2026-11-09 (#374): the `hindsight` recall server ports read-only for agents, and its hooks stay outside | - |
| security-guidance plugin hooks | `registry/estate-tooling.yaml` (`plugins: security-guidance`) | The plugin ships its own PreToolUse and Stop hooks for security review of generated code; the plugin is declared here, its hooks come with it. | stays outside | n/a | no — #367 Implementation Decisions, Stays outside: hooks have no Gold Band equivalent | - |

Not installed (found, recorded so nothing is invented): the retired
knowledge-recall hooks (edit recall, git-commit capture, session digest,
user-prompt recall) are purged from Claude and Codex homes on every setup run;
`installer/vibe-hooks.toml` declares no Vibe hooks today and setup removes the
old `agent-kit-retain-private` hook. Neither gets a row.

## Roles

The critique role files are the six under `skills/workstation/critique/roles/`
(pinned in `manifest.yaml`'s `critique.files`; there is no separate repo copy).
Role-like files elsewhere are listed too, so every one is in exactly one row.

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact | Target |
| --- | --- | --- | --- | --- | --- | --- |
| diff-reviewer | `skills/workstation/critique/roles/diff-reviewer.md` | Reviews one ticket's staged diff for bugs a third party can check. | role | read | no — owner decisions, 2026-11-09 (#374): question 2 is settled by #345 — the six roles are Gold Band profiles, rendered by the `gold-band-roles` crate | - |
| feature-reviewer | `skills/workstation/critique/roles/feature-reviewer.md` | Reviews the whole feature diff against its spec and tickets. | role | read | no — owner decisions, 2026-11-09 (#374): question 2 is settled by #345 — the roles are Gold Band profiles, rendered by the `gold-band-roles` crate | - |
| gap-critic | `skills/workstation/critique/roles/gap-critic.md` | Lists the questions nobody asked about a feature's decisions. | role | read | no — owner decisions, 2026-11-09 (#374): question 2 is settled by #345 — the roles are Gold Band profiles, rendered by the `gold-band-roles` crate | - |
| refuter | `skills/workstation/critique/roles/refuter.md` | In the refute round, may only refute or downgrade an accepted critical finding, with evidence. | role | read | no — owner decisions, 2026-11-09 (#374): question 2 is settled by #345 — the refuter is one of the six Gold Band profiles | - |
| spec-redteam | `skills/workstation/critique/roles/spec-redteam.md` | Redteams a spec for failure modes and omissions. | role | read | no — owner decisions, 2026-11-09 (#374): question 2 is settled by #345 — the roles are Gold Band profiles, rendered by the `gold-band-roles` crate | - |
| ticket-linter | `skills/workstation/critique/roles/ticket-linter.md` | Lints a spec's ticket structure read-only. | role | read | no — owner decisions, 2026-11-09 (#374): question 2 is settled by #345 — the roles are Gold Band profiles, rendered by the `gold-band-roles` crate | - |
| council role prompts | `council/prompts/` (mirrored byte-for-byte at `skills/workstation/council/prompts/`, pinned in `manifest.yaml` `council.files`) | Role prompts for council members: baseline, designer, planner, critic, reviewers (acceptance/adversarial/edgecase), consolidators, triage judge, verifier, worker, and companions. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| Vibe critic agents | `registry/generated/vibe/agents/critic-mistral-large.toml`, `critic-mistral-small.toml`, `critic-glm.toml`, `critic-deepseek-flash.toml` (from `registry/estate-tooling.yaml` `vibe_critic_agents:`) | Read-only critic client agents for the critique pipeline, each pinning a model and its prices so caps fire. | role | read | no — owner decision 2, 2026-11-09 (#374): workflows bind an agent and a model per node, so the critic agents become Gold Band agent bindings | - |
| Vibe delegate agents | `registry/generated/vibe/agents/delegate-readonly.toml`, `delegate-write.toml` (from `vibe_delegate_agents:`) | Read-only and worktree-write delegate agents of the model gateway, with no shell, no MCP, nothing outside scope. | role | read | no — owner decision 2, 2026-11-09 (#374): the Vibe delegate agents become Gold Band agents | - |
| codex-med | `agents/codex-med.md` (rendered to `.claude/agents/codex-med.md`) | Claude Code subagent supervising a Codex gpt-5.5 delegate at medium reasoning effort behind the model gateway and grants. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |
| codex-xhigh | `agents/codex-xhigh.md` (rendered to `.claude/agents/codex-xhigh.md`) | Same supervisor at high reasoning effort for difficult reasoning or deep review. | stays outside | n/a | no — #327 Implementation Decisions, Hard rules in validation: Codex never appears in a Gold Band binding; owner decisions 2 and 6, 2026-11-09 (#374), keep it outside the port | - |

Not found on disk: the review-stage skills ADR-0003 names (`/critique-adrs`,
`/recover-decisions`, `/review-architecture`) have no skill directories in
this worktree; they exist only as ADR-0003's design record and get no rows.

## MCP servers

Declared in `registry/estate-tooling.yaml` (`mcp_servers:`, and `hindsight`
via `coding_agents:`), plus the profiles the runner image declares in
`runner/manifests/runtime/mcp/` (Claude JSON and Codex TOML variants).
Retired and disabled records are listed and marked.

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact | Target |
| --- | --- | --- | --- | --- | --- | --- |
| memory-api | `registry/estate-tooling.yaml` (`mcp_servers:`) | Hindsight long-term memory for the shared bank: explicit read/write/search knowledge tools; workstation, Hermes and runner surfaces. | MCP server | write | yes — which MCP servers port, and at which reach, is unverified | - |
| memory-private | `registry/estate-tooling.yaml` (`mcp_servers:`) | Private Memory, the personal Hindsight bank, only in vibe-private behind the passphrase gate. | stays outside | n/a | no — owner decision 5, 2026-11-09 (#374): memory-private stays outside | - |
| memory-mcp | `registry/estate-tooling.yaml` (`mcp_servers:`) | Retired record: Basic Memory over the knowledge-vault; `surfaces: []`, registered nowhere. | stays outside | n/a | yes — whether a retired record stays outside without a Gold Band decision is unverified | - |
| n8n | `registry/estate-tooling.yaml` | n8n workflows through the instance-level MCP server, OAuth per client. | MCP server | write | yes — which MCP servers port, and at which reach, is unverified | - |
| n8n-mcp | `registry/estate-tooling.yaml` | n8n node docs and schemas, docs-only, no API key. | MCP server | read | yes — which MCP servers port, and at which reach, is unverified | - |
| kubernetes | `registry/estate-tooling.yaml` | Read-only cluster diagnostics through the server's own ClusterRole; workstation reaches it via a port-forward. | MCP server | read | yes — which MCP servers port, and at which reach, is unverified | - |
| github | `registry/estate-tooling.yaml` (`mcp_servers:`, Hermes/runner; the `github` Claude Code plugin on the workstation) | GitHub issues, branches, pull requests; delete and label tools excluded to stop invented labels. | MCP server | write | yes — which MCP servers port, and at which reach, is unverified | - |
| grimoire | `registry/estate-tooling.yaml` | Grimoire D&D 5e table companion via its MCP server, an account-scoped token. | MCP server | write-scoped | yes — which MCP servers port, and at which reach, is unverified | - |
| outline | `registry/estate-tooling.yaml` | Outline wiki at notes.jorisjonkers.dev, reached as a separate Outline user with that user's visibility. | MCP server | write | yes — which MCP servers port, and at which reach, is unverified | - |
| vuetify | `registry/estate-tooling.yaml` | Vuetify component API; disabled (`enabled: false`) on every surface today. | stays outside | n/a | yes — whether a server disabled on every surface today gets a Gold Band target is unverified | - |
| playwright | `registry/estate-tooling.yaml` | Headless Chromium browser automation via playwright-mcp. | MCP server | full | yes — which MCP servers port, and at which reach, is unverified | - |
| drawio | `registry/estate-tooling.yaml` | Native .drawio diagram authoring and export via drawio-mcp. | MCP server | write-scoped | yes — which MCP servers port, and at which reach, is unverified | - |
| overleaf | `registry/estate-tooling.yaml` | Self-hosted Overleaf: pull, push, compile, review comments via olcli-mcp. | MCP server | write | yes — which MCP servers port, and at which reach, is unverified | - |
| hindsight | `registry/estate-tooling.yaml` (`coding_agents:`), registered in every Claude profile and Codex by `installer/setup-workstation.sh` | Hindsight recall-inject memory server for Claude Code and Codex: one recall at a session's first prompt, saves are deliberate. | MCP server | read | no — owner decision 5, 2026-11-09 (#374): the `hindsight` MCP server is ported, read-only for agents | - |
| git-host | `runner/manifests/runtime/mcp/claude-mcp-servers.*.json`, `codex-mcp-servers.*.toml` (all profiles), via `bin/gh-mcp-wrapper` | GitHub MCP for the runner image through the wrapper and its gh-app token, minted at session start. | MCP server | write | yes — which MCP servers port, and at which reach, is unverified | - |
| code-intel | `runner/manifests/runtime/mcp/*code-intel*`, `*cluster*`, `*full-diagnostic*` (serena; also a registry `clis:` container entry) | LSP-backed semantic code intelligence (serena), no-memories mode. | MCP server | read | yes — which MCP servers port, and at which reach, is unverified | - |
| frontend-docs | `runner/manifests/runtime/mcp/claude-mcp-servers.full-diagnostic.json`, `...cluster.json`, cod equivalents | Frontend docs server via a build-time `@FRONTEND_DOCS_MCP_URL@` placeholder. | MCP server | read | yes — which MCP servers port, and at which reach, is unverified | - |
| ui-docs | same runner profiles | UI docs server via a build-time `@UI_DOCS_MCP_URL@` placeholder. | MCP server | read | yes — which MCP servers port, and at which reach, is unverified | - |
| cluster | same runner profiles (plus `claude-mcp-servers.cluster.json`) | Cluster MCP via a build-time `@CLUSTER_MCP_URL@` placeholder. | MCP server | read | yes — which MCP servers port, and at which reach, is unverified | - |

## Older issues

Each overlapping older issue this port touches gets exactly one row, resolved
by a named target or by staying outside, so none is closed twice or left
dangling (#367's Overlapping older issues rule; issue #370).

| Name | Path | What it does | Target kind | Reach | Depends on unverified Gold Band fact | Target |
| --- | --- | --- | --- | --- | --- | --- |
| #24 | issue #24 | Cross-provider review harness | workflow | write | no — #367 Implementation Decisions, Overlapping older issues: #24 is Workflow B in #350 | - |
| #167 | issue #167 | Critique pipeline foundation and /critique-spec | workflow | run-one-command | no — #367 Implementation Decisions, Overlapping older issues: critique-pipeline rows are Wave 2 (#363) | - |
| #181 | issue #181 | The remaining critique stages | workflow | run-one-command | no — #367 Implementation Decisions, Overlapping older issues: critique-pipeline rows are Wave 2 (#363) | - |
| #182 | issue #182 | /flow, hooks and /critique-stats | workflow | run-one-command | no — #367 Implementation Decisions, Overlapping older issues; owner decision 3, 2026-11-09 (#374): the runner rewrite moves `/flow` and `/critique-stats` to Wave 4 (## Waves below), and the hooks stay outside | - |
| #198 | issue #198 | Install the three critique hooks through setup | stays outside | n/a | no — #367 Implementation Decisions, Overlapping older issues: #198 (hooks) stays outside | - |
| #214 | issue #214 | ADR review, decision recovery, architecture review and refactoring plans | workflow | run-one-command | no — #367 Implementation Decisions, Overlapping older issues; owner decision 8, 2026-11-09 (#374): the ADR-0003 review stages are built directly as Gold Band templates in Wave 5 (## Waves below) | - |
| #285 | issue #285 | Research delegation with web access and citation checks | workflow | research | no — #367 Implementation Decisions, Overlapping older issues: #285 is Workflow A in #350; owner decision 2, 2026-11-09 (#374): the research workflow binds an agent and a model per node, landing in Wave 9 (## Waves below) | - |
| #286 | issue #286 | Estate workstation skills (umbrella) | skill set | read | no — the owner's decisions, 2026-11-09 (#374), place every workstation row of this inventory in a wave (## Waves below) | - |
| #287 | issue #287 | Skill sources drift check (a weekly CI workflow) | stays outside | n/a | yes — proposed mapping; #367 leaves the estate-skill issues unverified pending #351; the owner's decisions in #374 do not name the weekly drift check, so the proposed stays-outside stands | - |
| #288 | issue #288 | estate-project-intent skill | skill set | write | yes — proposed mapping; #367 leaves the estate-skill issues unverified pending #351; the owner's decisions in #374 do not name the estate-* skills, so the mapping stays proposed and the reach unverified | - |
| #289 | issue #289 | estate-delivery-status skill | skill set | read | yes — proposed mapping; #367 leaves the estate-skill issues unverified pending #351; the owner's decisions in #374 do not name the estate-* skills, so the mapping stays proposed and the reach unverified | - |
| #290 | issue #290 | estate-rollback skill (plans, never dispatches) | skill set | read | yes — proposed mapping; #367 leaves the estate-skill issues unverified pending #351; the owner's decisions in #374 do not name the estate-* skills, so the mapping stays proposed and the reach unverified | - |
| #291 | issue #291 | estate-bootstrap skill | skill set | write | yes — proposed mapping; #367 leaves the estate-skill issues unverified pending #351; the owner's decisions in #374 do not name the estate-* skills, so the mapping stays proposed and the reach unverified | - |
| #292 | issue #292 | Retire the six personal-stack repo skills | stays outside | n/a | no — owner decision 1, 2026-11-09 (#374): the six repo-specific skills become Gold Band workflows in Wave 9 (## Waves below), so retiring their CLI copies is a CLI-side action that stays outside the port | - |

## Check script

`scripts/crates/inventory-check` (std-only workspace member) checks this
document against the repository and exits 1 on any problem:

```bash
cargo run --manifest-path scripts/Cargo.toml -p inventory-check -- .
```

It fails when a directory under `skills/workstation/` or `skills/repo/` has no
row, when a skills row has no directory, when a hook in `manifest.yaml`, a
critique role file or an MCP server in `registry/estate-tooling.yaml` has no row
in the `## Hooks`, `## Roles` or `## MCP servers` section (and fails if one of
those sources yields nothing), and when any row (skills, hooks,
roles, MCP servers) lacks exactly one target kind from the five allowed or a
reach from the ladder (`n/a` only for "stays outside").

It also fails on the waves. A row whose kind is not "stays outside" must sit
in exactly one `### Wave ...` subsection of the `## Waves` section: a row no
wave lists fails as `<section>/<name>: row sits in no wave`, a row that two
waves list fails naming its waves, and a row that stays outside may not sit
in a wave. A wave item must name an inventory row that exists, spelled
`<section>: <name>` as the Name cell spells it, with the row's section first
so a name used in two sections stays unambiguous; an item may end in
`(loop)` to mark a row whose skill assumes a loop; anything else fails as
`<wave>: "<item>" names no inventory row`. Items written `new: <name>` name
new work with no inventory row and are exempt. Subsections of `## Waves`
that are not waves (such as `### Stays outside`) are skipped.

A kind cell that names two of the five kinds fails as "row has two target
kinds", and an empty kind cell fails as "row has no target kind". Every row of
the `## Hooks` section must say "stays outside"; any other kind there fails as
"a hook must stay outside". The last column, `Target`, is `-` while nothing is
ported; any other value must exist for the row's kind — a `workflow` target is a
path from the repository root, an `MCP server` target is a name in
`registry/estate-tooling.yaml`'s `mcp_servers:`, a `role` or `skill set` target
is an agent id or skill-set name the registry declares as `id: <name>` or
`name: <name>` (the Gold Band section is not built yet, so that text match is
the rule for now), and a "stays outside" target must be `-`; anything else fails
as "target <target> does not exist".

A row in any section whose `Depends on unverified Gold Band fact` cell starts
with `no` must cite one of `#327`, `#350`, `#367` or `#374`; a `no` without such a
citation fails as `<section>/<name>: "no" without a spec citation`. A
`workflow` target must stay inside the repository: an absolute path or a path
with a `..` segment is refused as not existing, because `root.join(target)`
would otherwise escape it.

## Interactive skills

The five interactive skills have no skill directories in this repository, so
they have no inventory rows; they enter the port in Wave 8 as new work. Each
maps, per owner decision 7 (2026-11-09, #374), to human-gate nodes, as in
Workflow C (#362): each question round is a manual-check node, and the human
answers in the app or over IM. The rounds are unrolled into the template as
a fixed sequence of manual-check nodes, bounded by the Gold Band DSL's
`control.max_attempts` / `control.max_rounds`.

- **grill-me** — the grilling's question rounds become manual-check nodes,
  one per round, and each human answer is that node's outcome.
- **grill-with-docs** — same mapping; an answer that changes a decision
  goes to the decision docs the way the skill does today.
- **brainstorming** — same mapping; each round of the exploration is one
  manual-check node.
- **to-spec** — same mapping; each question round before and while the spec
  is drafted is one manual-check node.
- **to-tickets** — same mapping; each question round while the tickets are
  cut is one manual-check node.

## Waves

Wave 2 (#363) is already ticketed; Waves 3 to 9 cut the rest of the port
into tickets, in the order the owner's decisions (2026-11-09) imply. Every
wave item names an inventory row as the Name cell spells it, with its
section first, so a name used in two sections stays unambiguous
(`skills/repo: open-pr`); work with no inventory row is named
`new: ...`. An item ending in `(loop)` marks a row whose skill assumes a
loop — it repeats a step until a condition holds. The static Gold Band DSL
has no loops, so the ticket porting a `(loop)` row must carry its loop
unrolled into the ticket's fixed node sequence, bounded by
`control.max_attempts` / `control.max_rounds`, as an acceptance criterion.

### Wave 2 — the critique stage templates (#363, already ticketed)

- skills/workstation: review-grill (loop)
- skills/workstation: review-spec (loop)
- skills/workstation: review-tickets
- skills/workstation: review-implementation (loop)
- skills/workstation: review-pr (loop)
- Older issues: #24
- Older issues: #167
- Older issues: #181

### Wave 3 — the roles, the agents and the shared prompt parts

- Roles: diff-reviewer
- Roles: feature-reviewer
- Roles: gap-critic
- Roles: refuter
- Roles: spec-redteam
- Roles: ticket-linter
- Roles: Vibe critic agents
- Roles: Vibe delegate agents

The `gold-band-roles` crate (#345) renders the six role files as Gold Band
profiles; the shared preamble, output contract, lens texts and finding schema
travel with them (docs/gold-band-roles.md). Owner decision 2 makes the Vibe
critic and delegate agents Gold Band agent bindings that a workflow binds
per node.

### Wave 4 — the Rust runners and the gate report (decisions 3 and 4)

- new: `critique.mjs`, reimplemented in the Rust workspace (#331)
- new: `build-tracker.mjs`, reimplemented in the Rust workspace (#331)
- new: `model-gateway.mjs`, reimplemented in the Rust workspace (#331, only
  where a workflow needs it)
- new: the gate-report node (#353, visibility only)
- skills/workstation: critique
- skills/workstation: build-tracker
- skills/workstation: critique-stats
- skills/workstation: flow (loop)
- Older issues: #182 (loop)

Owner decision 3 rewrites the bundled runners in the Rust workspace and has
the templates call the crates, so the runner-bound rows port here — and the
Wave 2 templates call the same crates, so this wave lands with them at the
latest. Owner decision 4 keeps the commit gate and the PR gate git- and
GitHub-side; the gate-report node adds visibility only.

### Wave 5 — the ADR-0003 review stages (decision 8)

- new: /critique-adrs
- new: /recover-decisions
- new: /review-architecture
- Older issues: #214

Per decision 8 the three review stages of ADR-0003 are in this epic; their
tickets build them directly as Gold Band templates.

### Wave 6 — the MCP servers, in dependency order (decision 5)

- MCP servers: memory-api
- MCP servers: github
- MCP servers: git-host
- MCP servers: hindsight
- MCP servers: kubernetes
- MCP servers: n8n-mcp
- MCP servers: code-intel
- MCP servers: frontend-docs
- MCP servers: ui-docs
- MCP servers: cluster
- MCP servers: outline
- MCP servers: grimoire
- MCP servers: drawio
- MCP servers: overleaf
- MCP servers: n8n
- MCP servers: playwright

The memory bank and the GitHub surfaces come first — the critique and flow
waves read and write through them — then `hindsight` (decision 5: read-only
for agents), the read-only servers, then the writing ones.

### Wave 7 — the guidance skill sets

- skills/workstation: agent-session-bootstrap
- skills/workstation: kb-first
- skills/workstation: token-economy
- skills/repo: agent-session-bootstrap
- skills/repo: kb-first
- skills/repo: token-economy
- Older issues: #286
- Older issues: #288
- Older issues: #289
- Older issues: #290
- Older issues: #291

`kb-first` rides after Wave 6's memory servers; `memory-private` stays
outside (decision 5).

### Wave 8 — the interactive skills as human-gate workflows (decision 7)

- new: grill-me (loop)
- new: grill-with-docs (loop)
- new: brainstorming (loop)
- new: to-spec (loop)
- new: to-tickets (loop)
- skills/workstation: pr-composer (loop)

The five interactive skills land per the section above;
`skills/workstation: pr-composer` rides here because its confirmation gate is
the same human-gate pattern, and its edit-and-confirm loop unrolls the same
way.

### Wave 9 — the repo-specific workflows and research (decision 1)

- skills/repo: add-public-service
- skills/repo: deploy-status
- skills/repo: fleet-change
- skills/repo: open-pr
- skills/repo: prepare-rollback
- skills/repo: run-tests
- Older issues: #285

### Stays outside

Every `stays outside` row of the inventory, for the record — none of these
sits in a wave:

- skills/workstation: council
- skills/workstation: delegate
- skills/workstation: model-gateway
- skills/repo: council
- skills/repo: council-amend
- skills/repo: council-design
- skills/repo: council-grill
- skills/repo: council-inject
- skills/repo: council-run
- skills/repo: council-supervise
- skills/repo: council-triage
- Hooks: critique-commit-gate
- Hooks: critique-pr-gate
- Hooks: critique-session-start
- Hooks: hindsight recall hooks
- Hooks: security-guidance plugin hooks
- Roles: council role prompts
- Roles: codex-med
- Roles: codex-xhigh
- MCP servers: memory-private
- MCP servers: memory-mcp
- MCP servers: vuetify
- Older issues: #198
- Older issues: #287
- Older issues: #292

## Owner decisions

The owner answered this inventory's questions on 2026-11-09, in the comment
on #374. The decisions, verbatim, dated 2026-11-09:

1. **Repo-specific skills are ported.** `add-public-service`, `deploy-status`,
   `fleet-change`, `open-pr`, `prepare-rollback` and `run-tests` each become a
   Gold Band workflow, scoped to their repository, in a later wave.
2. **Gold Band nodes replace the model gateway and delegation.** Workflows
   bind an agent and a model per node. `model-gateway` and `delegate` stay
   CLI-only and are not ported. The Vibe delegate agents become Gold Band
   agents.
3. **The Node runners are rewritten in Rust.** `critique.mjs`,
   `build-tracker.mjs` and `model-gateway.mjs` (the last only where a
   workflow needs it) are reimplemented in the Rust workspace (#331), and
   templates call the crates.
4. **The git hooks still enforce the gates.** The commit gate and the PR
   gate are git- and GitHub-side, so they hold whatever app makes the
   commit. Gold Band adds a gate-report node (#353) for visibility only.
5. **Memory:** the `hindsight` MCP server is ported, read-only for agents.
   `memory-private` stays outside.
6. **Council:** the whole council family stays outside.
7. **Interactive skills** (grill-me, grill-with-docs, brainstorming,
   to-spec, to-tickets) map to human-gate nodes, as in Workflow C (#362):
   each question round is a manual-check node, and the human answers in the
   app or over IM.
8. **The ADR-0003 review stages** (`/critique-adrs`, `/recover-decisions`,
   `/review-architecture`) are in this epic. They get tickets that build
   them directly as Gold Band templates.

Question 2 of the inventory (roles in Wave 2 or their own wave) is settled
by #345: the six roles are Gold Band profiles, rendered by the
`gold-band-roles` crate. Question 1 (the reach levels per row) is settled by
#339's support record: every cell not yet verified is refused.
