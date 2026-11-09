# Spec: agent-kit on Gold Band — two instances, one backlog

This spec replaces every open agent-kit epic and spec: #368, #326, #327, #350,
#367, #111, #113, #167, #181, #182, #214 and #286. Each open ticket in this
repository, except the Renovate Dependency Dashboard (#33), is rewritten as a
ticket under this spec. The old issues are closed as "superseded by" their new
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
  holds a live grant for that repository. It runs the same Gold Band
  workflows as the personal instance, with two model families (Claude and
  Codex), so author and reviewer always differ. Its research results stay
  inside the work repository. Its tickets go to the work repository's GitHub
  issues, through a per-repository tracker setup. It reads and writes only the
  Nedap bank.
- **Personal instance.** It holds every declared agent except Codex and the
  work Claude agent: the personal Claude agent, OpenCode, Vibe, Vibe-study,
  Pi, Antigravity and Hermes. It refuses `nedap`-owned repositories and allows
  a repository with no remote. It runs on the Linux workstation and on the
  work machine. It reads and writes Shared Memory. Its cheap agents reach
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
model is spent. Only judgment nodes may use Opus, and only judgment nodes may
write memory.

The backlog is rebuilt as code. The spec and a ticket graph live in this
repository. A Rust sync tool creates the issues from them, links the
sub-issues and "blocked by" edges, and closes the superseded issues with their
mappings. It shows a dry-run plan first and changes GitHub only after the
owner approves. One new build tracker shows every ticket of this spec.

## User Stories

### The two instances and their boundaries

1. As the owner, I want a work instance named `gold-band` and a personal instance named `gold-band-personal`, so that the names follow the launcher scheme (unsuffixed is work, `-personal` is personal).
2. As the owner, I want the work instance to hold only the work Claude agent and Codex, so that employer code reaches only employer-paid models.
3. As the owner, I want the personal instance to hold every declared agent except Codex and the work Claude agent, so that personal work never runs on an employer account.
4. As the owner, I want the work instance to refuse any repository whose remote is not owned by `nedap`, and any repository with no remote, so that it touches only employer repositories.
5. As the owner, I want the personal instance to refuse any repository with a `nedap`-owned remote, so that employer code never reaches a personal provider.
6. As the owner, I want the personal instance to allow a repository with no remote, so that I can bootstrap a new personal repository; I accept that I must be mindful there.
7. As the owner, I want the repository rule checked both when setup binds a Gold Band workflow and again when each run starts, so that a remote changed after setup still refuses the run.
8. As the owner, I want the work instance installed only on the work machine, so that the Linux workstation never holds employer configuration.
9. As the owner, I want the personal instance installed on the Linux workstation and on the work machine, so that I can use it on both.
10. As the owner, I want the two instances on one Mac to keep separate data, settings, agents and logs, so that a run in one can never see the other's state.
11. As the owner, I want the personal instance to run under a second macOS user account if Gold Band cannot keep two data directories apart, so that the separation holds whatever the app supports.
12. As the owner, I want no keep-file entry able to keep Codex or the work Claude agent on the personal instance, so that the boundary cannot be broken by one file.
13. As the owner, I want the registry to declare only the generic shape of the work instance (`claude-work`, `codex`), so that nothing about the employer enters the public registry.
14. As the owner, I want employer-specific work-instance settings, such as work MCP servers, in a per-machine overlay beside the keep file, so that they are repeatable on my machine but never published.
15. As the owner, I want setup to refuse a registry that names Codex, the work Claude agent or a `nedap` binding for the personal instance, so that the rule is enforced where it is declared.
16. As the owner, I want setup to refuse a registry that names any agent other than the work Claude agent and Codex for the work instance, so that no cheap or third-party provider sees employer code.

### Codex in the work instance

17. As the owner, I want every work-instance run that binds Codex to be refused unless the model gateway holds a live grant for that repository, so that Codex spend stays under the grant rule (ADR-0008).
18. As the owner, I want the run-start check to read the gateway's own grant record, not a copy, so that granting, expiry and caps keep one source.
19. As the owner, I want a refused run to say which grant is missing and how to ask for it, so that I can fix it in one step.
20. As the owner, I want grants still asked for and recorded only through the gateway, so that agents keep asking before they write a grant.

### Setup and convergence

21. As the owner, I want one setup run to converge both instances present on a machine, so that a fresh machine matches the registry.
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
47. As the owner, I want file writes outside the declared directory blocked by the operating system, not only by the tool settings, so that a gap in a tool list does not matter.
48. As the owner, I want the write limit to use Landlock on Linux and `sandbox-exec` on macOS behind one interface, so that the same declaration means the same thing on both machines.
49. As the owner, I want the launcher to refuse to start an agent when the write-limit backend for the current OS is unavailable, so that no agent runs unlimited.
50. As the owner, I want it written down that neither backend limits the network, so that nobody treats a read-only node as offline.
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
66. As the owner, I want recall from the Nedap bank checked before the moved facts are deleted from Shared Memory, so that nothing is lost.
67. As the owner, I want the moved facts deleted from Shared Memory after that check, so that no personal bank keeps employer content.
68. As the owner, I want only judgment nodes (an id ending in `-acceptance` or `-escalation`) to write memory, and every other node to read only, so that only checked results are remembered.
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
80. As the owner, I want the work instance to run the same Gold Band workflows as the personal instance, bound only to the work Claude agent and Codex, so that work repositories get the same sequencing, gates and critique.
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

110. As the owner, I want the ADR review stages (`/critique-adrs`, `/recover-decisions`, `/review-architecture`, `/critique-plan`) finished in the critique runner, so that work repositories get them from the command line.
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
146. As a maintainer, I want the setup guide to describe both instances on both OSes, including the second-user fallback, so that a new machine can be set up from the guide.
147. As a future contributor, I want every guarantee the critique runner enforces and a bare Gold Band node does not listed one by one, so that nobody assumes Gold Band enforces them.
148. As a future contributor, I want it written down which parts are proven by tests, which by live checks on which host, and which still depend on the app, so that I know what to trust.

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
- **Opus** is used only in judgment nodes. Reach levels apply, with one agent id
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
  agent map, role files and skill directories. Whether Gold Band can be
  pointed at a second data directory is the first fact to verify on each OS.
  If it cannot, the personal instance on the Mac runs under a second macOS
  user account. Setup prepares that account's instance in the same way and
  reports it.
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
    backend (ADR-0013).

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
  - in the work instance, a live gateway grant for every bound Codex agent id.

  It writes its report like a gate report. A failed check ends the run before
  any model node runs.
- **Memory binding.** Each agent id declares its bank: Nedap for the work
  instance, Shared for the personal instance. A memory-write flag is allowed
  only on agent ids bound to judgment nodes. The workflow lint gains the rule
  "memory write only on `-acceptance` or `-escalation` nodes". Direct-lane
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
- **Critique runner.** The ADR stages, decision recovery, architecture review,
  refactoring plans and calibration are finished in the existing runner. The
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
    pin on both OSes, including whether a second data directory is possible.
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
  - the cross-agent recall check.
- **B6 Workstation and hygiene.** It covers:
  - Playwright;
  - Private Memory in Vibe;
  - OpenCode parity;
  - the container language servers;
  - the SSO install remainder;
  - the github MCP server;
  - the kubeconfig helper;
  - the pnpm and uv cooldowns;
  - the dependency PR queue;
  - closing delivered issues with evidence.

### Work outside the tickets

These happen outside the ticket graph and are not blocked by it:

- Merging the green fix for the Vibe model list (#391).
- Merging the agent-attribution guard (#366).
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
  launcher refuses to start when the backend is unavailable.
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
  - the second data directory;
  - real-run smokes;
  - the Nedap migration recall check;
  - cross-agent recall;
  - Playwright on a cold cache;
  - Private Memory in Vibe;
  - the `/flow` run on a work repository.
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
  - the headless exit codes;
  - a second data directory.
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
  - #396: the operating-system layer limits writes, not reads. So on the
    work machine, a personal-instance agent can read employer clones. It
    belongs to B1. Its mitigation is an owner decision that is still open:
    the recommendation is the second macOS user (ADR-0011), with a read
    limit in the launcher as defence in depth. Stories 10, 11, 47 and 48 and
    ADR-0013 are amended when it is decided.
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

  Its candidate tickets would form an eighth branch, B7 "Code navigation".
  Whether B7 is part of this spec is an owner decision that is still open.
