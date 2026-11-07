# Draft amendments to spec #326: Rust and the crate cooldown

Status: DRAFT, not posted to the issue. Owner decisions of 2026-11-06 (after the handoff was written):

- New code is written in Rust. Existing TypeScript (council, gateway) and Python (setup, converge, render) stay for now and are migrated by their own tickets, outside #326.
- A 7-day cooldown on every Rust crate is a hard rule. Source: `JorisJonkers-dev/standards`, ADR-0014 (status: proposed), `scripts/README.md`, `scripts/crates/crate-gate`, `scripts/crates/crate-age`.

## Facts checked on this machine

- `crate-gate` is running: `curl http://127.0.0.1:17342/health` answers `crate-gate: up`, and `~/.cargo/config.toml` replaces crates.io with the `gated` source. Every `cargo` command here resolves through it.
- The rule has two layers: prevention (`crate-gate`, hides and refuses versions younger than 7 days) and an after-the-fact check (`crate-age`: pre-commit hook on any `Cargo.toml` or `Cargo.lock` change, plus a pull-request workflow; exit 1 for a too-young version, exit 2 when age cannot be proven).
- ADR-0014: one cargo workspace per repository, so one `Cargo.lock` and one gate. Git and path dependencies are refused. A security fix goes through an expiring, human-approved waiver, not an exception in the check.
- The rule is still "proposed" in standards, not accepted. agent-kit has no Rust, no `Cargo.toml` and no cooldown config today (checked: no match for `cooldown` or `minimum-release-age` outside a lens catalogue).
- The scope is crates. The owner did not extend it to npm or PyPI packages (Pi, `claude-agent-acp`, LiteLLM); those stay under pinned versions and `--ignore-scripts` as in the handoff.

## Changes to the spec text

**Implementation Decisions, new bullet "Language".**
Code new in this work is Rust, in a single cargo workspace in agent-kit. The Gold Band renderer and its registry validation (agent map, MCP list, role files, Codex / OpenRouter / literal-secret refusals, running-app guard, atomic replace with backup) are a Rust crate that the existing setup runs. The Python setup and registry code only calls it and passes the registry path. Migrating Python and TypeScript to Rust is out of scope.

**Implementation Decisions, new bullet "Crate cooldown".**
No crate version younger than 7 days is used in anything this work builds. The workspace follows standards ADR-0014: one workspace and lockfile; no git or path dependencies; `crate-gate` resolves every cargo command; `crate-age` runs in the pre-commit hook and in the pull-request workflow, and fails closed. Waivers follow the standards waiver rule, expire, and need the owner.

**Implementation Decisions, "Workflow template and headless proof".** Add: the headless `gold-band` binary is built from the audited commit with cargo through `crate-gate`. Before the build, `crate-age` runs over Gold Band's `Cargo.lock` and the result is recorded. A crate in that lockfile younger than 7 days at its publish date blocks the build and goes to the owner.

**Testing Decisions.**
- Seam 1 stays "setup against a temporary home" through `tests/setup_harness.py`, but the logic under test is the Rust crate. The crate also has its own unit tests. The 100 percent line-coverage standard applies to the new crate (state the tool, for example `cargo llvm-cov`, once chosen).
- New check: `crate-age` passes over the agent-kit workspace lockfile (CI and pre-commit).
- Add that Seam 1 must still work on a machine without the Rust toolchain only if a prebuilt crate binary is supplied; otherwise the Rust part is skipped with an explicit reason (open question below).

**User stories to add (numbers continue after 42).**
43. As the owner, I want every crate used by agent-kit to be at least 7 days old, so that a freshly published malicious version never reaches this machine.
44. As the owner, I want a commit that adds or updates a crate younger than 7 days to be refused before it lands and again on the pull request, so that a mistake on my own machine is still caught.
45. As the owner, I want the Gold Band Rust dependencies checked against the 7-day rule before I build or trust a headless binary, so that the audited build is also a cooled-down build.
46. As a maintainer, I want the Gold Band setup logic in one Rust crate, so that new code follows the direction of the repository.

**Out of Scope, add.** Porting existing TypeScript or Python to Rust. A cooldown for npm and PyPI packages. Accepting ADR-0014 in the standards repository (it stays proposed there; agent-kit follows it anyway).

**Further Notes, add.** The upstream Gold Band contribution (standalone `validate` CLI) is Rust and adds no crate unless it passes the same 7-day rule. Gold Band's own CI does not enforce this, so every upstream PR that touches `Cargo.toml` needs a `crate-age` run first.

## Open questions for the owner

1. Where does the crate live: `runner/`, `scripts/` or a new top-level `crates/`? Standards put all Rust in `scripts/`. agent-kit's root was regrouped into `runner/`, `scripts/`, `provenance/` and `docs/` (#283), so `scripts/` fits the layout. Recommendation: `scripts/` workspace, same as standards.
2. How do setup and CI get `crate-gate` and `crate-age`: build from standards, or copy them in? Copying code between the two repos needs the same licence check as any other copy; both are owner repositories.
3. Does Seam 1 build the Rust crate in the test, or use a prebuilt binary?
4. Does this need a new ADR in agent-kit (language direction plus the cooldown)? Recommendation: one short ADR, after the standards ADR-0014 is accepted.

## Handoff corrections

- HANDOFF section 9 point 9 says "write fresh code" for upstream. Add: in Rust, with the cooldown.
- HANDOFF section 3: the local Gold Band build predates this rule. `crate-age` compares publish dates with today, so on an old lockfile it passes trivially. The useful question for the audited build is whether any locked crate was younger than 7 days when the lockfile was committed; that needs publish dates against the commit date, which `crate-age` does not do today. Not done; the build tree is gone and needs `git archive 987fc3e`.

---

# Addition: per-node capabilities and skills (owner, 2026-11-06, later)

Owner requirement: workflow nodes must be restricted by what they need. Research needs web access and read access, and may write only to one named location (to hand back results). Some nodes use certain skills; others do not get them.

The owner pasted a chat answer describing a launcher `agent-acp --agent X --capabilities read|read-web|write|full`. It is **unverified third-party text** and lists Codex, which is banned. Checked locally (installed agents on this machine): Hermes has `platform_toolsets` and `HERMES_HOME`; the OpenCode binary contains `OPENCODE_CONFIG_CONTENT`; Vibe's installed package has **no** `enabled_tools` / `disabled_tools` string (not verified either way, may be named differently). Claude and the rest are not checked.

## Design points

1. **Capability profiles** are declared in the registry: `read` (read and search files only; no shell, no writes, no web, no delegation), `read-web` (read plus web search and fetch), `write-scoped` (read, plus writes only under one declared directory), `write` (read and edit in the repo; shell set separately), `full` (explicit, personal throwaway repos only, today only Pi). Each agent declares which profiles it supports.
2. **One Gold Band agent id per (agent, profile)**, for example `opencode-read`, `vibe-read-web`, `hermes-write-scoped`. Gold Band binds agents to workflow nodes, so a node's access is the access of the agent it names. Each id launches through the Rust launcher with the profile as an argument. The launcher is a managed binary installed by setup, like the Vibe-study launcher.
3. **Unsupported means refuse.** If an agent cannot express a profile, the launcher exits with an error and the registry validation rejects the declaration. It never starts with broader access.
4. **Two layers.** (a) Tool configuration per agent (OpenCode inline config, Hermes toolsets in a profile-specific home, Vibe tool lists, Claude `tools` / `disallowedTools`, not `allowedTools`, which only pre-approves). (b) Operating-system enforcement for file writes: the launcher runs the agent under `sandbox-exec` with writes denied except the declared output directory and the agent's own state directories. Prior art: `handoff-gold-band/build/sandbox.sb`. Reason: Hermes `file` toolset includes writes, Pi has no permission prompts, and tool lists do not stop shell commands.
5. **Web cannot be blocked at the operating-system layer.** Every agent needs network access to reach its model. "No web" is enforced only by removing the web tools and by not passing MCP servers that fetch. State this limit in the glossary; do not claim more.
6. **Inherited reach.** Each profile also fixes: MCP servers passed (none for `read` unless listed), extensions and plugins, sub-agent delegation, and hooks. Restricting edit tools alone does not make a node read-only.
7. **Per-node skills.** Gold Band writes skills into the agent's skill directory (`primaryAgentDir`), so skills are per agent id, not per node. A node gets a skill set by naming an agent id whose skill directory holds only that set. Setup builds one skill directory per skill set from the registry. Whether Gold Band can limit skills per node itself is unverified and is a first piece of work. Skills-migration design (out of scope above) must keep this property: no single shared skills directory for all ids.
8. **Not a replacement for the runner's guarantees.** These profiles reduce what a Gold Band node can reach. They do not give the read-only reach tripwire, the quote check or the other items in story 38.

## Stories to add

47. As the owner, I want each declared agent id to carry a capability profile, so that a node's reach is visible in the registry.
48. As the owner, I want a research node to read files, use web tools and write only into one named directory, so that results come back and nothing else changes.
49. As the owner, I want the launcher to refuse a profile an agent cannot enforce, so that a node never runs with more access than declared.
50. As the owner, I want file writes outside the declared directory blocked by the operating system, not only by tool settings, so that a tool-list gap does not matter.
51. As the owner, I want each node to see only the skills it needs, so that unrelated skills do not steer it.
52. As a maintainer, I want the registry to fail when an agent id has no profile, a profile an agent does not support, or a write-scoped profile without an output directory.

## Open questions

- Which of the five (Claude, OpenCode, Vibe, Hermes, Antigravity, Pi) can support which profile? Needs a per-agent check against the real binaries, then a table in the spec. Candidate cheap batch (read-only, one delegation per agent).
- Is `sandbox-exec` acceptable long term (deprecated by Apple, still works)? Alternative: none found for a user-level macOS process without a container.
- Does Gold Band pass node-level working directory or only the repo root? That decides whether the output directory can be set per node.
