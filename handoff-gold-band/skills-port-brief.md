# Brief: skills port to Gold Band (third sibling spec, depends on #327 and #350)

Owner decisions already made (invent no others):
- Owner said: "We wish to port all our skills, with new special considerations, and special ACLs, and custom installs, into workflows and dynamic workflows on goldband ... custom roles, skills, mcps, and more all structured around goldband." Also: all previous specs and parts should be done for Gold Band.
- Everything runs on Gold Band. No Gold Band Rust changes; gaps closed with shims (gate wrapper, one agent id per reach level and skill set, unrolled loops). New code is Rust under the 7-day crate cooldown.
- Each current skill, hook, role file and MCP server gets exactly one target: workflow, role, skill set, MCP server, or stays outside. Ticket #351 builds the inventory; this spec cuts the waves from it.
- Skill sets are per Gold Band agent id (Gold Band writes skills per agent directory, not per node). ACLs are reach levels: read, read-web, research, write-scoped, write, full, run-one-command. Two enforcement layers: tool configuration and sandbox-exec. Web cannot be blocked at the OS layer.
- Wave order: the three workflows first (done in #350). Then critique stages (#363, Wave 2). Remaining waves are decided from the #351 inventory.
- Hooks and plugins: Gold Band has none; hooks stay in the Claude Code and Codex homes ("stays outside").
- The critique runner stays the executor and keeps its guarantees (read-only proof, packet confinement, timeouts).
- Opus is used in judgment nodes only. Codex banned, no OpenRouter, no opencode/ Zen models.
- Skills on disk today: skills/workstation: agent-session-bootstrap, build-tracker, council, critique, critique-stats, delegate, flow, kb-first, model-gateway, pr-composer, review-grill, review-implementation, review-pr, review-spec, review-tickets, token-economy. skills/repo: add-public-service, council (+amend, design, grill, inject, run, supervise, triage), deploy-status, fleet-change, open-pr, prepare-rollback, run-tests, kb-first, agent-session-bootstrap, token-economy. Open issues: estate skills #285-#292, critique pipeline #167/#181/#182/#214 and children, hooks #198.
- Older issues that overlap: #285 (research delegation) is Workflow A; #198 hooks stay outside; #24 review harness is Workflow B.
Unknown, mark as open questions: wave order after Wave 2, which council skills port, whether delegate/model-gateway become workflows or stay as skills, how interactive skills (grill, brainstorm) map when Gold Band has no interactive node except manual_check.
