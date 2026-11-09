# Spec: agent-kit on Gold Band — two instances, one backlog

This spec replaces every open agent-kit epic and spec: #368, #326, #327, #350,
#367, #111, #113, #167, #181, #182, #214 and #286. It accounts by name for
every open issue and every open pull request in this repository. The
Coverage section at the end lists each one, with its branch, its disposition
and the user stories that carry it. Each open ticket, except the Renovate
Dependency Dashboard (#33), is rewritten as a ticket under this spec, closed
as done, or closed as superseded. The old issues are closed as "superseded by" their new
ticket, with a mapping from old ticket to new ticket. The owner's decisions
come from the grilling session of 2026-11-09. The ADRs that record the
hard-to-reverse ones are ADR-0011 (two Gold Band instances), ADR-0012 (the
Nedap bank) and ADR-0013 (the operating-system write limit for each OS). The
words used below are defined in `CONTEXT.md`.

## Problem Statement

The owner wants as much of their agent work as possible to run through
Gold Band workflows: research, implementing one ticket, grilling an idea into
tickets, and the critique stages. Work for the employer still has to be
possible, both through agents started directly and, from now on, through
Gold Band itself. Today the work towards that goal has several problems:

- **It is spread out.** Twelve epics and specs, more than a hundred open
  issues, sixteen open PRs and a 36-commit integration branch exist, and none
  of them gives one complete picture. Several of them contradict each other.
  On 2026-11-09 the owner said that the six old repo skills are ported and
  that the ADR stages are built only as Gold Band templates. Older specs
  retire those skills, and build those stages in the critique runner, where
  work repositories can use them.
- **It assumes one instance with no employer work.** The specs ban Codex from
  Gold Band and say Gold Band never touches a work repository. The owner now
  wants a work Gold Band with the work Claude agent and Codex, beside a
  personal Gold Band with every other agent.
- **It assumes one Mac.** Every Gold Band result so far was proven on the
  owner's Mac. The specs name the operating-system write limit after
  `sandbox-exec`, put secrets in the macOS keychain and hard-code the macOS
  application path. The owner also runs a Linux workstation, which today has
  no Gold Band, no Pi, no LiteLLM and no secret store besides a plain file.
- **Memory mixes employer and personal facts.** Shared Memory holds facts
  about employer repositories, and every personal agent can recall them.
- **Finished work is still open.** Some delivered work is still open, and
  some open work is stale, so the backlog hides what is really left.

## Solution

agent-kit treats Gold Band as two surfaces, the **work instance** (`gold-band`)
and the **personal instance** (`gold-band-personal`). Beside them is the
**direct lane**, which is unchanged and stays available on every repository.

- **Work instance.** It holds only the work Claude agent and Codex. It runs
  only on repositories with a `nedap`-owned remote and only on the work
  machine. Every run that binds Codex is refused unless the model gateway
  holds a live grant for that run, and the grant is checked again before
  every Codex node. It runs the same Gold Band
  workflows as the personal instance, with two model families (Claude and
  Codex), so author and reviewer always differ. Its research results stay
  inside the work repository. Its tickets go to the work repository's GitHub
  issues, through a per-repository tracker setup. It reads and writes only the
  Nedap bank.
- **Personal instance.** It holds every declared agent except Codex and the
  work Claude agent: the personal Claude agent, OpenCode, Vibe, Vibe-study,
  Pi, Antigravity and Hermes. It refuses `nedap`-owned repositories and allows
  a repository with no remote. It runs on the Linux workstation and on the
  work machine. On the work machine it runs under a second macOS user
  account. That account keeps its data, keychain and keep file apart, and
  cannot read the employer clones of the work account. It reads and writes Shared Memory. Its cheap agents reach
  their models through one LiteLLM proxy in the estate cluster, capped by
  per-key budgets.
- **Direct lane.** It keeps the model gateway, `delegate`, the council, the
  critique runner, the interactive skills and every installed skill and MCP
  server. Nothing is removed from it because the same thing now also exists
  in an instance.

One setup run converges both instances on each machine, with the same rules as
every other surface: declared entries, retired records, undeclared entries and
the keep file. It builds Gold Band from the pinned, audited source, both the
desktop app and the headless binary. It writes nothing while an instance is
running. Every agent id carries a reach level. A launcher holds the agent to
that reach level in two ways: through the agent's tool configuration, and
through an operating-system write limit (Landlock on Linux, `sandbox-exec` on
macOS). The launcher refuses to start an agent it cannot limit. Secrets come
from one secret-store interface (the macOS Keychain, or libsecret on Linux).
No secret is written into any Gold Band file.

The Gold Band workflows are:

- research (Workflow A);
- implementing one ticket (Workflow B);
- grilling an idea into tickets (Workflow C);
- the critique stages (Wave 2), which run the critique runner behind a gate
  shim so that the runner keeps its guarantees;
- the later port waves, cut from the port inventory.

Each Gold Band workflow is proven headless against stub agents before any
model is spent. Only judgment nodes (an id ending in `-escalation`,
`-acceptance` or `-planning`) may use Opus, and only `-acceptance` and
`-escalation` nodes may write memory.

Agents also get a cheaper way to find code. The first step is measuring what
they use today. Then every repository gets an `AGENTS.md`, and a small
read-only code-navigation server searches the live working tree, gives
outlines and repository maps, and stays inside each instance's repository
rule. An index is added only if the measurements show it is needed.

The backlog is rebuilt as code. The spec and a ticket graph live in this
repository. A Rust sync tool creates the issues from them, links the
sub-issues and "blocked by" edges, and closes the superseded issues with their
mappings. It shows a dry-run plan first and changes GitHub only after the
owner approves. One new build tracker shows every ticket of this spec.

## User Stories

### The two instances and their boundaries

1. As the owner, I want a work instance named `gold-band` and a personal instance named `gold-band-personal`, so that the names follow the Claude profiles (the unsuffixed profile is work, `-personal` is personal), not the Vibe launchers' account suffix, where no suffix means the personal account.
2. As the owner, I want the work instance to hold only the work Claude agent and Codex, so that employer code reaches only employer-paid models.
3. As the owner, I want the personal instance to hold every declared agent except Codex and the work Claude agent, so that personal work never runs on an employer account.
4. As the owner, I want the work instance to refuse any repository whose remote is not owned by `nedap`, and any repository with no remote, so that it touches only employer repositories.
5. As the owner, I want the personal instance to refuse any repository with a `nedap`-owned remote, so that employer code never reaches a personal provider.
6. As the owner, I want the personal instance to allow a repository with no remote, so that I can bootstrap a new personal repository; I accept that I must be mindful there.
7. As the owner, I want the repository rule and the family allowlist (including its code and history permissions) checked both when setup binds a Gold Band workflow and again by a run-start shim before any model node runs, so that a remote or allowlist entry changed after setup still refuses the run.
8. As the owner, I want the work instance installed only on the work machine, so that the Linux workstation never holds employer configuration.
9. As the owner, I want the personal instance installed on the Linux workstation and on the work machine, so that I can use it on both.
10. As the owner, I want the two instances on one Mac to keep separate data, settings, agents and logs, so that a run in one can never see the other's state.
11. As the owner, I want the personal instance on the work machine always to run under a second macOS user account, with its own data directory, keychain, keep file and overlay, and setup run once in each account, so that the split holds whatever the app supports and personal agents cannot read employer clones.
12. As the owner, I want no keep-file entry able to keep Codex or the work Claude agent on the personal instance, so that the boundary cannot be broken by one file.
13. As the owner, I want the registry to declare only the generic shape of the work instance (`claude-work`, `codex`), so that nothing about the employer enters the public registry.
14. As the owner, I want employer-specific work-instance settings, such as work MCP servers, in a per-machine overlay beside the keep file, so that they are repeatable on my machine but never published.
15. As the owner, I want setup to refuse a registry that names Codex, the work Claude agent or a `nedap` binding for the personal instance, so that the rule is enforced where it is declared.
16. As the owner, I want setup to refuse a registry that names any agent other than the work Claude agent and Codex for the work instance, so that no cheap or third-party provider sees employer code.

### Codex in the work instance

17. As the owner, I want each work-instance run to count as one gateway workflow (`<gold band workflow>@<repository>@<run id>`), refused unless it holds its own live grant, with the grant's cap counted in Codex node starts because the gateway never sees Gold Band spend, so that Codex spend stays under the grant rule (ADR-0008, as applied to Gold Band by ADR-0011).
18. As the owner, I want the grant read from the gateway's own record, not a copy, both at run start and by a grant-check node before every Codex node, so that a grant that lapses mid-run (by time or cap) fails the next Codex node and pauses the run for a manual check.
19. As the owner, I want a refused run to say which grant is missing and how to ask for it, so that I can fix it in one step.
20. As the owner, I want grants still asked for and recorded only through the gateway, so that agents keep asking before they write a grant.

### Setup and convergence

21. As the owner, I want one setup run to converge every instance that belongs to the account it runs in (on the work machine, one run in each of the two accounts), so that a fresh machine matches the registry.
22. As the owner, I want a second setup run to change nothing, so that I can run setup whenever I like.
23. As the owner, I want setup to skip an instance that is not installed, create nothing for it and say so, so that a machine without it still sets up cleanly.
24. As the owner, I want setup to build the Gold Band desktop app and the headless binary from the pinned, audited source on Linux and macOS, so that I never run an unknown build.
25. As the owner, I want every crate in the Gold Band lockfile checked against the 7-day rule before the build, so that the audited build is also a cooled-down build.
26. As the owner, I want the pin to stay on the audited commit until a newer upstream release is at least 7 days old and has been audited, so that an upgrade is a reviewed change.
27. As the owner, I want setup to report each instance's version and build commit, so that I can tell the audited build from any other.
28. As the owner, I want setup to refuse to write while an instance is running, and to say which instance to quit, so that the app does not overwrite the result.
29. As the owner, I want the running-app check to work on Linux and macOS without a hard-coded application path, so that the guard holds on both machines.
30. As the owner, I want setup to change only the keys it declares in an instance's settings and the files it generates, so that my choices in the app survive.
31. As the owner, I want each settings file replaced atomically, with one backup, so that a failed write leaves the old file in place.
32. As the owner, I want an undeclared agent, MCP server or role removed from an instance unless the keep file names it, so that both instances follow the same rule as every other surface.
33. As the owner, I want a retired record removed from both instances, so that retired things do not linger.
34. As the owner, I want setup never to enable the updater, metrics, feedback, the chat connectors or the task-board integration in either instance, so that the audited, quiet configuration stays that way.
35. As the owner, I want each instance's data directory readable only by my account, so that logged prompts and code are not readable by other users.

### Agents, identities and launchers

36. As the owner, I want the work Claude agent always to use the work configuration directory, whatever the ambient environment says, so that identity cannot switch by accident.
37. As the owner, I want the personal Claude agent always to use the personal configuration directory, so that personal work stays on the personal account.
38. As the owner, I want setup to install the work Claude profile into the home directory's default Claude directory whatever `CLAUDE_CONFIG_DIR` says, so that direct use and the work instance agree.
39. As the owner, I want Vibe and Vibe-study in the personal instance, with Vibe-study reading its key from the secret store at launch, so that no key is written into any Gold Band file.
40. As the owner, I want the Vibe-study launcher to exit with a message and start nothing when its key is missing, so that a misconfigured launch never falls back to another account.
41. As the owner, I want Pi installed from pinned versions without install scripts, and recorded as throwaway-repositories-only, so that its lack of permission prompts is a visible decision.
42. As the owner, I want OpenCode, Antigravity and Hermes declared with the launch command each provides on the current OS, so that no macOS-only path is written on Linux.
43. As the owner, I want an agent that is not installed on a machine skipped and reported, so that the rest of the instance still converges.

### Reach levels and the write limit

44. As the owner, I want every agent id to carry a reach level, so that a node's reach is visible in the registry.
45. As the owner, I want one agent id for each agent, reach level and skill set, so that a node gets exactly the reach of the id it names.
46. As the owner, I want the launcher to refuse a reach level the agent cannot enforce, so that a node never runs with more access than declared.
47. As the owner, I want each reach level's writable paths fixed by one table (ADR-0013) and enforced by the operating system, not only by the tool settings, with `write` unable to change `.git/hooks` or `.git/config`, so that a gap in a tool list does not matter and no agent can switch off the repository's hooks.
48. As the owner, I want the write limit to use Landlock on Linux and `sandbox-exec` on macOS behind one interface, so that the same declaration means the same thing on both machines.
49. As the owner, I want the launcher to refuse to start an agent when the write-limit backend for the current OS is unavailable, so that no agent runs unlimited.
50. As the owner, I want it written down that neither backend limits reads or the network, and that the account boundary is what keeps personal agents out of employer clones on the work machine, so that nobody treats a read-only node as offline or confined.
51. As the owner, I want each cell of the support record (agent by reach level) verified on each OS before it is used, so that a level proven on the Mac is not assumed on Linux.
52. As a maintainer, I want validation to fail when an agent id has no reach level, a reach level its agent does not support on that OS, or a `write-scoped` or `research` level without an output directory, so that errors show in CI.

### Secrets

53. As the owner, I want one secret-store interface backed by the macOS Keychain on macOS and libsecret on Linux, so that launchers and MCP proxies read keys the same way on both machines.
54. As the owner, I want a launch to fail closed when the secret store is unavailable or the key is missing, so that a missing key is never replaced by a plain file.
55. As the owner, I want no secret value in any Gold Band file, and setup to refuse a declaration that contains one, so that the settings file never holds a token.
56. As the owner, I want token-bearing MCP servers declared through the secret-store proxy, so that the token is read at launch and never lands on disk.

### Roles, skill sets and MCP servers

57. As the owner, I want the declared roles rendered from agent-kit's own role and lens files, with templating switched off, so that there is one source and the braces survive.
58. As the owner, I want each skill set installed in exactly one agent id's skill directory, so that a node sees only the skills it needs.
59. As the owner, I want the same-name overlap test run in each instance before any MCP server is declared there, so that I know whether tools appear twice.
60. As the owner, I want each MCP server declared in exactly one layer for a given agent, and listed only for agent ids whose reach level allows it, so that no tool is duplicated or over-reached.
61. As the owner, I want the work instance's MCP servers to come from the per-machine overlay, so that work servers never appear in the personal instance or the public registry.

### Memory

62. As the owner, I want a Nedap bank that holds every employer fact, so that employer knowledge is kept apart from personal knowledge.
63. As the owner, I want the work instance and direct-lane sessions on `nedap` repositories to read and write only the Nedap bank, so that employer facts never reach Shared Memory again.
64. As the owner, I want the personal instance to read and write only Shared Memory, so that personal agents cannot recall employer facts.
65. As the owner, I want the existing employer facts moved from Shared Memory into the Nedap bank under the work Claude subscription, so that employer content is processed only under the employer's account.
66. As the owner, I want recall from the Nedap bank checked before the moved facts are deleted, and the same pass run over Private Memory, started by me through `vibe-private` behind the passphrase gate, so that nothing is lost and no personal bank keeps employer facts.
67. As the owner, I want the moved facts deleted from Shared Memory after that check, so that no personal bank keeps employer content.
68. As the owner, I want only nodes whose id ends in `-acceptance` or `-escalation` to write memory inside Gold Band (`-planning` judgment nodes may use Opus but not write memory), and every other node to read only, while direct-lane sessions keep writing their bank as today, so that only checked results are remembered by workflows.
69. As the owner, I want the workflow lint to refuse a memory-writing binding on any other node, so that the rule is enforced like the Opus rule.
70. As the owner, I want Private Memory reachable only from `vibe-private`, and never from either instance, so that the passphrase gate stays the only way in.
71. As the owner, I want it re-checked that a fact learned by one agent can be recalled by another agent in a fresh session, within the same bank, so that cross-agent memory is proven after the bank split.

### Gold Band workflows: shared rules

72. As the owner, I want every Gold Band workflow checked into agent-kit as a reviewed file, so that it is repeatable.
73. As the owner, I want every gap Gold Band cannot express closed by a shim (a cheap worker node and a pinned wrapper), and never by a change to Gold Band's code, so that an upgrade keeps the workflows working.
74. As the owner, I want deterministic gates (build, tests, lint, 100 percent line coverage on changed files, crate age for Rust) run through a gate shim that writes a gate report the gate agent cannot write, so that a green gate means the command ran and exited zero.
75. As the owner, I want a read-only copier node to fail the run when the gate report is missing, stale, from another attempt or does not match, so that a faked pass ends in failure.
76. As the owner, I want a hung gate command to time out into a failed gate report, so that a hang becomes a failure the workflow can route on.
77. As the owner, I want Opus only in judgment nodes (`-escalation`, `-acceptance`, `-planning`) in both instances, so that expensive judgment is spent only where it decides.
78. As the owner, I want repair loops unrolled into fixed nodes and capped at two repairs, then an Opus judgment node, then a manual check, so that every loop is bounded by the file itself.
79. As the owner, I want each Gold Band workflow proven by a headless run against stub agents before any real run, and a real run refused when that proof was skipped, so that structure and routing are known good before a model is spent.
80. As the owner, I want the work instance to run the same Gold Band workflows as the personal instance, bound only to the work Claude agent and Codex, with critique on a `nedap` repository allowed only through the work-only critic roster (Codex under a grant, and the work Claude agent, never the orchestrator's own family), so that work repositories get the same sequencing, gates and critique.
81. As the owner, I want author and reviewer families to differ in every workflow, using Claude and Codex in the work instance and the allowed families in the personal instance, so that no reviewer shares the author's blind spot.

### Workflow A: research

82. As the owner, I want Opus to state the research plan at the start and accept the result at the end, with cheap fetchers in between, so that judgment is spent only where it decides.
83. As the owner, I want fetchers fanned out in one dynamic node with explicit width and depth limits, so that fetching runs in parallel within known bounds.
84. As the owner, I want a read-only verifier to check every quote against the fetched page, so that a citation is verified, not trusted.
85. As the owner, I want personal research written into the private research repository in the papers layout, so that results land in a known shape.
86. As the owner, I want work research written into a research directory inside the `nedap` repository being worked on, so that employer research never leaves the employer's repository.
87. As the owner, I want a research skill in the direct lane that starts a Workflow A run, and falls back to direct delegation when no instance is available or the repository is not allowed, so that research has one front door.

### Workflow B: implementing one ticket

88. As the owner, I want a test author from a different family to write failing tests from the ticket's acceptance criteria first, so that the tests are not the writer's own.
89. As the owner, I want exactly one writer to implement until the tests pass, so that a ticket has one coherent implementation.
90. As the owner, I want two critics, in families that differ from the writer's, to run in parallel and return findings in the finding schema, so that findings validate against the schema agent-kit already uses.
91. As the owner, I want a finding without checkable evidence never acted on automatically, so that hints do not drive repairs.
92. As the owner, I want a "wait for CI to be fully green" gate command, with a structured verdict and a rerun of known-flaky jobs, so that Workflow B and direct sessions stop guessing about CI.

### Workflow C: grill to tickets

93. As the owner, I want Workflow C to pause on a manual check for every grill round, every needs-human finding and every publication, so that I settle every answer and every publication.
94. As the owner, I want a per-repository tracker setup that records where a repository's tickets go and with which labels, so that Workflow C publishes correctly in both personal and work repositories.
95. As the owner, I want the published ticket exported to a requirement file for Workflow B by a wrapper, so that no agent needs GitHub write access.

### Budgets and LiteLLM

96. As the owner, I want one LiteLLM proxy in the estate cluster, shared by both machines, with per-key budgets of 255 EUR (Vibe), 128 EUR (Vibe-study) and 60 EUR (OpenCode Go), so that one budget per key holds across machines.
97. As the owner, I want a budget refusal to fail the node and end the run in failure, and a stub test to prove it, so that a spent key stops the work.
98. As the owner, I want no reliance on Gold Band's own cost or token numbers, so that budget enforcement does not depend on a broken counter.
99. As the owner, I want the personal Claude agent on my Max x5 plan kept to judgment nodes, so that unmetered capacity is spent only on judgment.

### Port and inventory

100. As the owner, I want every skill, hook, role file and MCP server to carry exactly one Gold Band target (a Gold Band workflow, a role, a skill set, an MCP server, or stays outside) and its lanes, so that nothing is ported twice or missed.
101. As the owner, I want the inventory to state which lanes keep each item, so that a port to an instance never removes the direct-lane copy.
102. As the owner, I want the critique stages ported as Gold Band workflow templates whose nodes run the critique runner through the gate shim, so that the runner keeps its read-only tripwire, packet confinement, timeouts and family allowlist.
103. As the owner, I want hooks recorded as staying outside, in the Claude Code and Codex homes, so that no run depends on a hook Gold Band cannot fire.
104. As the owner, I want the interactive skills kept in the direct lane and also offered in the instances as manual-check Gold Band workflows, so that both lanes can grill and brainstorm.
105. As the owner, I want the model gateway, `delegate` and the council recorded as direct lane only, so that the inventory says where they live.
106. As the owner, I want the later waves cut from the inventory as ordered groups of rows, each row with its target decided, so that the port is schedulable and reviewable.
107. As the owner, I want the six old repo skills (`add-public-service`, `deploy-status`, `fleet-change`, `open-pr`, `prepare-rollback`, `run-tests`) retired, so that no skill describes the dead monorepo.
108. As the owner, I want four estate skills (`estate-bootstrap`, `estate-project-intent`, `estate-delivery-status`, `estate-rollback`) in the direct lane and as a skill set in the personal instance, so that every agent knows how the estate delivers and rolls back today.
109. As the owner, I want a weekly drift check that flags when a source an estate skill was checked against has moved, so that the skills do not silently go stale.

### Direct lane

110. As the owner, I want the review stages (`/critique-adrs`, `/recover-decisions`, `/review-architecture`) and the refactoring-plan critique stage of the feature flow (`/critique-plan`) finished in the critique runner, with the work-only critic roster on `nedap` repositories, so that work repositories get them from the command line.
111. As the owner, I want those stages wrapped as Gold Band workflow templates after they land, so that both instances get them without a second implementation.
112. As the owner, I want calibration in the critique stats, so that I can see whether critics are right over time.
113. As the owner, I want one real feature taken through `/flow` to a passing gate on a work repository, so that the direct-lane pipeline is proven end to end.
114. As the owner, I want the remaining critique live checks (decision, ticket and code stages, the hooks in a live Claude session) run and recorded, so that the direct lane's claims are evidence.
115. As the owner, I want the model gateway's Codex client to give a worktree worker a writable git directory and a per-repository brief, so that Codex delegations in worktrees can commit.
116. As the owner, I want the model gateway to list only the Vibe models Vibe accepts, so that delegations do not fail on an unknown model.
117. As the owner, I want Vibe headless delegations to fail fast when web tools are missing, with the workaround documented, so that they do not stall until the time limit.
118. As the owner, I want the in-cluster runner to fetch the newest kit at session start, so that the runner does not lag behind releases.
119. As the owner, I want the Node runners (critique, build tracker, gateway) ported to Rust after the stages land, behind the same commands, so that new code follows the repository's direction without blocking the stages.

### Workstation and hygiene

120. As the owner, I want the Linux workstation to reach the same agent baseline as the Mac (Gold Band, Pi, LiteLLM access, secret store), so that both machines are first-class.
121. As the owner, I want every macOS-only path in setup (the application path, Homebrew paths, launchd, the keychain calls, `~/Library`) behind an OS check with a Linux equivalent or an explicit skip, so that setup runs cleanly on Linux.
122. As the owner, I want the Playwright MCP to list its tools in every agent from one pinned binary on a cold cache, so that browser tools always start.
123. As the owner, I want the private Hindsight bank proven in Vibe and its issues closed with evidence, so that the Private Memory claims are tested.
124. As the owner, I want OpenCode in the direct lane to get the same skills and MCP servers as the other agents, consistent with its personal-instance declaration, so that the two never disagree.
125. As the owner, I want the Kotlin and Java language servers in the container setup, so that container agents get the same code intelligence.
126. As the owner, I want the in-cluster runner's image to keep the tools it needs (including `oras`), so that the runner still works.
127. As the owner, I want the SSO-gated install finished for what remains (the revoke demonstration, the per-host OpenRouter key, and a recorded decision for the Overleaf CLI login), so that every host is set up without a shared token.
128. As the owner, I want the failing github MCP server fixed or reported with its cause, so that direct sessions get GitHub tools.
129. As the owner, I want a workstation kubeconfig helper that gives agents read-only cluster access and keeps admin behind sudo, so that agents can inspect the cluster without being able to change it.
130. As the owner, I want JavaScript dependencies installed with pnpm and Python dependencies with uv, each with a 7-day cooldown, so that a freshly published package never reaches this machine.
131. As the owner, I want the Renovate plugin-pin PRs reviewed and merged or closed, and the failing dependency PRs investigated, so that the dependency queue is not stale.
132. As the owner, I want delivered and superseded issues closed with evidence (`install.sh` retired, installer artifacts served from GitHub releases, the critique hooks installed, the session brief covered by Hindsight), so that the backlog shows only what is left.

### Backlog as code and tracking

133. As the owner, I want this spec and its ticket graph kept as files in agent-kit, so that the backlog is reviewed in a pull request.
134. As the owner, I want a Rust sync tool that creates and updates the issues, sub-issue links and "blocked by" edges from the ticket graph, so that GitHub matches the files.
135. As the owner, I want the sync tool to print a dry-run plan and change GitHub only after I approve, so that no issue is created or closed by surprise.
136. As the owner, I want a second sync run to change nothing, so that I can sync whenever I like.
137. As the owner, I want every superseded issue closed with a comment that names its new ticket, and the mapping kept in the ticket graph, so that old links still lead somewhere.
138. As the owner, I want every ticket to name its lanes and hosts, so that I can see what each lane and machine gets.
139. As the owner, I want one build tracker for this whole spec, reading the same ticket graph, so that I see every ticket in one place.
140. As the owner, I want the old tracker pages to point to the new one, so that I do not read a stale board.
141. As the owner, I want each ticket to land as one reviewable PR from the main branch, so that review stays small.
142. As the owner, I want the landed work on the old integration branch reused ticket by ticket and that branch closed once it is empty, so that finished work is not rebuilt.
143. As the owner, I want the settled facts from the old handoff notes moved into this spec and the docs, and their research reports into the private papers repository, so that no untracked notes directory is needed.
144. As the owner, I want every live check to be its own ticket for each host, so that a result proven on one machine is never assumed on the other.

### Documentation

145. As a maintainer, I want the glossary to define the instances, the direct lane, agent ids, reach levels and the Nedap bank with one meaning each, so that the model stays learnable.
146. As a maintainer, I want the setup guide to describe both instances on both OSes, including the second macOS user on the work machine, so that a new machine can be set up from the guide.
147. As a future contributor, I want every guarantee the critique runner enforces and a bare Gold Band node does not listed one by one, so that nobody assumes Gold Band enforces them.
148. As a future contributor, I want it written down which parts are proven by tests, which by live checks on which host, and which still depend on the app, so that I know what to trust.

### Code navigation

149. As the owner, I want a transcript analyser that reports which tools each agent uses to find code (search, reads, the language-server tool, the knowledge pages), and how many tokens each task spends exploring, so that retrieval work starts from what agents actually do.
150. As the owner, I want a frozen set of about twenty questions with known answers about my personal repositories, run on several model families, so that every retrieval change is measured the same way.
151. As the owner, I want every repository to carry an `AGENTS.md`, a pointer to its `CLAUDE.md` or a short curated file, so that every agent, not only Claude Code, reads the repository's own guidance.
152. As the owner, I want a lint that fails when an `AGENTS.md` names a path or symbol that no longer exists, so that the guidance does not rot.
153. As the owner, I want it found out why agents leave the installed language-server tool unused, so that the next tool is not ignored in the same way.
154. As the owner, I want one small read-only code-navigation MCP server that searches the live working tree, ranks and caps its results, and keeps no index, so that agents explore with fewer tokens and never see stale results.
155. As the owner, I want that server to give an outline of a file and expand one symbol on request, so that agents stop reading whole files to find one function.
156. As the owner, I want it to give a ranked map of a repository's files and symbols, and the repositories related to it from a manifest generated from the workspace and the build files, so that cross-repository questions get a short answer.
157. As the owner, I want the server to search only an allowlist of canonical roots that fit the instance's repository rule, to refuse symlinks out of them, and to refuse secret files, so that it never crosses the employer boundary or leaks a key.
158. As the owner, I want the server listed for every reach level, including `read`, because it writes nothing and opens no socket, so that every agent id can use it.
159. As the owner, I want an index (BM25 over identifiers first, dense vectors only after that, on the workstation GPU) added only when the measurements show that questions asked as concepts miss more than 15 percent of the time, so that maintenance cost is paid only where it earns its keep.
160. As the owner, I want the "no vector index for code" decision recorded with the conditions for revisiting it, so that it is not reopened without new evidence.

### Filed during review

161. As the owner, I want a model-gateway delegation that names a model family never to be answered by another family, and any escalation to be stated in the summary with both families and the cause, so that a second opinion is really from the family I asked for.
162. As the owner, I want the attribution guard to catch credit lines split across a line break, to be a required check in each repository's pipeline, and to reach every repository through the shared workflow release, so that new work never credits an AI.
163. As the owner, I want the nix-config `agent-kit-setup` wrapper to run from the projects-layout clone and refuse a non-main branch, so that setup never runs stale code from a forgotten clone.

## Implementation Decisions

### Carried decisions

These earlier owner decisions hold unless this spec says otherwise:

- **Code.** New code is Rust in one cargo workspace, under the 7-day crate
  cooldown (standards ADR-0014): `crate-gate` resolves every cargo command,
  and `crate-age` fails closed in the pre-commit hook and the pull-request
  workflow. npm and PyPI packages are not under the crate rule; they get
  their own cooldown tickets.
- **Gold Band.** Gold Band's code is never changed. Each gap is closed with a
  shim, and gaps are recorded for possible upstream issues. Gold Band is
  AGPL-3.0-only, and agent-kit only writes configuration and starts
  processes.
- **Hooks** stay outside Gold Band, in the Claude Code and Codex homes. The git
  commit gate and the PR gate hold whatever app makes the commit.
- **Opus** is used only in judgment nodes: nodes whose id ends in
  `-escalation`, `-acceptance` or `-planning`. Reach levels apply, with one agent id
  per agent, reach level and skill set.
- **Models on the personal instance.** OpenRouter and OpenCode models,
  including `opencode/` Zen models, are allowed, guarded by the LiteLLM
  per-key budgets. The model gateway keeps its own refusals.
- **The critique runner** stays the executor of the critique stages. Gold Band
  only sequences the stages and pauses on manual checks.
- **The model gateway**, `delegate` and the council are direct lane only. Gold
  Band workflows bypass the gateway, as a recorded exception to ADR-0007,
  except that the work instance reads the gateway's grant record for Codex.

### Changed decisions

- The Codex ban now applies to the personal instance only (ADR-0011).
- "Gold Band is never used on a work repository" is replaced by the
  repository rule of the two instances.
- #374's 2026-11-09 decision 1 (port the six old repo skills) is replaced:
  they are retired, and the four estate skills replace them.
- #374's decision 8 (build the ADR stages only as Gold Band templates) is
  replaced: they are built in the critique runner and then wrapped.
- #374's decision 3 (rewrite the Node runners in Rust) is deferred until
  after the stages land.
- #374's decision 5 (Hindsight read-only for agents) is replaced by the
  judgment-node write rule and the bank split.
- The family allowlist's rule that a `nedap` repository sends nothing to
  critics is replaced by a work-only critic roster (owner decision,
  2026-11-09). On a `nedap` repository, a critique stage may send to Codex on
  the nedap account, under a grant, and to the work Claude agent, and to
  nothing else, whatever the repository's entry says. This holds in the work
  instance and in the direct lane. A critic still never shares the
  orchestrator's family. So with a Claude orchestrator the critic is Codex,
  and with a Codex orchestrator the critic is the work Claude agent. The
  family allowlist entry in the glossary is amended to match.

### Modules

- **Instance model in the registry.** The registry gains an instance
  dimension. The Gold Band surface becomes two surfaces, `gold-band` and
  `gold-band-personal`. Each has:
  - its allowed agents;
  - its repository rule: work allows only `nedap`-owned remotes; personal
    refuses `nedap`-owned remotes and allows no remote;
  - its hosts;
  - its memory bank.

  Validation refuses any declaration that crosses an instance boundary. The
  work instance's employer-specific entries come from a per-machine overlay
  beside the keep file. The overlay may add MCP servers and settings to the
  work instance only, never agents beyond the two allowed. Its schema is
  validated like the registry.
- **Instance isolation.** Each instance has its own data directory, settings,
  agent map, role files and skill directories. The Linux workstation holds
  only the personal instance. On the work machine, the personal instance
  always runs under a second macOS user account (ADR-0011). That account has
  its own data directory, keychain, keep file and overlay, and setup runs
  once in each account. The account boundary is also the read boundary that
  keeps personal agents out of employer clones, which the OS write limit
  does not provide. So no machine needs two instances in one account.
- **Gold Band build.** It is a deep module with one interface: "make the
  pinned Gold Band present on this host". It:
  - checks out the pinned commit;
  - runs `crate-age` over Gold Band's lockfile;
  - builds the desktop app and the headless binary through `crate-gate` for
    the host's OS and architecture;
  - installs them per instance;
  - reports the version and build commit.

  The pin is `987fc3e` (= `v0.19.1`) until a newer release is at least 7 days
  old and has been audited. Upstream ships Linux packages, but they are not
  used, because they are not the audited, cooled-down build.
- **Setup converger for Gold Band.** This is the existing crate from the
  integration branch. It is generalised to:
  - two instances;
  - running-app detection by process and data directory on each OS, never by
    a hard-coded application path;
  - an atomic replace with one backup;
  - a write-group guard, so that setup stops when an instance starts in the
    middle of a run.
- **Launcher.** It is one Rust launcher with two interfaces:
  - the reach-level tool configuration for each agent (unchanged from the
    support record);
  - a write-limit interface with a Landlock backend and a `sandbox-exec`
    backend (ADR-0013). It builds each agent's profile from ADR-0013's table
    of writable paths for each reach level.

  When the backend for the OS is unavailable, or when the agent cannot hold
  the reach level, the launcher exits with an error before it starts
  anything. The support record gains an OS dimension: each cell is verified
  separately on Linux and on macOS.
- **Secret store.** It is one interface, "read secret by name", with a
  Keychain backend and a libsecret backend. It is used by the launchers, by
  the MCP stdio proxy and by the existing key script. The Linux plain-file
  fallback is retired for agent keys, and a missing backend fails closed.
- **Run-start shim.** Every Gold Band workflow starts with a cheap, read-only
  node that runs one pinned wrapper. The wrapper checks, from the working
  copy and from no model output:
  - the instance's repository rule against the repository's remotes;
  - the family allowlist for every bound agent id;
  - in the work instance, a live gateway grant for this run, for every bound
    Codex agent id.

  It writes its report like a gate report. A failed check ends the run before
  any model node runs. In the work instance, every node that binds Codex is
  preceded by a grant-check node that runs the same pinned wrapper for the
  grant alone. Before the node runs, it reserves one Codex node start
  against the grant's cap in the gateway's grant record. It refuses when the
  reservation does not fit, as ADR-0008 does for parallel delegations, so
  parallel Codex nodes cannot both pass on the last start. It fails, routing
  to a manual check, once the grant has lapsed. Each run names
  its gateway workflow `<gold band workflow>@<repository>@<run id>`.
- **Memory binding.** Each agent id declares its bank: Nedap for the work
  instance, Shared for the personal instance. A memory-write flag is allowed
  only on agent ids bound to `-acceptance` or `-escalation` nodes. The workflow lint gains the rule
  "memory write only on `-acceptance` or `-escalation` nodes". The rule
  applies only inside Gold Band; direct-lane sessions keep reading and
  writing their bank as today. Direct-lane
  sessions choose their bank by the repository's remote owner, using the same
  rule as the instances: `nedap` goes to the Nedap bank, and everything else
  goes to Shared Memory.
- **Nedap bank migration.** It is a one-time, owner-run procedure, done by the
  work Claude agent:
  - select the employer facts in Shared Memory;
  - copy them into the Nedap bank;
  - check recall against a recorded question set;
  - delete them from Shared Memory only after the check passes, with a
    report of what was moved and what was deleted.

  The owner then runs the same pass over Private Memory through
  `vibe-private`, after typing the passphrase, because no agent can reach
  Private Memory on its own.
- **Gate shim, Workflows A, B and C, the export wrapper, the workflow lint,
  and the inventory check.** These are as specified in #350 and #367, and
  their landed crates are reused. They change in three ways:
  - each Gold Band workflow has a work variant and a personal variant, which
    differ only in their bindings, their output locations and the bank;
  - Workflow C's publish step reads the per-repository tracker setup;
  - Workflow B's gates include the CI-wait command.
- **Per-repository tracker setup.** It is a small file in each repository
  that records the tracker (GitHub issues), the label vocabulary and the
  triage labels. A setup command writes it, as the existing skills setup
  does. Workflow C and the export wrapper refuse a repository without one.
- **Critique runner.** The review stages (ADR review, decision recovery,
  architecture review), the refactoring-plan critique stage inside the
  feature flow, and calibration are finished in the existing runner. On a
  `nedap` repository, the runner's routing uses the work-only critic roster.
  It runs the one critic whose family is not the orchestrator's. The
  draft stage work on the old branches is rebased and reused. The Wave 2
  templates call the runner through the gate shim. The Rust port of the
  runner is a later ticket, behind the same commands.
- **Estate skills.** The four estate skills and the drift check are as
  decided in #286. They are installed as workstation skills for the direct
  lane and as a skill set for the personal instance. The six old repo skills
  and their helpers are removed.
- **Backlog as code.** The spec and the ticket graph live together in the
  repository. Each ticket entry has:
  - a stable key, a title and a body;
  - its labels;
  - its parent branch;
  - its "blocked by" keys;
  - its lanes and hosts;
  - the old issues it supersedes.

  A Rust sync tool turns the graph into GitHub issues, sub-issue links and
  dependency links. It records each created issue number against its key,
  and on owner approval it closes the superseded issues with a mapping
  comment. It is idempotent, and it has a dry-run mode that prints the full
  plan. The build tracker reads the same graph and the GitHub state.

### Branches of the ticket graph

Each branch is a sub-issue of this spec, and its tickets are its sub-issues.
Dependencies may cross branches.

- **B0 Foundations.** It covers:
  - the cargo workspace and the cooldown on Linux;
  - the sync tool;
  - the Gold Band build on both OSes;
  - the first ticket, which re-verifies every cited Gold Band fact at the
    pin on both OSes.
- **B1 Instances and setup.** It covers:
  - the instance model and overlay;
  - convergence and safe writes;
  - isolation;
  - the launcher and both write-limit backends;
  - the secret store;
  - identities and launchers;
  - reach levels for each agent on each OS;
  - skill sets, roles and MCP servers in each instance;
  - the macOS-only paths in setup.
- **B2 Gold Band workflows.** It covers:
  - the run-start shim and gate shim;
  - Workflows A, B and C in both variants;
  - the lint rules;
  - headless proofs and the refusal of unproven runs;
  - LiteLLM in the cluster and its budgets;
  - the per-repository tracker setup;
  - the CI-wait gate.
- **B3 Port.** It covers:
  - the inventory with lanes;
  - the Wave 2 critique templates;
  - the estate skills and the retirement of the six old skills;
  - the cutting of the later waves.
- **B4 Direct lane.** It covers:
  - the critique stages and live checks;
  - the `/flow` run on a work repository;
  - the gateway's Codex worktree support and the Vibe model fixes;
  - the in-cluster runner;
  - the research front door;
  - the Rust port of the Node runners.
- **B5 Memory.** It covers:
  - the Nedap bank;
  - bank selection by remote owner;
  - the write rule;
  - the migration;
  - the cross-agent recall check;
  - Private Memory in Vibe.
- **B6 Workstation and hygiene.** It covers:
  - Playwright;
  - OpenCode parity;
  - the container language servers;
  - the SSO install remainder;
  - the github MCP server;
  - the kubeconfig helper;
  - the pnpm and uv cooldowns;
  - the dependency PR queue;
  - closing delivered issues with evidence;
  - the attribution-guard follow-ups and the setup wrapper (stories 162–163).
- **B7 Code navigation.** It covers:
  - the transcript analyser and the question set (measure first);
  - `AGENTS.md` everywhere, with its lint;
  - the investigation of why the language-server tool goes unused;
  - the code-navigation server: boundary checks and search, outline and
    expand, the repository map, symbols and related repositories;
  - its registry entry and instance binding, and its boundary proof under
    each OS's write limit;
  - the measurement runs per layer, the ADR against a vector index for code,
    and the two conditional tickets (an identifier index, and a
    language-server bridge for non-Claude agents);
  - the critique runner reading the server's repository map after the
    runner's Rust port.

### Work outside the tickets

These happen outside the ticket graph and are not blocked by it:

- Done: the Vibe model-list fix (#391), the Vibe sign-in check on Linux
  (#394, closing #393) and the allowlist attribution guard (#366) are merged.
- Open: the estate-wide attribution check (github-workflows#222) and the
  extended attribution rule text (#400).
- Deleting the stray duplicate clone outside the projects layout.

The ADR on the old integration branch that records "Gold Band workflows run
outside the model gateway" is renumbered when it lands, because `main`
already uses its number.

## Testing Decisions

- **What a good test checks.** A good test checks behaviour a person would
  see: the files after setup and their content, what a second run changes,
  what is refused and with which message, run states, exit codes, gate
  reports and exported files. It does not check how a module builds them.
- **Seam 1 (main): setup against a temporary home.** The existing setup
  harness runs a real setup against a temporary home, with fake agent
  binaries and a host argument for Linux or macOS. For the instances, it
  asserts:
  - both instances converge to exactly their declared and kept entries;
  - every boundary-crossing declaration is refused, including one from the
    overlay;
  - a second run changes nothing;
  - an instance that is not installed is skipped;
  - the running-app guard refuses;
  - the data directories are owner-only;
  - on a macOS host, the personal instance is prepared only for the second
    user account, never beside the work instance;
  - launchers are installed;
  - a missing secret fails closed;
  - no macOS path is written on a Linux host.

  Prior art: the setup-harness tests for OpenCode, Antigravity, MCP and
  plugin convergence, and the Gold Band convergence tests on the old
  integration branch.
- **Seam 2: a headless `gold-band run` against stub ACP agents.** Each Gold
  Band workflow, in both variants, runs headless with stub agents. No model
  and no network are needed. The test asserts the run state and the exit
  code (0 success, 1 failure, 2 paused manual check), and routing:
  - the repair path;
  - escalation after the second repair;
  - refusal by the run-start shim for a wrong-instance repository, a
    forbidden family, and a missing Codex grant;
  - a grant that lapses mid-run: the next Codex node's grant check fails and
    the run pauses at a manual check;
  - the gate-report tamper cases;
  - budget exhaustion;
  - a memory write from a non-judgment node refused by the lint before the
    run.

  The seam is skipped, with a reason, when no headless binary is supplied.
  It runs on Linux and macOS CI runners. Prior art: the spec-stage template
  proof and the stub-agent fixtures on the old integration branch.
- **Seam 3: the launcher's write limit on a real OS.** On a Linux CI runner
  (Landlock) and a macOS CI runner (`sandbox-exec`), the launcher starts a
  stub agent. The stub tries to write inside and outside its output
  directory. The test asserts that the outside write is refused by the
  operating system, with the tool configuration out of the way, and that the
  launcher refuses to start when the backend is unavailable. For each reach
  level, it also asserts the ADR-0013 table, including that a `write` agent
  cannot change `.git/hooks` or `.git/config`.
- **Existing seams, kept.** These stay as they are:
  - critique runner tests for the direct-lane stages;
  - model-gateway tests for the Codex worktree support and the Vibe model
    list;
  - registry render and validation tests;
  - the inventory check test.
- **The sync tool.** Its dry-run plan is its test seam. Given a fixture graph
  and a fixture GitHub state, the test asserts:
  - the plan: create, update, link and close;
  - that a second plan is empty;
  - that a superseded issue's closing comment names its new ticket.

  GitHub is faked at the tool's client interface.
- **Live checks.** Live checks are tickets, never CI, and each is run on each
  host where its claim applies:
  - the same-name MCP overlap test;
  - the reach-level cells for each OS;
  - the second macOS user on the work machine: a personal agent cannot read
    an employer clone;
  - real-run smokes;
  - the Nedap migration recall check;
  - cross-agent recall;
  - Playwright on a cold cache;
  - Private Memory in Vibe;
  - the `/flow` run on a work repository.
- **Code navigation.** The server's own Rust crate carries its tests: search
  ranking and caps, outline and expand, the repository map, and refusals
  (a root outside the allowlist, a symlink out, a secret file). Its boundary
  is also proven in seam 3: an agent id at `read` reach can use the server,
  and the server writes nothing. The question set is a measurement, not a
  CI gate: each layer is run on it and the token and accuracy results are
  recorded per layer.
- **Coverage.** The 100 percent line-coverage standard applies to all new
  Rust code. `crate-age` runs over the workspace lockfile in the pre-commit
  hook and the pull-request workflow.

## Out of Scope

- Any change to Gold Band's own code, including the gaps recorded for
  upstream (a command node, static fan-out, budgets). Upstream contributions
  stay a separate ticket, under the cooldown rule, and the relicensing
  decision is not part of this spec.
- Reimplementing the critique runner's guarantees inside Gold Band.
- Removing anything from the direct lane.
- Any work-instance agent other than the work Claude agent and Codex, and any
  personal-instance use of Codex or the work Claude agent.
- Publishing anything employer-specific in the public registry.
- The Renovate Dependency Dashboard (#33).
- Other repositories' backlogs (the cross-repository parents in fleet-infra
  and workspace are linked, not rewritten).
- Upgrading the Gold Band pin past `987fc3e` before a newer release is at
  least 7 days old and audited.
- A container- or VM-based isolation layer.

## Further Notes

- **Facts to re-verify first.** Every Gold Band fact the old specs cite at
  `987fc3e` is unverified until the first B0 ticket re-checks it on each OS.
  That includes:
  - how the agent map merges;
  - whether the Claude adapter forwards the environment;
  - whether hooks fire;
  - skill-directory handling;
  - per-node working directories;
  - the behaviour of the dynamic node;
  - the headless exit codes.
- **macOS assumptions to port first.** Found on the old integration branch:
  - the hard-coded application path in the converger and its running-app
    check;
  - the Homebrew path for Antigravity;
  - the keychain calls in the key script and the Vibe-study launcher;
  - launchd in the port-forward agent;
  - `~/Library` paths in the registry;
  - a headless binary built only for Apple silicon.
- **Notes missing from the integration branch.** The old handoff notes refer
  to a sandbox profile, a build script and a standards-integration report
  that are not on that branch. They are treated as lost and redone where
  needed.
- **Open for the owner, settled by tickets:**
  - real LiteLLM keys and their behaviour at the cap;
  - the two open decisions on the kubeconfig helper;
  - whether the exported ticket body counts as history under the family
    allowlist;
  - the later wave order, cut from the inventory.
- **Old trackers.** The build trackers for #327, #350 and #367, and the
  agent-kit build board, get a pointer to the new tracker once it exists.
- **Issues filed after this spec.** They belong to its ticket graph and are
  rewritten into it like every other open ticket:
  - #393: the critique runner did not see Vibe's sign-in on Linux. Fixed by
    #394 and closed. It is recorded under B1 (the secret store) as done.
  - #395: the model gateway silently moves a delegation that names a family
    to Claude when it fails. Belongs to B4 (direct lane).
  - #396: the operating-system layer limits writes, not reads, so on the
    work machine a personal-instance agent could read employer clones. It is
    decided (ADR-0011, ADR-0013): the personal instance on the work machine
    always runs under a second macOS user. A read limit in the launcher is
    optional defence in depth. Belongs to B1.
  - #399: Antigravity on the Linux workstation, so that Linux gets a
    Google-family critic and the personal instance's Antigravity agent.
    Belongs to B1 (agents on each OS).
- **Attribution guard.** Done on 2026-11-09, outside the graph:
  - #366 merged. It is an allowlist of the owner's identities, and setup
    turns attribution off in every agent that has a setting for it.
  - github-workflows#222 adds the same rule as an estate-wide CI check, and
    is open for review.

  Their follow-ups go under B6:
  - catch credit lines split across a line break;
  - make the check required in each repository's pipeline;
  - bump consumers to the release that carries it;
  - point the nix-config `agent-kit-setup` wrapper at the projects-layout
    clone, refusing a non-main branch, and retire the stray clone that kept
    this machine on an old setup.
- **Code navigation.** An investigation on 2026-11-09 found this order:
  1. measure what agents actually use, from transcripts and a fixed question
     set;
  2. give every repository an `AGENTS.md`;
  3. build a small read-only code-navigation MCP server over the live
     working tree, with no index, kept inside each instance's repository
     rule;
  4. add Zoekt, SCIP and vector embeddings only if the measurements show
     misses.

  This is branch B7, stories 149–160.

## Coverage

Every open issue and pull request in this repository on 2026-11-09, by name.
"Done on the integration branch" means the work landed on #383's branch, not
on `main`: its ticket is rewritten, and its code is reused there. The ticket
sync closes each superseded issue with a comment naming its new ticket.

### Issues

| Issue | Title | Branch | Disposition | Stories |
| :- | :- | :- | :- | :- |
| #23 | codex worker adapter: worktree-aware sandbox, fresh tokens, commit fallback | B4 | Rewrite: gateway Codex worktree support | 115 |
| #24 | cross-provider review harness: one command from branch to verdict | B3 | Close: absorbed by Workflow B and the review stages | 88–90 |
| #26 | runner: fetch the latest kit at session bootstrap instead of relying on bake-time pins | B4 | Rewrite: runner fetches the newest kit | 118 |
| #31 | Add a robust 'wait for CI fully green' tool for agents | B2 | Rewrite: CI-wait gate, also for the direct lane | 92 |
| #33 | Dependency Dashboard | — | Outside: Renovate dashboard | Out of Scope |
| #35 | Publish the installer artifacts to the static host on release | B6 | Close: done (installer from GitHub releases, install.sh retired) | 132 |
| #40 | Make setup-workstation.sh a true superset and retire install.sh | B6 | Close: done (installer from GitHub releases, install.sh retired) | 132 |
| #41 | SSO-gated install: bootstrap any host without a shared token | B6 | Rewrite: SSO install remainder | 127 |
| #42 | Agent A learns a fact, agent B recalls it in a fresh session | B5 | Rewrite: cross-agent recall per bank (live check) | 71 |
| #43 | Inject a bounded session brief from accepted knowledge and memory | B5 | Close: covered by Hindsight's session recall | — |
| #48 | Repair or report the failing github MCP server | B6 | Rewrite: github MCP server | 128 |
| #61 | Add the Kotlin and Java language servers and oras to the container setup script | B6 | Rewrite: container language servers and oras | 125–126 |
| #111 | Epic: Playwright MCP starts in every agent from one pinned binary | B6 | Close: umbrella superseded by this spec | 122–123 |
| #113 | Epic: private Hindsight bank verified in Vibe and its issues closed | B6 | Close: umbrella superseded by this spec | 122–123 |
| #116 | [LIVE: workstation] Vibe lists the Playwright tools on a cold npx cache | B6 | Rewrite: Playwright on a cold cache (live check) | 122 |
| #126 | [LIVE: workstation] Private Hindsight bank works in Vibe | B5 | Rewrite: Private Memory in Vibe (live check) | 123 |
| #127 | [LIVE: GitHub] Close the private-bank and Vibe issues with evidence | B5 | Rewrite: Private Memory in Vibe (live check) | 123 |
| #132 | Hindsight memory tools in Vibe | B1 | Rewrite: Hindsight MCP in Vibe, per instance bank | 59–64 |
| #167 | Cross-provider critique pipeline: foundation and /critique-spec | B4 | Close: umbrella superseded by this spec | 110–114 |
| #178 | Critique: live smoke check and seeded sample run | B4 | Rewrite: remaining live checks | 114 |
| #181 | Cross-provider critique pipeline: the remaining critique stages | B4 | Close: umbrella superseded by this spec | 110–114 |
| #182 | Cross-provider critique pipeline: /flow, hooks and /critique-stats | B4 | Close: umbrella superseded by this spec | 110–114 |
| #190 | Critique: live checks for the decision, ticket and code stages | B4 | Rewrite: remaining live checks | 114 |
| #198 | Critique: install the three hooks through setup | B4 | Rewrite: remaining live checks | 114 |
| #200 | Critique: one real feature through the whole pipeline | B4 | Rewrite: one feature through /flow on a work repository | 113 |
| #214 | Critique pipeline: ADR review, decision recovery, architecture review and refactoring plans | B4 | Close: umbrella superseded by this spec | 110–114 |
| #226 | Critique: check-adrs pre-check | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #227 | Critique: adr-reviewer role with adr-record and decision-log lenses | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #228 | Critique: editorial ADR rewrites and check-rewrite | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #229 | Critique: cold-read check for rewritten ADRs | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #230 | Critique: adr-record critic at the decisions stage | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #231 | Critique: phase 1 live check on agent-kit | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #232 | Critique: history facts | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #233 | Critique: code facts | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #234 | Critique: drift and stale checks in /critique-adrs | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #235 | Critique: decision-recovery stage in the runner | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #236 | Critique: /recover-decisions skill | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #237 | Critique: phase 2 live check on agent-kit | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #238 | Critique: dependency, domain and aggregate lenses and the design-evidence fragment | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #239 | Critique: architecture-review stage in the runner | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #240 | Critique: /review-architecture skill | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #241 | Critique: phase 3 live check on agent-kit | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #242 | Critique: refactoring plan format and check-plan | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #243 | Critique: plan stage in the feature flow | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #244 | Critique: plan-critic role, plan lenses and the plan evidence rule | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #245 | Critique: phase 4 live check on agent-kit | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #246 | Critique: calibration in /critique-stats | B4 | Rewrite: calibration in the stats | 112 |
| #247 | Critique: phase 5 live check on agent-kit | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #249 | Critique: /critique-adrs core | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #250 | Critique: coupling, test-seam and quality-scenario lenses | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #251 | Critique: /critique-plan skill | B4 | Rewrite: ADR review stages in the runner | 110–112 |
| #279 | build-tracker: one page for several specs | B0 | Rewrite: one tracker for this spec | 139–140 |
| #284 | vibe headless runs get no working web_search or web_fetch, and stall until the time limit | B4 | Rewrite: fail fast, document workaround | 117 |
| #285 | Research delegation: fan a research question out to several model families with web access, and verify the citations | B4 | Rewrite: research front door | 87 |
| #286 | Estate workstation skills: bootstrap, Project Intent, delivery status and rollback, kept current by a drift check | B3 | Close: superseded by this spec | 107–109 |
| #287 | Skill sources drift check: sources.yaml, check_skill_sources.py and a weekly workflow | B3 | Rewrite (PR #386): drift check | 109 |
| #288 | estate-project-intent skill: author and validate deploy-kit Project Intent | B3 | Rewrite: estate skills in both lanes | 108 |
| #289 | estate-delivery-status skill: did my release land | B3 | Rewrite: estate skills in both lanes | 108 |
| #290 | estate-rollback skill: plan a pause, rollback or resume without dispatching | B3 | Rewrite: estate skills in both lanes | 108 |
| #291 | estate-bootstrap skill: start or audit a repo against templates, workflows and testing standards | B3 | Rewrite: estate skills in both lanes | 108 |
| #292 | Retire the six personal-stack repo skills | B3 | Rewrite: retire the six old skills | 107 |
| #326 | Gold Band surface: setup converges Gold Band agents, roles and MCP servers | — | Close: superseded by this spec | — |
| #327 | Gold Band surface: Rust launcher, capability profiles and 7-day crate cooldown (amends #326) | — | Close: superseded by this spec | — |
| #328 | Gold Band real-app facts: merge semantics, Claude env, hooks, skills, node settings | B0 | Rewrite: re-verify every Gold Band fact on each OS | Further Notes: facts to re-verify |
| #329 | Same-name MCP overlap test | B1 | Rewrite: overlap test per instance | 59 |
| #330 | Settle the unverified reach cells | B1 | Rewrite: reach cells per agent and OS | 51 |
| #331 | Rust workspace with the 7-day crate cooldown | B0 | Done on the integration branch; reuse | Carried decisions (Code) |
| #332 | Registry declares the Gold Band surface and validation refuses bad input | B1 | Done on the integration branch; extend to two instances | 13–16, 52, 55 |
| #333 | Setup converges Gold Band agents (OpenCode first) | B1 | Done on the integration branch, Mac paths; rewrite for both OSes | 21–23, 32–33 |
| #334 | Safe writes: running-app guard, backup, not-installed skip, hardening | B1 | Done on the integration branch, Mac paths; rewrite | 28–31 |
| #335 | Claude work and personal identities | B1 | Done on the integration branch; split per instance | 36–37 |
| #336 | Vibe and Vibe-study launcher | B1 | Done on the integration branch, macOS keychain; rewrite on the secret store | 39–40 |
| #337 | Pi pinned install and declaration | B1 | Done on the integration branch; install on Linux too | 41 |
| #338 | Declare Antigravity and Hermes | B1 | Done on the integration branch, Homebrew path; rewrite | 42 |
| #339 | Reach-level launcher and support record | B1 | Done on the integration branch; add the OS dimension | 44–46 |
| #340 | Pi reach levels | B1 | Rewrite: reach levels per agent and OS | 44–52 |
| #341 | Vibe reach levels | B1 | Rewrite: reach levels per agent and OS | 44–52 |
| #342 | Claude reach levels | B1 | Rewrite: reach levels per agent and OS | 44–52 |
| #343 | Operating-system write confinement | B1 | Rewrite: Landlock and sandbox-exec behind one interface | 47–50 |
| #344 | Skill sets per agent id | B1 | Rewrite: skill sets per agent id | 58 |
| #345 | Roles generated from the critique role files | B1 | Done on the integration branch; reuse | 57 |
| #346 | MCP fleet into Gold Band | B1 | Rewrite: MCP servers per instance, overlay for work | 59–61 |
| #347 | Headless Gold Band binary with the cooldown check | B0 | Done on the integration branch, Mac only; rewrite for both OSes | 24–25 |
| #348 | Spec-stage workflow template and headless proof | B2 | Done on the integration branch, Mac binary; re-prove on both OSes | 79 |
| #349 | Docs: glossary, setup guide, lost guarantees | B1 | Rewrite: docs for both instances | 145–148 |
| #350 | Delegation workflows on Gold Band: research, implementation, and grill-to-tickets | — | Close: superseded by this spec | — |
| #351 | Port inventory of skills, hooks, roles and MCP servers | B3 | Done on the integration branch; add the lane column | 100–101 |
| #352 | Family allowlist check at setup and the ADR amending ADR-0007 | B2 | Done on the integration branch (crate); fold into the run-start shim | 7 |
| #353 | Gate wrapper, gate report and read-only copier | B2 | Rewrite: gate shim | 74–76 |
| #354 | LiteLLM per-key budgets and the Pi route | B2 | Rewrite: LiteLLM in the estate cluster | 96–98 |
| #355 | Ticket-to-requirement export wrapper | B2 | Done on the integration branch; reuse | 95 |
| #356 | Research repository scaffold and layout validator | B2 | Done on the integration branch; reuse | 85 |
| #357 | Coverage gate on changed files | B2 | Rewrite | 74, 91 |
| #358 | Finding check node for implementation critics | B2 | Rewrite | 74, 91 |
| #359 | Workflow B skeleton: unrolled repair loop with stub proof | B2 | Rewrite: Workflow B, work and personal variants | 88–92 |
| #360 | Workflow B real agents, families and skill sets | B2 | Rewrite: Workflow B, work and personal variants | 88–92 |
| #361 | Workflow A: deep research with Pi fetchers | B2 | Rewrite: Workflow A, work and personal variants | 82–87 |
| #362 | Workflow C: grill to tickets with human gates | B2 | Rewrite: Workflow C with per-repository tracker setup | 93–95 |
| #363 | Wave 2: critique stages as Gold Band templates | B3 | Rewrite: Wave 2 critique templates over the runner | 102, 111 |
| #364 | Docs: glossary, setup guide and recorded gaps | B2 | Rewrite: docs | 145–148 |
| #365 | Real-run smoke: one research run and one ticket | B2 | Rewrite: real-run smoke per instance (live check) | 79, 144 |
| #367 | Port skills, roles, hooks and MCP servers to Gold Band | — | Close: superseded by this spec | — |
| #368 | Epic: adopt Gold Band for agent setup, delegation workflows and the skills port | — | Close: superseded by this spec | — |
| #369 | Inventory check enforces one target per row | B3 | Done on the integration branch; reuse | 100 |
| #370 | Inventory records every overlapping older issue | B3 | Done on the integration branch; reuse | 100 |
| #371 | Workflow lint for the model and loop rules | B2 | Done on the integration branch; add the memory-write rule | 69, 77 |
| #372 | Registry validation of port bindings | B3 | Rewrite: port-binding validation | 100 |
| #373 | Real run refused when its headless proof was skipped | B2 | Rewrite | 79 |
| #374 | Wave plan after Wave 2 recorded in the inventory | B3 | Done on the integration branch; decisions 1, 3, 5 and 8 replaced by this spec | 106 |
| #377 | Upstream Gold Band contributions: validate CLI first, then a command-node proposal | B0 | Rewrite: upstream contributions, under the cooldown | Out of Scope (upstream) |
| #379 | Setup installs the work Claude profile into ~/.claude whatever CLAUDE_CONFIG_DIR says | B1 | Rewrite (do early): work Claude profile in the default directory | 38 |
| #381 | Install JavaScript dependencies with pnpm and a 7-day cooldown | B6 | Rewrite: pnpm and uv cooldowns | 130 |
| #382 | Install Python dependencies with a 7-day cooldown | B6 | Rewrite: pnpm and uv cooldowns | 130 |
| #389 | Workstation kubeconfig helper: read-only agent access, admin only behind sudo | B6 | Rewrite (PR #388): kubeconfig helper; two owner decisions | 129 |
| #390 | OpenCode on the workstation: skills and MCP coverage after #313 | B6 | Rewrite (PR #387): OpenCode parity | 124 |
| #392 | Spec: agent-kit on Gold Band — two instances, one backlog | — | This spec | — |
| #395 | Model gateway silently escalates a cross-family delegation to Claude | B4 | Rewrite: binding family in the gateway | 161 |
| #396 | Personal Gold Band instance can read employer clones on the work machine | B1 | Decided: second macOS user; live check | 11, 50 |
| #399 | Set up Antigravity on the Linux workstation | B1 | Rewrite: Antigravity on Linux | 42, 120 |

### Pull requests

| PR | Title | Branch | Disposition |
| :- | :- | :- | :- |
| #400 | docs(attribution): never describe AI use in pull requests, commits or issues | B6 | Review and merge on its own |
| #388 | feat: workstation kubeconfig helper | B6 (#389) | Hold for the two owner decisions, then fix CI |
| #387 | feat: install and configure OpenCode on the workstation | B6 (#390) | Rebase, review, align with the personal instance's OpenCode declaration |
| #386 | feat: skill-sources drift check (#287) | B3 (#287) | Rebase off main without the gateway commit, then finish |
| #385 | feat: editorial ADR rewrites and check-rewrite (#228) | B4 (#228) | Rebase onto the runner stages and finish |
| #384 | feat: critique pipeline review stages (#214) | B4 (#249, #230) | Rebase and finish in the runner |
| #383 | feat: Gold Band adoption (epic #368) | B0–B3 | Source of finished code: each ticket lands as its own PR from main; close when empty |
| #378 | chore(deps): lock file maintenance | B6 | Close or let Renovate retry after the pnpm change (#381) |
| #268 | chore(main): release 6.0.0 | — | Re-run on the current main and merge to cut the release |
| #156, #153, #152, #148 | Renovate plugin pin updates | B6 | Review each pin, then merge or close |
| #73 | chore(deps): update minor and patch dependencies | B6 | Investigate the real CI failures |
| #70 | fix(deps): pin dependencies | B6 | Investigate, or close in favour of the cooldown tickets |
