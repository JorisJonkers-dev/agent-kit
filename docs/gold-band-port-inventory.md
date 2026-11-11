# Gold Band port inventory

Inventory of everything this repository ships to agent surfaces, ahead of the
Gold Band port. Each row has exactly one target (`workflow`, `role`,
`skill set`, `MCP server` or `stays outside`) and the lanes it is offered in:
`direct`, `gold-band` or `gold-band-personal`. The decisions are from spec #392.

The document is checked by `scripts/crates/inventory-check`. Run it from the
repository root:

```bash
cargo run --manifest-path scripts/Cargo.toml -p inventory-check -- .
```

## Skills

### skills/workstation

| Name | Path | What it does | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| agent-session-bootstrap | `skills/workstation/agent-session-bootstrap/` | Checklist for configuring a Claude Code or Codex session: config layers, MCP servers, memory-file hygiene, parity. | skill set | direct, gold-band, gold-band-personal | |
| build-tracker | `skills/workstation/build-tracker/` | Renders each ticket's status for a spec being implemented into a one-page summary with a dependency graph. | workflow | direct, gold-band, gold-band-personal | Runner reimplemented in the Rust workspace (#331). |
| council | `skills/workstation/council/` | Cross-model orchestrator (Claude and Codex): triage, planning, grill, design checkpoint, guarded fan-out, review council. | stays outside | direct | Whole council family stays outside (decision 6). |
| critique | `skills/workstation/critique/` | Shared bundle of the critique pipeline: runner `critique.mjs`, prompt preamble, roles, lenses, finding schema, default routing. | workflow | direct, gold-band, gold-band-personal | Runner reimplemented in the Rust workspace (#331). |
| critique-adrs | `skills/workstation/critique-adrs/` | Critique stage that reviews ADRs. | workflow | direct, gold-band, gold-band-personal | Ported as Wave 2 templates over the runner. |
| critique-stats | `skills/workstation/critique-stats/` | Reports how each critic client performs per stage from the ledger, ending with routing suggestions; writes nothing. | workflow | direct, gold-band, gold-band-personal | Runner `stats` command reimplemented in Rust (#331). |
| delegate | `skills/workstation/delegate/` | Plans work to hand to other model clients through the model gateway. | stays outside | direct | Gold Band binds agents per node; stays CLI-only (decision 2). |
| flow | `skills/workstation/flow/` | The pipeline stepper: says where the current feature stands and names the next step. | workflow | direct, gold-band, gold-band-personal | Runner `state` command reimplemented in Rust (#331). |
| kb-first | `skills/workstation/kb-first/` | Consult the memory platform before changing behaviour; capture durable lessons near task completion. | skill set | direct, gold-band, gold-band-personal | |
| model-gateway | `skills/workstation/model-gateway/` | The model gateway's runner and shipped defaults: client catalogue, difficulty tiers, caps and thresholds. | stays outside | direct | Stays CLI-only and unported (decision 2). |
| pr-composer | `skills/workstation/pr-composer/` | Composes a pull request in the repository's house style, gated on explicit confirmation before push or create. | workflow | direct, gold-band, gold-band-personal | Human-confirmation gate maps to a manual-check node. |
| projects-layout | `skills/workstation/projects-layout/` | Where clones and worktrees live: `~/Projects/<owner>/<repo>` and `~/Projects/<owner>/worktrees/<repo>/<name>`. | stays outside | direct | |
| research | `skills/workstation/research/` | Front door for research: starts a Workflow A run in the personal instance, or falls back to direct delegation when no instance is available or the repository rule refuses. | stays outside | direct | Starts the workflow from outside it (#497). |
| recover-decisions | `skills/workstation/recover-decisions/` | Recovers the decisions a repository's code embodies and no record states: critics propose them, the owner confirms, and an ADR is written only from the owner's answer. | workflow | direct | Direct lane (#480). |
| critique-plan | `skills/workstation/critique-plan/` | Critiques a refactoring plan in the feature flow, in place of the decisions stage for a refactor: the plan is drafted when none exists, critics check its steps, order and anchors, and the owner decides whatever would change the target. | workflow | direct | Direct lane (#486). |
| review-architecture | `skills/workstation/review-architecture/` | Reviews a repository's architecture against the rules it declared: critics cite the lines, facts and observable for each candidate, and the owner picks one for `/improve-codebase-architecture`; no issue is filed. | workflow | direct | Direct lane (#483). |
| review-grill | `skills/workstation/review-grill/` | Critiques a feature's decisions after grilling and before `/to-spec`: gap critics list the unasked questions. | workflow | direct, gold-band, gold-band-personal | Ported as Wave 2 templates over the runner. |
| review-implementation | `skills/workstation/review-implementation/` | Reviews one ticket's staged change before its commit: risk triage, one critic on the diff, bug findings decided by a failing test. | workflow | direct, gold-band, gold-band-personal | Ported as Wave 2 templates over the runner. |
| review-pr | `skills/workstation/review-pr/` | Reviews the whole feature diff before the pull request; accepted findings become follow-up tickets. | workflow | direct, gold-band, gold-band-personal | Ported as Wave 2 templates over the runner. |
| review-spec | `skills/workstation/review-spec/` | Critiques a spec after `/to-spec`: critics from other families review it read-only. | workflow | direct, gold-band, gold-band-personal | Ported as Wave 2 templates over the runner. |
| review-tickets | `skills/workstation/review-tickets/` | Checks a spec's tickets after `/to-tickets`: deterministic checker plus one critic linting structure. | workflow | direct, gold-band, gold-band-personal | Ported as Wave 2 templates over the runner. |
| token-economy | `skills/workstation/token-economy/` | Token-budget discipline: progressive disclosure, bounded recall, narrow MCP profiles, cache-friendly ordering. | skill set | direct, gold-band, gold-band-personal | |
| estate-project-intent | `skills/workstation/estate-project-intent/` | Writes and validates a Project's deploy-kit Intent and wires its publishing, reading the canonical docs live. | skill set | direct, gold-band-personal | Replaces two of the retired repo skills (#467). |
| estate-delivery-status | `skills/workstation/estate-delivery-status/` | Answers whether a release landed from the estate compose run, the commit status and the deployment. | skill set | direct, gold-band-personal | Replaces the six retired repo skills. |
| estate-rollback | `skills/workstation/estate-rollback/` | Plans a pause, rollback or resume of one estate Project and hands the owner the exact gh workflow run command; never dispatches it. | skill set | direct, gold-band-personal | Replaces the six retired repo skills. |
| estate-bootstrap | `skills/workstation/estate-bootstrap/` | Starts or audits a repository against the estate templates, shared workflows and standards, reporting gaps with evidence. | skill set | direct, gold-band-personal | Replaces the six retired repo skills. |

### skills/repo

| Name | Path | What it does | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| agent-session-bootstrap | `skills/repo/agent-session-bootstrap/` | Repo-source variant of the session-configuration checklist, rendered per CLI target. | skill set | direct, gold-band, gold-band-personal | |
| council | `skills/repo/council/` | The council skill's repo source (Claude and Codex targets). | stays outside | direct | Whole council family stays outside (decision 6). |
| council-amend | `skills/repo/council-amend/` | Amends an existing council brief, design, task DAG or run after feedback. | stays outside | direct | Whole council family stays outside (decision 6). |
| council-design | `skills/repo/council-design/` | Produces a council-ready design, consolidated plan and parallel task DAG from a brief. | stays outside | direct | Whole council family stays outside (decision 6). |
| council-grill | `skills/repo/council-grill/` | Stress-tests a plan, patch, task DAG or decision through an adversarial council review. | stays outside | direct | Whole council family stays outside (decision 6). |
| council-inject | `skills/repo/council-inject/` | Injects new constraints, evidence or operator guidance into an active council flow. | stays outside | direct | Whole council family stays outside (decision 6). |
| council-run | `skills/repo/council-run/` | Executes an approved council plan or task DAG through supervised workers. | stays outside | direct | Whole council family stays outside (decision 6). |
| council-supervise | `skills/repo/council-supervise/` | Supervises a single cross-CLI delegate with bounded scope and progress relay. | stays outside | direct | Whole council family stays outside (decision 6). |
| council-triage | `skills/repo/council-triage/` | Decides whether work belongs in a normal session, a council split skill, or a full council run. | stays outside | direct | Whole council family stays outside (decision 6). |
| kb-first | `skills/repo/kb-first/` | Repo-source variant of the memory-first consultation and capture skill. | skill set | direct, gold-band, gold-band-personal | |
| token-economy | `skills/repo/token-economy/` | Repo-source variant of the token-budget discipline skill. | skill set | direct, gold-band, gold-band-personal | |

## Hooks

This repository installs exactly the three critique hooks ADR-0002 allows, plus
the other hooks listed here. Hooks always stay outside Gold Band, so every row
is `stays outside` in the `direct` lane.

| Name | Path | What it does | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| critique-commit-gate | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-commit-gate.sh` | Blocks a risky staged diff before `git commit` until a passing `/review-implementation` run matches it. | stays outside | direct | Commit gate stays git-side; a gate-report node adds visibility only (decision 4). |
| critique-pr-gate | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-pr-gate.sh` | Blocks `gh pr create` or a push outside the feature branch patterns unless `/review-pr` passed. | stays outside | direct | PR gate stays GitHub-side; a gate-report node adds visibility only (decision 4). |
| critique-session-start | `manifest.yaml` (`hooks:`), `skills/workstation/critique/hooks/critique-session-start.sh` | Prints one line suggesting `/flow setup` in an opted-in repository whose machine setup is missing or failing. | stays outside | direct | |
| projects-layout-guard | `manifest.yaml` (`hooks:`) | Denies a clone or worktree outside the `~/Projects` layout. | stays outside | direct | Recorded in ADR-0009. |
| hindsight recall hooks | `registry/estate-tooling.yaml` (`coding_agents:`), wired by `installer/setup-workstation.sh` | The Hindsight coding-agent plugin's hooks, locked to recall-once at session start. | stays outside | direct | The `hindsight` server ports read-only; its hooks stay outside (decision 5). |
| security-guidance plugin hooks | `registry/estate-tooling.yaml` (`plugins: security-guidance`) | The plugin's own PreToolUse and Stop hooks for security review of generated code. | stays outside | direct | Come with the plugin. |

## Roles

| Name | Path | What it does | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| adr-reviewer | `skills/workstation/critique/roles/adr-reviewer.md` | Reviews a decision log as an official reviewer: whether each record carries its decision, never whether the choice was right. | role | direct | Direct lane (#475). |
| cold-reader | `skills/workstation/critique/roles/cold-reader.md` | Reads one rewritten decision record cold and answers what was decided, what was rejected and when to revisit. | role | direct | Direct lane (#476). |
| decision-recoverer | `skills/workstation/critique/roles/decision-recoverer.md` | Proposes the decisions a repository's code embodies and no record states, each traced to two sites of the code, as Y-statements with every clause marked observed or inferred. | role | direct | Direct lane (#480). |
| plan-critic | `skills/workstation/critique/roles/plan-critic.md` | Reviews a refactoring plan for safety and anchoring: each finding breaks one named rule at one step, with the severity the rule fixes. | role | direct | Direct lane (#486). |
| architecture-critic | `skills/workstation/critique/roles/architecture-critic.md` | Reviews a repository's design against the rules it declared and the consequences its packet shows, each finding naming the observable that would show a fix worked. | role | direct | Direct lane (#483). |
| diff-reviewer | `skills/workstation/critique/roles/diff-reviewer.md` | Reviews one ticket's staged diff for bugs a third party can check. | role | direct, gold-band, gold-band-personal | Gold Band profile, rendered by the `gold-band-roles` crate (#345). |
| feature-reviewer | `skills/workstation/critique/roles/feature-reviewer.md` | Reviews the whole feature diff against its spec and tickets. | role | direct, gold-band, gold-band-personal | Gold Band profile (#345). |
| gap-critic | `skills/workstation/critique/roles/gap-critic.md` | Lists the questions nobody asked about a feature's decisions. | role | direct, gold-band, gold-band-personal | Gold Band profile (#345). |
| refuter | `skills/workstation/critique/roles/refuter.md` | In the refute round, may only refute or downgrade an accepted critical finding, with evidence. | role | direct, gold-band, gold-band-personal | Gold Band profile (#345). |
| spec-redteam | `skills/workstation/critique/roles/spec-redteam.md` | Redteams a spec for failure modes and omissions. | role | direct, gold-band, gold-band-personal | Gold Band profile (#345). |
| ticket-linter | `skills/workstation/critique/roles/ticket-linter.md` | Lints a spec's ticket structure read-only. | role | direct, gold-band, gold-band-personal | Gold Band profile (#345). |
| council role prompts | `council/prompts/` (mirrored at `skills/workstation/council/prompts/`, pinned in `manifest.yaml` `council.files`) | Role prompts for council members: designer, planner, critic, reviewers, consolidators, triage judge, verifier, worker. | stays outside | direct | Whole council family stays outside (decision 6). |
| Vibe critic agents | `registry/generated/vibe/agents/critic-mistral-large.toml`, `critic-mistral-small.toml`, `critic-glm.toml`, `critic-deepseek-flash.toml` | Read-only critic client agents for the critique pipeline, each pinning a model and its prices. | role | direct, gold-band, gold-band-personal | Become Gold Band agent bindings (decision 2). |
| Vibe delegate agents | `registry/generated/vibe/agents/delegate-readonly.toml`, `delegate-write.toml` | Read-only and worktree-write delegate agents of the model gateway. | role | direct | Kept in the direct lane. |
| codex-med | `agents/codex-med.md` (rendered to `.claude/agents/codex-med.md`) | Claude Code subagent supervising a Codex delegate at medium reasoning effort. | stays outside | direct | Codex never appears in a Gold Band binding. |
| codex-xhigh | `agents/codex-xhigh.md` (rendered to `.claude/agents/codex-xhigh.md`) | Same supervisor at high reasoning effort for difficult reasoning or deep review. | stays outside | direct | Codex never appears in a Gold Band binding. |

## MCP servers

Declared in `registry/estate-tooling.yaml` (`mcp_servers:`, and `hindsight` via
`coding_agents:`), plus the profiles the runner image declares in
`runner/manifests/runtime/mcp/`.

| Name | Path | What it does | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| memory-api | `registry/estate-tooling.yaml` (`mcp_servers:`) | Hindsight long-term memory for the shared bank: explicit read, write and search knowledge tools. | MCP server | direct, gold-band-personal | |
| memory-private | `registry/estate-tooling.yaml` (`mcp_servers:`) | Private Memory, the personal Hindsight bank, only in vibe-private behind the passphrase gate. | stays outside | direct | Stays outside (decision 5). |
| memory-mcp | `registry/estate-tooling.yaml` (`mcp_servers:`) | Retired record: Basic Memory over the knowledge-vault; registered nowhere. | stays outside | direct | Retired record. |
| n8n | `registry/estate-tooling.yaml` (`mcp_servers:`) | n8n workflows through the instance-level MCP server, OAuth per client. | MCP server | direct, gold-band-personal | |
| n8n-mcp | `registry/estate-tooling.yaml` (`mcp_servers:`) | n8n node docs and schemas, docs-only, no API key. | MCP server | direct, gold-band-personal | |
| kubernetes | `registry/estate-tooling.yaml` (`mcp_servers:`) | Read-only cluster diagnostics: kubernetes-mcp-server run locally over stdio against the read-only kube context, read-only with Secrets denied. | MCP server | direct, gold-band-personal | |
| github | `registry/estate-tooling.yaml` (`mcp_servers:`) | GitHub issues, branches and pull requests; delete and label tools excluded. | MCP server | direct, gold-band-personal | |
| grimoire | `registry/estate-tooling.yaml` (`mcp_servers:`) | Grimoire D&D 5e table companion, account-scoped token. | MCP server | direct, gold-band-personal | |
| outline | `registry/estate-tooling.yaml` (`mcp_servers:`) | Outline wiki at notes.jorisjonkers.dev, as a separate Outline user. | MCP server | direct, gold-band-personal | |
| vuetify | `registry/estate-tooling.yaml` (`mcp_servers:`) | Vuetify component API; disabled on every surface today. | stays outside | direct | Stays outside. |
| playwright | `registry/estate-tooling.yaml` (`mcp_servers:`) | Headless Chromium browser automation via playwright-mcp. | MCP server | direct, gold-band-personal | |
| drawio | `registry/estate-tooling.yaml` (`mcp_servers:`) | Native .drawio diagram authoring and export via drawio-mcp. | MCP server | direct, gold-band-personal | |
| overleaf | `registry/estate-tooling.yaml` (`mcp_servers:`) | Self-hosted Overleaf: pull, push, compile and review comments via olcli-mcp. | MCP server | direct, gold-band-personal | |
| hindsight | `registry/estate-tooling.yaml` (`coding_agents:`), registered by `installer/setup-workstation.sh` | Hindsight recall-inject memory server for Claude Code and Codex. | MCP server | direct, gold-band, gold-band-personal | Ported read-only for agents (decision 5). |
| git-host | `runner/manifests/runtime/mcp/claude-mcp-servers.*.json`, `codex-mcp-servers.*.toml`, via `bin/gh-mcp-wrapper` | GitHub MCP for the runner image through the wrapper and its gh-app token. | MCP server | direct, gold-band, gold-band-personal | |
| code-intel | `runner/manifests/runtime/mcp/*code-intel*`, `*cluster*`, `*full-diagnostic*` | LSP-backed semantic code intelligence (serena), no-memories mode. | MCP server | direct, gold-band, gold-band-personal | |
| frontend-docs | `runner/manifests/runtime/mcp/claude-mcp-servers.full-diagnostic.json`, `...cluster.json`, cod equivalents | Frontend docs server via a build-time `@FRONTEND_DOCS_MCP_URL@` placeholder. | MCP server | direct, gold-band, gold-band-personal | |
| ui-docs | same runner profiles | UI docs server via a build-time `@UI_DOCS_MCP_URL@` placeholder. | MCP server | direct, gold-band, gold-band-personal | |
| cluster | same runner profiles (plus `claude-mcp-servers.cluster.json`) | Cluster MCP via a build-time `@CLUSTER_MCP_URL@` placeholder. | MCP server | direct, gold-band, gold-band-personal | |

## Planned

Rows for work with no directory yet.

| Name | Path | What it does | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| grill-me | (no directory yet) | Grills the user on a plan, decision or idea. | workflow | direct, gold-band, gold-band-personal | Human-gate workflow: each question round is a manual-check node, and the human answers in the app or over IM. Also kept in the direct lane. |
| grill-with-docs | (no directory yet) | Grilling that writes answers that change a decision into the decision docs. | workflow | direct, gold-band, gold-band-personal | Same human-gate mapping as grill-me. Also kept in the direct lane. |
| brainstorming | (no directory yet) | Explores user intent, requirements and design before implementation. | workflow | direct, gold-band, gold-band-personal | Same human-gate mapping; each exploration round is one manual-check node. Also kept in the direct lane. |
| to-spec | (no directory yet) | Turns decisions into a spec through question rounds. | workflow | direct, gold-band, gold-band-personal | Same human-gate mapping; each question round is one manual-check node. Also kept in the direct lane. |
| to-tickets | (no directory yet) | Cuts a spec's tickets through question rounds. | workflow | direct, gold-band, gold-band-personal | Same human-gate mapping; each question round is one manual-check node. Also kept in the direct lane. |
