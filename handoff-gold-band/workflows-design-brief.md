# Design brief: delegation workflows on Gold Band (owner decisions, 2026-11-06)

Settled by the owner in a grilling session. Sibling spec to #327 (Gold Band adoption). Source facts: reports/gb-dsl-facts.md.

## Direction
- Everything runs on Gold Band: the three new workflows, the existing critique stages, and later all skills (roles, skill sets, MCP servers, ACLs/reach levels, custom installs) ported in waves. First wave: the three workflows. Second: critique stages (/review-spec, /review-tickets, ...). Later: remaining skills, after an inventory.
- No change to Gold Band's Rust. Gaps are closed with shims: tiny cheap worker nodes plus pinned wrapper scripts.
- New code (wrappers, validators) is Rust under the 7-day crate cooldown (standards ADR-0014).
- Budgets: LiteLLM per-key max_budget (Vibe 255 EUR, Vibe-study 128 EUR, OpenCode Go 60 EUR). Claude on Max x5 is not metered; keep Claude use to judgment nodes. Gold Band has no cost cap and its token metrics regressed to 0; do not rely on them.
- Pi (built-in pi-acp agent) reaches Vibe and OpenCode Go models via LiteLLM.
- Codex banned, no OpenRouter, no opencode/ Zen models.
- Private research papers may be read by any cheap worker (owner accepted provider exposure).

## Gold Band facts that shape the design
- Loops: a worker with output + success_condition is a judge; its failure edge may point to any earlier node; control.max_attempts bounds repairs, control.max_rounds bounds $new-round rounds. No generic loop counter.
- Static DSL is sequential (one success and one failure edge per node). Parallelism only inside ai-dynamic nodes (maxFanout 5, maxParallel 3 defaults; worktree per branch; merge + acceptance nodes; one result artifact).
- No command/script node, no per-node skills, cwd or tool allow-lists (only permissionMode, autoAccept). Reach and skill sets are therefore carried by one Gold Band agent id per (agent, reach level, skill set).
- Headless: gold-band run --requirement-file X (--workflow J | --auto-config J), gold-band continue; exit 0 ok, 1 failed, 2 paused, 3 command error, 4 not settled. manual_check pauses (exit 2); resolved in the app or IM.
- No plugin/hook system; node kinds are a closed enum.

## Gate shim (tests, coverage, check-tickets, crate-age)
Gate node = cheap worker whose agent id may only run one pinned wrapper (reach: run-one-command, sandbox-exec). The wrapper writes a verdict JSON (command, exit code, output hash, timestamp) into a directory the agent cannot write. A second tiny read-only node copies that file into its success_condition artifact; a mismatch or missing file fails. The model never authors the verdict.

## Workflow A: deep research
- Orchestrated by Opus only at the start (targeted prompt + inputs) and acceptance.
- Planner node states the source list and questions.
- ai-dynamic node fans out fetchers (Pi, read-web reach, cheap models across Vibe/Vibe-study/OpenCode Go keys via LiteLLM), up to maxFanout.
- Merge/organise: cheap workers write into the research repo in the estate-kit-papers layout: catalog.json single source of truth, generated INDEX.md, notes/, evaluations/. Write scope: research/ only (write-scoped).
- research/ is a git submodule of a private separate repo.
- Quote check and source-tier rules from the papers repo apply; a read-only verifier node checks quotes against fetched pages.

## Workflow B: implementation (one ticket, from requirement.md)
1. Test author: a model family different from the writer writes failing tests from the ticket's acceptance criteria first.
2. One writer only (Claude Sonnet) implements until the tests pass.
3. Deterministic gates via the gate shim: build, tests, lint, coverage = 100% lines on changed files, crate-age for Rust.
4. Judges, in parallel inside one ai-dynamic node: code-quality judge and tests-vs-spec judge. At least two judge families, each different from the writer's family (for example GLM and DeepSeek). Judges return findings in the finding schema.
5. Any failed judge or gate sends back to the writer with the critiques. Cap: 2 repairs (max_attempts = 2), then an Opus judgment node, then manual_check by the human.
6. Opus is used only for judgment nodes: escalation and final acceptance.

## Workflow C: grill to tickets (all inside Gold Band)
Grill (manual_check per question round) -> critique -> spec -> critique (review-spec equivalent) -> tickets -> critique (review-tickets equivalent) -> publish tickets and sub-issues. manual_check on: grill answers, needs-human findings, ticket publication. Output feeds workflow B: a wrapper exports the issue and sub-issues to requirement.md (agents never need GitHub write reach).

## Per-node restrictions
Reach levels: read, read-web, research (web + scoped write), write-scoped, write, full, plus run-one-command for gates. Each Gold Band agent id fixes one reach and one skill set. Two enforcement layers: tool config per agent and sandbox-exec. Web cannot be blocked at the OS layer.

## Open for the spec
Port inventory of existing skills, hooks, roles and MCPs; Gold Band gaps recorded for possible upstream issues (command node, static fan-out, budgets); LiteLLM key layout per agent id; wrapper binary names and sandbox profiles; what happens to the Gold Band `ai-dynamic` agent proposing fan-out versus fixed fan-out.
