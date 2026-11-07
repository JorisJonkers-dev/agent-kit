# Critique report: spec, run 20261106T101401Z-spec

Repository: github.com/jorisjonkers-dev/agent-kit
Depth: deep, chosen by signals: user_stories 41 > 30; risk term "migration"
Verdicts: accepted 13, rejected 0, needs-human 2, hint 4

## Critics

### c1-vibe-glm

- vibe-glm (zai-glm-5-3, zhipu): ok, 8 findings, $0.0769
- Role spec-redteam, lenses fidelity, testability, integration
- Exit code 0, 45262.359375 ms
- Tools: read_file, run_typescript, mcp_packet.grep, mcp_packet.list
- Prompt hashes: contract 976364a24eab795bb4d98dd8f22c988c47db699be1631d1cd35ed2bc2542b26b, preamble f7da57e294e7012cb20088f2d18f7dda7398396d55119537bd0e56ca5d81aa19, role 73c26e9862d4fc72fb066e4228bb174596733fe43a08fbd0cc62d42437e37a14, schema 3e5a5fc33f61562457c628c33a8706b6f9fa3d6e849502bc94b7a73464833bc6
- Lens hashes: fidelity caeb79a390b5cd773df298e285dbe759a8a29fa9b8a66d45afab82c6d5d713f8, testability f9628794c06dc8b21ef2a4fd16cbd662ab89931e608bfe174e95b8a9a217608a, integration 57c7efbe680878eac7b4ca617352f8d71b98044b1c97b94e3c3fbf5e1e1af138

### c2-vibe-deepseek-flash

- vibe-deepseek-flash (deepseek-v4-1-flash-1017, deepseek): ok, 11 findings, $0.0829
- Role spec-redteam, lenses failure-modes, security, simplicity
- Exit code 0, 108153.891125 ms
- Tools: read_file, run_typescript, mcp_packet.grep, mcp_packet.list
- Prompt hashes: contract 976364a24eab795bb4d98dd8f22c988c47db699be1631d1cd35ed2bc2542b26b, preamble f7da57e294e7012cb20088f2d18f7dda7398396d55119537bd0e56ca5d81aa19, role 73c26e9862d4fc72fb066e4228bb174596733fe43a08fbd0cc62d42437e37a14, schema 3e5a5fc33f61562457c628c33a8706b6f9fa3d6e849502bc94b7a73464833bc6
- Lens hashes: failure-modes 065246fe8dc81466f44cc0597358bd26c9c31b5301f6c72780123c2370ca5d62, security b5de59d883f30d436029c6b8c114c99f85d5cc93ccc873db30a66e36c215e7bb, simplicity 2cc16d1399cb56cf35ee8129152f0f41db56927e039aeb439af37ffef129c080

## Findings

### F1 [major contradiction] User Story 27 vs Implementation Decisions, MCP

Raised by: c1-vibe-glm (zhipu)
Verdict: hint (no-decision-or-scenario): no quote from a decision source and no concrete scenario

Story 27 commits to "exactly one layer" as the outcome, while the MCP decision says the overlap test decides between two possible rules, so the story pre-decides an unsettled question.

Evidence:

- quote (artifact/issue-326.md): I want each MCP server declared in exactly one layer for a given agent once the test has decided the layering, so that no tool is duplicated.
- quote (artifact/issue-326.md): Its result decides the layering rule (a server is declared in exactly one layer for a given agent, or the layers are allowed to overlap).

Suggestion: Reword story 27 to defer to the test's result, e.g. "I want the layering rule the overlap test decides to be applied to every MCP server, so that no tool is duplicated."

### F2 [major contradiction] User Story 8 vs Implementation Decisions, Convergence rules

Raised by: c1-vibe-glm (zhipu)
Verdict: accepted

Story 8 demands Codex never appear, but the convergence rule removes an undeclared entry only "unless the keep file names it", so a keep-file entry could keep Codex on the surface; the spec has no rule that the keep file cannot keep Codex, unlike the recorded Private Memory precedent.

Evidence:

- quote (artifact/issue-326.md): I want Codex never to appear in the Gold Band agents, in any role binding, or in any routing list, so that the employer-metered account is only reached through the model gateway with a grant.
- quote (artifact/issue-326.md): an undeclared agent, MCP server or generated role is removed unless the keep file names it
- quote (references/CONTEXT.md): No keep file can keep it on another surface.

Suggestion: Add to the convergence rules that no keep-file entry can keep Codex on the Gold Band surface.

### F3 [major failure-mode] Implementation Decisions, Convergence rules

Raised by: c1-vibe-glm (zhipu)
Verdict: accepted

The spec leaves the behaviour for built-ins the registry omits (in particular Codex) to an unverified merge fact, so if Gold Band merges rather than replaces its built-in agent list, story 8's absolute ban has no specified mechanism and no failure handling.

Evidence:

- quote (artifact/issue-326.md): Whether Gold Band merges the agent map with its built-in list or replaces it is not yet verified; the first piece of work establishes it against the real app, and the behaviour for built-ins the registry omits (in particular Codex) follows from that.
- quote (artifact/issue-326.md): I want Codex never to appear in the Gold Band agents, in any role binding, or in any routing list
- input: The overlap/merge experiment finds that Gold Band merges its built-in list with the agent map; Codex is a built-in the registry does not declare, so it survives every setup run and story 8 is violated with no specified remedy.

Suggestion: State the fallback explicitly: if the merge keeps undeclared built-ins, setup must still remove or disable Codex, or the spec must record the ban as blocked on that experiment.

### F4 [major invention] Implementation Decisions, Hard rules in validation

Raised by: c1-vibe-glm (zhipu)
Verdict: accepted

The spec claims its validation rules are "the same rules the model gateway already enforces", but no reference file records the gateway enforcing a Codex, OpenRouter or opencode-go ban; ADR-0007 in fact permits Codex for nedap repositories under a grant, and `opencode-go` appears nowhere in the references.

Evidence:

- quote (artifact/issue-326.md): These are the same rules the model gateway already enforces, now enforced at the declaration.
- quote (references/docs/adr/0007-one-model-gateway-for-every-delegation.md): a repository owned by `nedap` may delegate to Codex on the nedap account (under a grant) and to Claude Code, and to nothing else, whatever its family allowlist entry says

Suggestion: Drop the "same rules" claim and state these as new registry-validation rules for the Gold Band surface, with the source of the `opencode-go` constraint named.

### F5 [major untestable] User Story 39, registry validation

Raised by: c1-vibe-glm (zhipu)
Verdict: hint (no-decision-or-scenario): no quote from a decision source and no concrete scenario

"an id that Gold Band rejects" names no rule set: the spec does not say which ids Gold Band rejects, nor list this among the unverified facts, so a builder cannot write the validation or its test without guessing.

Evidence:

- quote (artifact/issue-326.md): I want the registry validation to fail on an unknown agent field, a duplicate id or an id that Gold Band rejects, so that errors show up in CI, not in the app.
- quote (artifact/issue-326.md): Each declared agent has an id (lowercase letters, digits, dashes)

Suggestion: Either pin the id grammar Gold Band accepts, or add Gold Band's id-rejection rules to the Further Notes list of facts the first pieces of work must settle.

### F6 [minor drift] Solution and Implementation Decisions ("roles (profiles)", "profile files"), Testing Decisions ("wrapper script")

Raised by: c1-vibe-glm (zhipu)
Verdict: accepted

The spec uses "profile" and "wrapper" as synonyms for launcher/role concepts the glossary explicitly marks _Avoid_, so tickets cut from it will drift from the established vocabulary.

Evidence:

- quote (artifact/issue-326.md): the roles (profiles) and the MCP servers Gold Band should hold
- quote (artifact/issue-326.md): the wrapper script is installed and executable
- quote (references/CONTEXT.md): **Launcher**:
A named command that starts an agent with a fixed account, memory access and
local state.
_Avoid_: alias, wrapper, profile

Suggestion: Use "role" and "launcher script" throughout, including in the Seam 1 assertion.

### F7 [minor contradiction] User Story 26 vs Implementation Decisions, MCP

Raised by: c1-vibe-glm (zhipu)
Verdict: hint (no-decision-or-scenario): no quote from a decision source and no concrete scenario

Story 26 requires the overlap test to run "before any MCP server is declared in Gold Band", but the test itself works by declaring one server in both layers, so a declaration exists before the test's result is recorded.

Evidence:

- quote (artifact/issue-326.md): I want the same-name overlap test run and recorded before any MCP server is declared in Gold Band, so that I know whether tools appear twice.
- quote (artifact/issue-326.md): The first piece of work is that test: one harmless server declared in both layers for Claude, Vibe and OpenCode

Suggestion: Scope story 26 to the MCP fleet: "before any MCP server of the fleet is declared in Gold Band (the test's own probe server excepted)".

### F8 [minor drift] User Story 38 and Implementation Decisions, What a role cannot enforce

Raised by: c1-vibe-glm (zhipu)
Verdict: accepted

The spec says "budget routing" and "budgets" stay in the critique runner, but ADR-0003's amendment replaced per-run dollar budgets with the plan limit checked through the model gateway, and the glossary marks "budget" as a term to avoid for Plan limit.

Evidence:

- quote (artifact/issue-326.md): Read-only reach, quote verification, schema enforcement, the refute protocol and budget routing stay in the critique runner.
- quote (references/docs/adr/0003-review-stages-run-outside-the-feature-flow.md): The per-flow dollar cap is gone. Vibe is a subsidised plan counted by plan limit, not a metered API billed by the token
- quote (references/CONTEXT.md): **Plan limit**:
How much a subsidised client's plan allows in one window (5 hours, day, week
or month): dollars or runs, never both.

Suggestion: Replace "budget routing" with "plan-limit routing" or "the plan-limit check through the model gateway".

### F9 [critical contradiction] User Story 8 and Implementation Decisions, Convergence rules

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

The spec promises Codex never appears in Gold Band, then makes that promise depend on an unverified fact about how the app merges its built-in agent list.

Evidence:

- quote (artifact/issue-326.md): Codex never to appear in the Gold Band agents, in any role binding, or in any routing list
- quote (artifact/issue-326.md): Whether Gold Band merges the agent map with its built-in list or replaces it is not yet verified; the first piece of work establishes it against the real app, and the behaviour for built-ins the registry omits (in particular Codex) follows from that.
- quote (artifact/issue-326.md): ends with exactly the declared entries plus the kept ones
- input: Gold Band merges its built-in list with the declared map. Setup writes the declared agents; Codex is a built-in the registry omits, so it stays in the app's list. Story 8's 'never' is false and the 'exactly the declared entries plus the kept ones' end state is unreachable.

Suggestion: State the merge behaviour as a decided rule, or make story 8 conditional on the first piece of work and say what setup does if the app merges built-ins.

### F10 [critical contradiction] Solution, second paragraph; User Story 38; Implementation Decisions, Workflow template and headless proof

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: needs-human, answered: Should the Gold Band spec-stage template drive the existing critique runner (one worker node per stage, so allowlist, read-only checks, quote checks, refute and plan-limit checks keep running in the runner), or replace the runner with per-critic worker nodes?

The spec runs the spec critique stage as a Gold Band workflow whose workers are agents, while saying the runner's guarantees stay in the runner; with no runner in the loop, nothing enforces them.

Evidence:

- quote (artifact/issue-326.md): A critique stage (the spec stage) is also expressed as a Gold Band workflow template
- quote (artifact/issue-326.md): It uses worker nodes bound to non-Codex agents
- quote (artifact/issue-326.md): the template's guarantees that the runner enforces (read-only reach, quote checks, the refute protocol, budgets) to be listed as staying in the runner, so that nobody assumes Gold Band enforces them
- quote (references/docs/adr/0001-critics-run-as-direct-cli-calls.md): We start the critic CLIs (`vibe -p`, `agy -p`) as subprocesses of the critique
- quote (references/docs/adr/0006-critics-search-their-packet.md): a small MCP server the runner starts for each Vibe critic
- trace: Gold Band `run` starts the worker agents itself. No critique runner means no packet built by `prepare`, no per-critic search server, no read-only tripwire, no quote check and no refute round. Story 38's guarantees have no runner to stay in.

Suggestion: Decide whether the Gold Band template calls the critique runner as its worker or replaces it, and if it replaces it, say which guarantees are reimplemented where.

### F11 [critical omission] Implementation Decisions, Roles (profiles); the spec never mentions the family allowlist

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

The spec sends the artifact to model families through a Gold Band workflow but drops the per-repository family allowlist that decides which families may receive it.

Evidence:

- quote (artifact/issue-326.md): A role-to-agent table assigns families so that author and reviewer differ, uses only families the gateway's catalogue already offers, and excludes Codex.
- quote (references/CONTEXT.md): It is keyed by the repository's remote.
- quote (references/CONTEXT.md): critics, whatever its entry says.
- quote (references/CONTEXT.md): documents only unless its code permission is on; the code stages refuse
- input: A personal repository whose recorded allowlist entry names one family. The Gold Band spec-stage workflow sends the artifact to whichever family the role table names, because nothing in the spec reads the allowlist. A repository with no remote is sent to as well.

Suggestion: Add the allowlist check to the workflow template's decisions: which entry applies, what happens when it is missing, and what the stage does when the family is not permitted.

### F12 [critical contradiction] User Stories 10 and 11; Implementation Decisions, Claude identities; Further Notes

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

The spec guarantees the Claude configuration directory cannot be switched by an ambient variable, while its own Further Notes list the mechanism that would carry that variable as unverified.

Evidence:

- quote (artifact/issue-326.md): so that an ambient environment variable can never switch identity
- quote (artifact/issue-326.md): differ only by an explicit configuration-directory environment variable, set on both, so no ambient variable can decide
- quote (artifact/issue-326.md): whether the Claude adapter forwards an agent's environment to Claude Code
- input: The adapter does not forward the agent's environment. Both Claude agents start with the ambient `CLAUDE_CONFIG_DIR` or none, so the personal agent runs on the work account. That is exactly the failure stories 10 and 11 forbid.

Suggestion: Make the identity guarantee conditional on the adapter forwarding the environment, and say what setup does if it does not.

### F13 [major contradiction] Implementation Decisions, Surface and vocabulary; Running-app guard

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

Setup is told never to write the app's preferences, but the write step builds new settings and replaces the file without saying the app's existing keys are carried over.

Evidence:

- quote (artifact/issue-326.md): It never writes the app's workflow store, automatic-template store, preferences or databases.
- quote (artifact/issue-326.md): It writes by building the new settings, validating them, and replacing the file, keeping one backup.
- input: The settings file holds the app's own preferences. Setup builds 'the new settings' from the declared agent map and MCP list and replaces the file; the preferences are gone, which story 5 forbids. Nothing says the new settings are merged with the existing keys.

Suggestion: Say that the new settings are the existing document with only the declared keys replaced, and that every other key is preserved byte for byte.

### F14 [major omission] User Story 25; Implementation Decisions, Roles (profiles)

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

The role table keeps author and reviewer in different families but drops the recorded rule that no critic may share the orchestrator's family, and the refute round's family rule is missing too.

Evidence:

- quote (artifact/issue-326.md): A role-to-agent table assigns families so that author and reviewer differ
- quote (references/CONTEXT.md): critic shares a family with the orchestrator that adjudicates its findings.
- quote (references/CONTEXT.md): one critic from a family that neither raised it nor is the
- input: The orchestrator is Claude. The table assigns the feature-reviewer role to a Claude agent because it differs from the author's family. The critic then shares the orchestrator's family, which the glossary forbids.

Suggestion: Add the orchestrator's family as an exclusion in the table and state the refute round's family rule.

### F15 [major untestable] User Story 30; Implementation Decisions, Hard rules in validation

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

Validation must reject 'any literal secret value', but the spec never defines what makes a value literal, so the rule and its test have no pass/fail line.

Evidence:

- quote (artifact/issue-326.md): any literal secret value
- quote (artifact/issue-326.md): I want setup to fail if a declared MCP record contains a literal token
- quote (artifact/issue-326.md): Token-bearing servers are only ever declared through the existing keychain stdio proxy pattern, so no secret is written.
- input: A record with `"token": "${GITHUB_TOKEN}"` and one with `"token": "ghp_..."`. The spec gives no rule that separates them, so the test 'literal secrets are refused' has no defined input.

Suggestion: Define the check: for example, any value that is not an environment reference or a keychain proxy field name is refused.

### F16 [major failure-mode] User Story 32; Solution; Implementation Decisions, Running-app guard

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: needs-human, answered: When Gold Band is not installed, should setup skip the Gold Band surface, create nothing and report it, or fail the setup run?

The spec says setup reports whether Gold Band is installed but never says what convergence does when it is not.

Evidence:

- quote (artifact/issue-326.md): I want setup to report whether Gold Band is installed, which version and which build it is
- quote (artifact/issue-326.md): A setup run makes the Gold Band settings and profiles match that declaration
- quote (artifact/issue-326.md): Setup detects a running Gold Band process and refuses to write
- input: A fresh machine with no Gold Band. Setup finds no process, so the guard passes. Nothing says whether setup creates the settings file and profile directory, skips the surface, or fails. Two builders will produce two different behaviours.

Suggestion: State the not-installed branch: skip the surface, create nothing, and report it.

### F17 [major failure-mode] Implementation Decisions, Running-app guard

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

The running-app guard is a check before the write with no lock, so the app can start between the check and the replace and two setup runs can overwrite each other's file and backup.

Evidence:

- quote (artifact/issue-326.md): Setup detects a running Gold Band process and refuses to write, saying to quit the app and re-run.
- quote (artifact/issue-326.md): It writes by building the new settings, validating them, and replacing the file, keeping one backup.
- quote (artifact/issue-326.md): because the app rewrites its own files
- input: Setup checks for the process, finds none, then the owner launches Gold Band while setup writes the profiles. The app rewrites its settings from memory; setup reports success. A second setup run started at the same time replaces the same file and overwrites the one backup.

Suggestion: Add a setup lock and re-check the process immediately before the replace, and say what happens when the replace fails after the profiles were written.

### F18 [major contradiction] Implementation Decisions, Hard rules in validation

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: accepted

The spec says these are rules the model gateway already enforces, but the recorded gateway rule for Codex is a grant, not a ban, and no reference records an OpenRouter or opencode-go rule.

Evidence:

- quote (artifact/issue-326.md): These are the same rules the model gateway already enforces, now enforced at the declaration.
- quote (references/docs/adr/0008-agents-write-grants-after-asking.md): The model gateway refuses a metered-employer client unless a grant
- trace: Searched all eight reference files for `OpenRouter`: the only hit is CONTEXT.md's Old stack entry, which says the old stack does not cover OpenRouter. No reference records a gateway rule banning OpenRouter-backed providers or restricting OpenCode to `opencode-go`.

Suggestion: Say these are new Gold Band declaration rules, and record the OpenRouter and opencode-go rules as decisions rather than as gateway behaviour.

### F19 [major omission] User Story 34; Implementation Decisions, Surface and vocabulary; Workflow template and headless proof

Raised by: c2-vibe-deepseek-flash (deepseek)
Verdict: hint (no-decision-quote): no quote from a decision source

The template is said to ship with agent-kit so the stage can be run from Gold Band, but setup never writes the app's workflow store and no decision says how the template reaches the app.

Evidence:

- quote (artifact/issue-326.md): a critique spec-stage workflow template shipped with agent-kit, so that the stage can be run from Gold Band
- quote (artifact/issue-326.md): It never writes the app's workflow store, automatic-template store, preferences or databases.
- quote (artifact/issue-326.md): One workflow template ships for the critique spec stage.
- input: The template is a file in agent-kit. Gold Band's UI reads its workflow store, which setup never writes. Nothing says whether the owner imports the file by hand, setup copies it somewhere the app reads, or the app is pointed at the repo path. The ticket has no decided behaviour.

Suggestion: Decide the installation path: either setup writes the template into the workflow store as a declared entry, or the spec says the owner imports it and where the file lives.

## Refute round

- F9: stands (invalid-answer) by r-F9-vibe-glm (vibe-glm, zai-glm-5-3, zhipu): The spec does not contradict itself: story 8 states the owner's requirement, and the convergence rule explicitly defers the merge behaviour to the first piece of work, naming Codex as the built-in whose treatment follows from that result. The only real gap is that the spec does not state what setup does if the app does merge built-ins, an acknowledged open question rather than a critical contradiction.
- F11: no-objection by r-F11-vibe-glm (vibe-glm, zai-glm-5-3, zhipu): Verified that artifact/issue-326.md never mentions the family allowlist, the repository remote, or any per-repository family policy (grep for 'allowlist|remote|family|families' returns only the role-table lines), while references/CONTEXT.md makes the allowlist a per-repository rule keyed by the remote with 'A repository with no remote sends nothing', and ADR-0007 states 'The family allowlist applies to every delegation, not only to critiques.' The workflow template decision binds worker nodes to agents with no allowlist check, and story 38's list of runner-enforced guarantees omits the allowlist, so the finding's claim and its input scenario hold as reported; no ground to refute or downgrade.
- F12: stands (invalid-answer) by r-F12-vibe-glm (vibe-glm, zai-glm-5-3, zhipu): The tension is real — the identity guarantee is stated unconditionally while its carrying mechanism is listed as unverified — but the artifact itself flags the adapter's environment forwarding as a fact that must be settled by the first pieces of work, so building from it costs a design adjustment caught by that gate, not a wasted build.

## Dissent

none
