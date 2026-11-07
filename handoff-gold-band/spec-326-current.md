## Problem Statement

The owner now runs Gold Band, a desktop client that launches coding agents over the Agent Client Protocol and sequences work with a JSON workflow. Today it is configured by hand in its own settings file. Nothing in agent-kit says which agents, roles and MCP servers Gold Band should hold, so the result cannot be reviewed, repeated on another machine, or kept in line with the rest of agent-kit. Two Claude identities, two Vibe launchers, Pi, Antigravity, Hermes and the MCP fleet are all wanted, and one mistake (Codex appearing in the agent list, a secret written in plain text, an MCP server declared in two layers) breaks a rule the owner has already set.

agent-kit's convergence model (declared entries, retired records, undeclared entries, the keep file) covers every other agent surface. Gold Band is the one surface that is not converged.

## Solution

A setup run treats Gold Band as one more surface. The registry declares the agents, the roles (Gold Band calls them profiles) and the MCP servers Gold Band should hold. A setup run makes the Gold Band settings and role files match that declaration, leaves everything the app owns alone, and ends with exactly the declared entries plus the kept ones. A second run changes nothing. Setup refuses to write while the app is running, because the app rewrites its own files.

A critique stage (the spec stage) is also expressed as a Gold Band workflow template with one worker node per critic, and a headless Gold Band run proves the template is valid and ends in the expected state, using a stub agent so no model is called. This is a deliberately degraded, time-boxed path that sits beside the real critique runner, not instead of it; the guarantees it loses are listed explicitly.

## User Stories

1. As the owner, I want the registry to declare which agents Gold Band holds, so that the list is reviewed in a pull request and not clicked together in a window.
2. As the owner, I want a setup run to make the Gold Band agent list match the registry, so that a fresh machine gets the same agents as this one.
3. As the owner, I want a second setup run to change nothing, so that I can run setup whenever I like.
4. As the owner, I want setup to refuse to write while Gold Band is running, and say what to do, so that the app does not overwrite the result.
5. As the owner, I want setup to leave the app's own preferences, workflows and automatic templates untouched, so that my choices in the window survive a setup run.
6. As the owner, I want an agent that is not declared to be removed from Gold Band unless the keep file names it, so that Gold Band follows the same rule as every other surface.
7. As the owner, I want a declared entry I later retire to be removed from Gold Band, so that retired things do not linger.
8. As the owner, I want Codex never to appear in the Gold Band agents, in any role binding, or in any routing list, so that the employer-metered account is only reached through the model gateway with a grant, and I want no keep-file entry able to keep it on this surface.
9. As the owner, I want setup to fail with a clear message if the registry names Codex or an OpenRouter-backed provider for Gold Band, so that the rule cannot be broken by editing one file.
10. As the owner, I want a work Claude agent that always uses the work configuration directory, so that an ambient environment variable can never switch identity.
11. As the owner, I want a personal Claude agent that always uses the personal configuration directory, so that personal work stays under the personal account.
12. As the owner, I want both Claude agents to share the same Claude binary, hooks and skills, so that behaviour is the same apart from the login.
13. As the owner, I want a Vibe agent and a Vibe-study agent, so that I can use both accounts from Gold Band.
14. As the owner, I want the Vibe-study agent to read its key from the agent-kit secret store at launch, so that no key is written into any Gold Band file.
15. As the owner, I want the Vibe-study launcher for Gold Band to be a managed script installed by setup, so that it is versioned and repaired like the other launchers.
16. As the owner, I want OpenCode, Antigravity and Hermes to be declared with the launch command each already provides, so that they work without custom launcher scripts.
17. As the owner, I want Pi installed from pinned versions and declared with its own binary path, so that Gold Band does not download code at runtime.
18. As the owner, I want Pi marked as throwaway-repos-only in the registry, so that its lack of permission prompts is a visible, reviewed decision.
19. As the owner, I want skill directories written only for agents whose skill directory is verified, so that Gold Band does not write into a directory it should not.
20. As the owner, I want the declared roles to be generated from agent-kit's own role files, so that there is one source of truth and no drift.
21. As the owner, I want the generated roles to carry our evidence rules and output contract, so that a critic run through Gold Band follows the same rules as today.
22. As the owner, I want role files to be written so that Gold Band does not try to expand them as templates, so that braces in our output contracts survive.
23. As the owner, I want a role I retire to disappear from Gold Band, so that the role list stays reviewed.
24. As the owner, I want my own hand-made roles to survive when they are on the keep list, so that experiments are not deleted.
25. As the owner, I want a table of which agent and model family fills each role by default, so that an author and its reviewer are never the same family.
26. As the owner, I want the same-name overlap test run and recorded before any MCP server is declared in Gold Band, so that I know whether tools appear twice.
27. As the owner, I want each MCP server declared in exactly one layer for a given agent once the test has decided the layering, so that no tool is duplicated.
28. As the owner, I want no secret value in any Gold Band file, so that plain-text storage in the settings file never holds a token.
29. As the owner, I want token-bearing servers declared through the existing keychain proxy pattern, so that the token is read at launch and never lands on disk.
30. As the owner, I want setup to fail if a declared MCP record contains a literal token, so that a mistake is caught before it is written.
31. As the owner, I want the data directory of Gold Band restricted to my account, so that prompts and code it logs are not readable by other users.
32. As the owner, I want setup to report whether Gold Band is installed, which version and which build it is, so that I can tell the audited build from an unknown one.
33. As the owner, I want setup never to enable the updater, metrics, feedback, WeCom or Multica, so that the audited, quiet configuration stays that way.
34. As the owner, I want a critique spec-stage workflow template shipped with agent-kit, so that the stage can be run from Gold Band.
35. As the owner, I want a headless run of that template against a stub agent to end in the expected state, so that I know the template is valid and routes as designed.
36. As the owner, I want that template to contain no Codex and to use the finding schema as its output contract, so that it keeps today's guarantees.
37. As the owner, I want a needs-human verdict in the template to pause for a manual check, so that a person decides.
38. As the owner, I want every guarantee the critique runner enforces and a Gold Band run does not (read-only reach tripwire, per-critic search server and packet, quote check, refute round, family allowlist and `nedap` deny, plan-limit routing) listed one by one as lost in the Gold Band path, so that nobody assumes Gold Band enforces them.
39. As a maintainer, I want the registry validation to fail on an unknown agent field, a duplicate id or an id that Gold Band rejects, so that errors show up in CI, not in the app.
40. As a maintainer, I want the Gold Band surface documented in the glossary and the setup guide with the same words as the other surfaces, so that the model stays learnable.
41. As a future contributor, I want it written down which parts are proven by tests and which still depend on the app, so that I know what to trust.

42. As the owner, I want setup to skip the Gold Band surface, create nothing and say so when Gold Band is not installed, so that a machine without it still sets up cleanly.

## Implementation Decisions

- **Surface and vocabulary.** Gold Band becomes a surface in the registry with the same words as the others: declared entry, retired record, undeclared entry, keep file, converge. Setup owns only the keys it declares in the Gold Band settings file (the agent map and the MCP server list) and the profile files it generates. It never writes the app's workflow store, automatic-template store, preferences or databases.
- **Agent declarations.** Each declared agent has an id (lowercase letters, digits, dashes), a launch command and arguments, a display name, an environment map, and optional skill directories. Built-in Gold Band agent ids carry only the user fields (the app injects their command from its own catalog). The declared set is: the work Claude agent, the personal Claude agent, OpenCode, Vibe, Vibe-study, Pi, Antigravity and Hermes.
- **Claude identities.** Both Claude agents run the same adapter and the same Claude binary and differ only by an explicit configuration-directory environment variable, set on both, so no ambient variable can decide. This guarantee holds only if the adapter forwards the agent's environment to Claude Code; verifying that is the first piece of work for these two agents, a test asserts the declared environment on both entries, and if the adapter does not forward it the identity stories are not met and that ticket is blocked. The personal configuration directory shares skills, hooks and settings with the work one by link, so skill directories are written for these two agents only.
- **Vibe.** Vibe uses the ACP binary Vibe ships. Vibe-study is plain Vibe in the shared Vibe home with the Study account key exported under the name Vibe reads. A managed launcher script, installed by setup beside the existing launchers, reads that key from the agent-kit secret store and then starts the ACP binary. Private Memory launchers are not declared.
- **Pi.** Pi and its ACP adapter are installed globally from pinned versions without install scripts, and the declared command is the installed binary, so the app never fetches code at launch. The registry records Pi as throwaway-repos-only because it has no permission prompts.
- **Hard rules in validation.** Registry validation rejects, for the Gold Band surface: any agent or binding naming Codex, any provider or model id naming OpenRouter, any OpenCode model outside the `opencode-go` provider, any literal secret value (a value stored under a token, secret, key, password or authorization field counts as literal unless it is an environment-variable reference or the name of a keychain-proxy field), and any duplicate or malformed id. These are new declaration rules for this surface. They mirror the model gateway where it has an equivalent: the gateway refuses OpenRouter and any `opencode/...` model, allows only the `opencode-go` provider for OpenCode by default, and requires a grant for Codex; this surface has no grant, so Codex is banned outright. Each rule is the owner's recorded decision, now enforced at the declaration.
- **Roles.** Gold Band calls roles profiles. The role set is twelve: six Gold Band built-ins reused (plan, dev, dev-test, test, accept, grill) and six generated (decision-gap critic, spec red team, ticket linter, diff reviewer, feature reviewer, adversarial verifier). Generated roles are rendered from the existing critique role and lens files, so those files stay the single source. Templating is switched off for generated roles. A role-to-agent table assigns families so that author and reviewer differ, uses only families the gateway's catalogue already offers, and excludes Codex. The table only names candidates: the family allowlist decides which family may receive a repository's content (a repository with no remote, or one owned by `nedap`, sends nothing), no critic shares the family of the orchestrator that adjudicates its findings, and a refuter is a family that neither raised the finding nor is the orchestrator's.
- **What a role cannot enforce.** Read-only reach, quote verification, schema enforcement, the refute protocol and plan-limit routing are what the critique runner enforces today. The role text states the rules; it does not enforce them. This is recorded next to the role table.
- **MCP.** The MCP fleet moves into Gold Band only after the same-name overlap test. The first piece of work is that test: one harmless server declared in both layers for Claude, Vibe and OpenCode, recording whether tools appear twice or the name overrides. Its result decides the layering rule (a server is declared in exactly one layer for a given agent, or the layers are allowed to overlap). Token-bearing servers are only ever declared through the existing keychain stdio proxy pattern, so no secret is written. The built-in Gold Band memory server is reviewed against the Hindsight setup and either kept or declared off.
- **Running-app guard.** Setup detects a running Gold Band process and refuses to write, saying to quit the app and re-run. The new settings are the existing document with only the declared keys replaced; every other key is preserved unchanged. Setup builds them, validates them, re-checks that the app is not running immediately before the replace, replaces the file atomically and keeps one backup; if the replace fails, the old file stays in place and setup reports the failure.
- **Convergence rules.** A declared entry is written; a retired record is removed; an undeclared agent, MCP server or generated role is removed unless the keep file names it. No keep-file entry can keep Codex on this surface, and Codex is never in anything setup declares (agents, role bindings, routing lists). Whether Gold Band merges the agent map with its built-in list or replaces it is not yet verified; the first piece of work establishes it against the real app. If the app turns out to keep a built-in Codex entry selectable after setup, setup says so in its report and the first piece of work decides whether the app can hide it; the ban applies to everything agent-kit declares, not to the app's own catalogue.
- **Not installed.** When Gold Band is not installed, setup skips the Gold Band surface, creates no settings file or role directory, and reports that Gold Band is not installed; the run still succeeds.
- **Hardening.** Setup restricts the Gold Band data directory to the owner. It never enables the updater, metrics, feedback, the chat connectors or the task-board integration, and it reports the installed version and build commit.
- **Workflow template and headless proof (degraded path).** One workflow template ships for the critique spec stage. By the owner's decision it replaces the runner on this path: worker nodes, one per critic, bound to non-Codex agents, launch the critics themselves, with the finding schema as the output contract, success on the stage summary, and a manual-check node for a needs-human verdict. It does not call the critique runner, so the guarantees listed in story 38 are not enforced on this path. This is a deliberate, time-boxed exception to the earlier rule that the experiment stops if a stage loses an ADR-0001 or ADR-0004 guarantee: the real critique runner stays the stage of record, the template is limited to personal repositories, the work is boxed to one day, and it ends early if the template cannot produce findings that validate against the finding schema. A fixture stub stands in for the agent, so the headless run needs no model and no network. The headless run uses the Gold Band command-line entry (`run`) with the workflow file. The test builds nothing itself: it runs only when a Gold Band command-line binary is supplied by the environment, and the build of that binary (from the audited source) is documented.
- **Licensing.** Nothing from Gold Band source is copied into agent-kit; agent-kit only writes configuration and starts processes. The relicensing decision is separate and not part of this work.

## Testing Decisions

- A good test checks behaviour a person would see: the files that exist after setup, their content, what a second run changes, and what is refused. It does not check how setup builds them.
- **Seam 1 (main): setup against a temporary home.** The existing setup harness runs a real setup with a temporary home and fake agent binaries and asserts the resulting files. For Gold Band it asserts: the declared agents, roles and servers appear; Codex, OpenRouter and literal secrets are refused with a clear message; a second run changes nothing; an undeclared entry is removed unless kept; a retired record is removed; the app's own files are untouched; a running-app guard refuses to write; the data directory is owner-only; the launcher script is installed and executable.
- **Seam 2: headless workflow run.** It proves structure and routing only; it cannot prove the guarantees the degraded path loses. A fixture workflow and a stub ACP agent run through the Gold Band command-line `run`; the test asserts the run state and exit code (0 for success, 2 for the paused manual check) and the findings artefact against the finding schema. It is skipped, with an explicit reason, when no Gold Band binary is provided, and it runs in a separate, optional job.
- **Prior art:** the setup-harness tests for OpenCode, Antigravity, the MCP convergence and the plugin convergence, and the registry rendering tests. The validation rules follow the existing registry-validation tests.
- The same-name overlap test is a recorded manual or scripted experiment, not a CI test, because it needs the real app; its result is written into the setup guide.
- The 100 percent line-coverage standard applies to any new renderer or validation code.

## Out of Scope

- Skills migration to a single Gold Band source directory (needs its own design; the facts are in the handoff).
- The LiteLLM proxy and per-key budgets.
- Installing, updating or building Gold Band itself.
- Upstream contributions to Gold Band, and the relicensing decision.
- Using Gold Band on any work (employer) repository; Gold Band is never used there.
- Replacing the critique runner, the council or the model gateway, and reimplementing the lost guarantees inside Gold Band.
- Fixing Gold Band's probe sessions that start in the filesystem root.
- Gold Band workflow templates other than the critique spec stage.

## Further Notes

- Facts that are still unverified and must be settled by the first pieces of work: how Gold Band merges its agent map with its built-in list; whether the Claude adapter forwards an agent's environment to Claude Code; whether our hooks fire for Gold Band-launched Claude; whether the headless binary builds from the audited commit; how Gold Band applies skill-directory fields for non-Claude agents.
- Decisions already taken (posture, Codex ban, no work repos, role list, Pi policy, MCP overlap test first) are recorded in the project memory and in `handoff-gold-band/HANDOFF.md` in the working copy. Drafts of the agent block, the Vibe-study launcher script and the role files are in `handoff-gold-band/config/`.
- Gold Band is AGPL-3.0-only; agent-kit's licence is proprietary source-available. This spec only writes configuration and starts processes, which keeps the two separate.
