# Gold Band reference (owner-supplied notes; repo diodeme/Gold-Band v0.19.1, commit 48ebd69, 2026-11-03)

Treat as the only source about Gold Band. Not verified by us. Where a claim matters, say "per the reference".

## Overview
- Local-first desktop client for coding agents: direct agent chat plus workflow orchestration. Every agent runs as an ACP (Agent Client Protocol) subprocess. Rust core, Tauri 2 shell, React frontend. Win/macOS/Linux. AGPL-3.0-only. Developer Preview. ~37 stars, 3 forks. Docs/community mostly Chinese; READMEs and release notes bilingual zh/en. Accepted into OpenAI Codex open-source program. Frequent releases (0.17-0.19 in Oct 2026). Tests: Rust suites for ACP lifecycle, DSL, profiles, memory.
- Sits ABOVE agents: launches them, sends prompts, renders output, coordinates several agents. Does not reach inside an agent's loop; agent-internal features (sub-agents, hooks, the agent's own skills) stay the agent's responsibility.
- Motivation: orchestrators inside one agent session drift on long tasks; acceptance, verification and recovery should be enforced by the runtime, not the model. Authors' benchmarks: WORKFLOW and AUTO beat a single agent on long tasks; on short/medium tasks equal quality but more tokens and time.

## Run modes
- DIRECT: plain conversation with one agent, no injected system prompt; session persistence, attachments, model/reasoning/permission selection, slash commands, prompt queueing, token+time metrics.
- WORKFLOW: deterministic graph in a JSON Workflow DSL. Worker node = one agent execution (own agent, model, role, goal, output contract, success condition). Edges route on success/failure, choose new vs continue session. Fix-up loops bounded by max_attempts; full restarts (rounds) bounded by max_rounds. Manual-check nodes pause for a human decision. Running config editable for not-yet-dispatched nodes only.
- AUTO (ai-dynamic node): AI planner proposes subsequent nodes: splits goal, fans out parallel subtasks (each in its own git worktree created from committed HEAD), merges, adds acceptance nodes incl. adversarial verification. Runtime validates proposals against limits and owns state. Can call saved workflows as sub-workflows (allowedWorkflows).

## Agents (resources/agent-catalog.json)
claude-acp (.claude; system prompt via ACP yes), codex-acp (.codex +.agents), cursor, gemini, codebuddy-code, goose, qwen-code, opencode (`opencode acp`, .opencode +.agents), kimi, amp-acp, pi-acp. Only Claude Code and Codex are fully tested. ACP registry snapshot lists ~40 more incl. `mistral-vibe`, `antigravity-acp`, github-copilot-cli, cline, junie, kilo. Hermes is in neither list: custom ACP agent only. Any ACP-speaking command can be added as a custom agent (command, args, env, agent dirs). For agents that cannot take a system prompt over ACP, the role text is prepended to the user prompt. "Doctor" diagnostics per agent. Idle session reclaim (>10 min idle; >8 idle). Built-in agents' command/args are owned by the catalog (read-only).

## Workflow DSL (web/src/types.ts WorkflowDsl; validator src/dsl/mod.rs)
Top level: {version:"0.1", id, entry, control:{max_attempts,max_rounds}, nodes[], edges[]}; terminal target `$end`.
Worker node fields: type:"worker", id, provider (agent id), model (as advertised over ACP), profile (role id), goal, permission_mode (ACP mode id), auto_accept, config_options (e.g. {"effort":"high"}), output {kind:"json", artifact, schema(JSON Schema)}, success_condition ({path,equals} or {expression:"$.path >= 3"} ops >=,<=,!=,==,>,<), manual_check, executionSlotId.
Edge: from, to, on ("success"|"failure", one edge per outcome per node), session ("new"|"continue"), new_round_entry ("$entry" or node id).
Model bindings: template carries model_bindings {definitionRevision, bindingRevision, bindings[{executionSlotId, agentId, modelId, permissionModeId, autoAccept, configOptions}]} separating which agent/model fills a slot from the graph.
Template record: {id,name,isBuiltIn,optionalEntryStage,workflow,modelBindings,createdAt,updatedAt}. Built-ins: `default` and a lightweight variant.
Validation: version "0.1"; non-empty id/entry; no duplicate or reserved node ids; entry and all edge endpoints exist; terminal cannot be an edge source; at most one edge per outcome per node; max_attempts/max_rounds positive; success expression non-blank and path must match output schema; `session: continue` only toward a real node whose agent supports continuation.

## AUTO config
ai-dynamic node: agentStrategy fixed|dynamic, allowedProfiles, allowedWorkflows, globalGoal, configOptions, modelBoundOverrides, control {maxDynamicNodes,maxFanout,maxDepth,maxParallel,maxGroupDepth,maxWorkflowInvocations,allowNestedDynamic}.
Fixed: one provider/model for everything. Dynamic: bootstrapProvider/Model, acceptanceModel, permissionMode, routingPrompt (natural-language routing), availableAgents[{provider,model,permissionMode,configOptions}]. Planner picks provider per worker from availableAgents guided by routingPrompt; merge and acceptance always run on the bootstrap agent; planner never outputs model/permissionMode (runtime takes them from saved config). Demo: codex-acp running a non-OpenAI model (deepseek-flash) by pointing the agent at another provider through its own config.

## Profiles (roles)
Markdown with YAML frontmatter in ~/.gold-band/context/profiles/<name>-<id>.md; frontmatter id, name, summary, createdAt, updatedAt, dynamicTemplate (renders body with runtime context: execution surface, session mode new|continue, output-contract present). Claude gets the profile appended to its system prompt; others get it in the user prompt. Built-ins (read-only, localized): pf-builtin-plan, -dev, -dev-test, -review, -test, -accept, -cleanup, -interview, -grill (relentless interrogation writing a consensus doc), -cicd. Only scopes: built-in and user. NOT per repository.

## Skills
Standard SKILL.md. Global ~/.gold-band/skills/<skill>/, project <repo>/.gold-band/skills/<skill>/. Gold Band SYMLINKS skills into each configured agent's own skills dir (from primaryAgentDir, e.g. .claude/skills, .codex/skills, .pi/agent/skills), reconciles/cleans links, detects name conflicts. Per agent: primaryAgentDir (written), projectPrimaryAgentDir (optional), compatibleAgentDirs (read, never written). Skills can be targeted at selected agents.

## MCP servers
Stored in settings.json `contextServers`, injected into ACP sessions according to each agent's advertised MCP capabilities. Records: stdio {command,args,env}, http {url,headers,oauth{clientId,clientSecret}}, sse. Health checks: healthy|unhealthy|auth_required|unknown plus discovered tool list. Own built-in memory MCP server (`--gold-band-memory-mcp`): per-workspace and per-task parameter store, explicit create/update/delete.

## Models, usage, observability
Models/reasoning levels discovered from what each agent advertises over ACP. Usage per session/node/attempt: input, output, cache-read tokens, elapsed time, context occupancy, per-model usage. NOT tracked: monetary cost, provider quotas/rate limits, per-API-key spend. Observability: messages, reasoning, tool calls, injected system prompt, raw ACP frames, artifacts, per-turn diffs, rounds, attempts; graph view.

## Other features
Scheduled tasks (at/repeat/every/cron, time zones) triggering DIRECT, WORKFLOW or AUTO. WeCom and Feishu connectors for remote permission/manual-check intervention. Workspace tooling: file tree, git (worktrees, PRs, issues), built-in browser. Optional Multica integration. Peers named by authors: AionUi, Paseo (closest), Multica.

## Storage and config-as-code
User level ~/.gold-band/: settings.json (agents, MCP servers, prefs, camelCase), context/workflows.json (template store), context/auto-templates.json, context/profiles/*.md, skills/, desktop/agent-model-catalogs.json, logs/runtime.log, projects/<project-id>/ (tasks/<task>/runs/<run>/rounds/<round>/nodes/<node>/attempt-<n>/ artifacts, SQLite DBs, worktrees). Repo level <repo>/.gold-band/: skills/ only.
Agent entry in settings.json: {adapter:{command,args,displayName,env}, icon, primaryAgentDir, projectPrimaryAgentDir, compatibleAgentDirs, systemPromptDelivery, externalSessionSyncSupported/Enabled}; ids lowercase ascii + digits + `-`; for built-in ids command/args are stripped on save and re-injected from catalog on load.
Config-as-code behaviour: all plain files (JSON, Markdown+YAML, SKILL.md dirs), versionable and generatable. BUT: the app REWRITES workflows.json on every load (migrates format, re-inserts/updates built-ins); NO file watcher / live reload; strict Rust validation with NO standalone CLI validator (malformed JSON surfaces as an error in-app); workflows, AUTO templates and profiles exist only at user level, not per repo; built-ins are read-only (edits saved as new user items).
Built-in prompts in src/prompts/<lang>/ (runtime/ai-dynamic/* planner, fan-out, merge, acceptance, output protocol, proposal repair; profile/*.md).

## Limitations
Developer Preview, UI/behaviour changes fast. Only Claude Code and Codex fully tested. No routing proxy, API-key pooling or cost tracking: credentials and providers stay in each agent's own config/env. Cannot steer an agent mid-loop beyond ACP (cancel, follow-up, mode/config changes). ACP has no "background task finished" signal (assumed after 10 s idle). AUTO planning stability on complex tasks still being refined. Docs/community mostly Chinese.
