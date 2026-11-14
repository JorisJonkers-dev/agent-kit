# The Rust workspace and the 7-day crate cooldown

The estate's Rust lives in **one cargo workspace**, `scripts/Cargo.toml`, with
**one lockfile**, `scripts/Cargo.lock`, and **no git or path dependency**
outside the workspace. The rule and the 7-day crate cooldown come from the
standards repo (`JorisJonkers-dev/standards`, ADR-0014, whose `scripts/README.md`
documents its tooling); this page is how agent-kit follows it, on the Mac and
on the Linux workstation.

* **The cooldown.** No crate version younger than **7 days** may be used.
  Two things hold it at resolve time, so a normal `cargo build` cannot pick a
  young version up even by accident: cargo's own cooldown
  (`registry.global-min-publish-age = "7 days"`, honoured from Rust 1.100), and
  the packages proxy every workstation and CI job resolves through
  (`packages.jorisjonkers.dev/cargo`), which hides those versions from its
  index. **crate-age** is the check for after the fact: it exits `1` for a version proven younger than 7
  days, `2` for a version whose age cannot be proven, and `0` only when every
  locked version is proven old enough. The gate **fails closed**: an unprovable
  age blocks the change.
* **The workspace.** Twenty-one members. `crates/agent-kit-setup` is the canary: a
  std-only crate (nothing in the lockfile has an age to question) whose
  `--version` proves the build and whose `validate <PATH>` checks a path.
  `crates/inventory-check` fails when an on-disk skill, hook, role file or MCP
  server is missing from docs/gold-band-port-inventory.md, has two targets or no
  lanes. `crates/gold-band-registry` is std-only too and validates the Gold Band
  instances and the overlay (docs/REGISTRY.md). `crates/code-nav-eval` runs the
  frozen code-navigation question set. `crates/code-nav` is the read-only
  code-navigation MCP server (below). `crates/workflow-lint` is the Gold Band
  workflow lint (below). `crates/gold-band-proof` is the Gold Band headless proof harness
  (docs/gold-band-proof.md; serde_json only). `crates/ticket-sync` is the ticket sync tool (below);
  it takes serde, serde_json and serde_yaml, each pinned exactly.
  `crates/agent-launcher` is the reach-level launcher (below); it takes serde
  and serde_yaml, each pinned exactly.
  `crates/model-gateway` is the first slice of the Rust model gateway (below);
  std-only.
  `crates/build-tracker` is the Rust port of the build tracker (below); it is
  std-only, with the ordered JSON its state file needs written in the crate.
  `crates/litellm-gate` is the node-side check that a LiteLLM per-key budget is
  not exhausted: one probe request through curl with the key on stdin, exit 1
  when the budget is spent or the proxy cannot be asked.
  `crates/ticket-publish` is the publisher Workflow C's publish gate runs (below); it takes serde_json
  and serde_yaml, each pinned exactly, and the workspace's own ticket-export.
  `crates/ticket-export` is the issue export wrapper (below); it takes serde_json
  and serde_yaml, each pinned exactly.
  `crates/transcript-mix` is the transcript tool-mix analyser (below), std-only
  with the JSON parser borrowed from `code-nav-eval`.
  `crates/gold-band-report` is the gate report format shared by the Gold Band
  shims: a report struct with its stable-key JSON, an atomic write into a
  report directory, a std-only SHA-256 and a std-only RFC 3339 time. It takes
  serde_json, pinned exactly. `crates/gold-band-run-start` is the run-start shim
  (below): it checks the repository rule, the family allowlist and the Codex
  grant before any model node runs, and writes its report through
  gold-band-report. `crates/gold-band-gate` is the gate shim (below): the
  wrapper that runs one gate command with a timeout and writes the gate report,
  and the read-only copier that verifies it. It takes no registry crate, only
  gold-band-report. `crates/coverage-gate` is the coverage gate (below), std
  only. `crates/gold-band-findings` is the finding check node (below): it sorts
  critic findings into kept and hints and takes only serde_json.
  `crates/ci-wait` is the CI-wait gate (below); it takes
  serde_json and serde_yaml, each pinned exactly. New Rust joins this
  workspace as a new member, never as its own Cargo project.

```bash
cargo build --manifest-path scripts/Cargo.toml
cargo test --manifest-path scripts/Cargo.toml
```

The workspace aims at **100 percent line coverage** of every crate: the unit
tests in `src/tests.rs` execute each of its lines, with one exception that each
crate names in a comment beside the code, the single executable line in
`src/main.rs` (the exit wrapper).

## code-nav

`code-nav` is a read-only MCP server over stdio that searches the live working
tree (no index). It takes no registry dependency: its JSON comes from
`code-nav-eval`.

```bash
code-nav --root ~/Projects/owner/repo [--root DIR ...] [--limit N]
```

* **Boundary.** The `--root` directories are the allowlist, canonicalised at
  start (overlapping roots collapse, a secret root is refused); the caller passes only roots that fit the instance's repository rule.
  A path outside every root, a symlink that leaves them and a secret file
  (`.env` and `.env.*` except `.env.example`, `.sample` and `.template`; SSH
  and other private keys; `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.kdbx`,
  `*.keystore`; `.netrc`, `.npmrc`, `.pypirc`, credential files; anything under
  `.ssh`, `.gnupg`, `.aws`, `.git` or `.kube`, and `*.env`, `*.tfvars`, `.envrc`, `kubeconfig`) are refused. Relative paths are taken against the
  first root. `--root` expands a leading `~` against HOME. A root that does not exist is skipped with a warning on stderr; the server exits 2 only when no root remains.
* **Read-reach proof.** `crates/code-nav/tests/read_reach.rs` (no libtest
  harness; it re-executes itself as the launcher) starts the server through
  `agent-launcher` at `read` reach under the real write limit (Landlock on
  Linux, `sandbox-exec` on macOS), calls all six tools, checks that a path
  outside the root, a symlink out, a secret file and a `nedap` tree are still
  refused, and checks that the tree is byte-for-byte and mtime unchanged
  afterwards. A control launch of a shell at the same level proves the limit
  was on: its writes beside, in and above the root fail with the OS's error.
  It runs on the Linux workspace job and the `Rust Tests (macos)` job.
* **Tool `search`** (`query`, optional `path` and `limit`). A line matches
  when it contains the query, ignoring case. **Rank:** score 1, plus 4 when the
  query is a whole identifier on the line, plus 2 when the file name contains
  it; highest first, ties to fewer path components, then path, then line.
  **Cap:** the default is 20 (`--limit` changes it), a call may ask for 1 to
  200, and a capped result says how many matches it left out. The walk skips
  symlinks, `.git`, `node_modules`, `target`, `.venv`, files over 1 MiB and
  anything that is not UTF-8 text.
* **Tool `map`** (optional `path` and `limit`). The repository map: symbols
  and the files that declare them, ranked by references. A symbol's references
  are the whole-identifier occurrences of its name across the scanned files,
  minus its declarations (`impl` blocks are not symbols); declarations that share a name share one count. Symbols sort by
  references, then fewer path components, path and line; a file scores the sum
  of its symbols' references. Each list is capped at `limit` (default as for
  `search`) and a capped result says how much it left out. The walk follows
  `search`'s skips and stops after 20,000 files.
* **Tool `symbols`** (`name`, optional `path` and `limit`). Where a name is
  declared across every repository under the roots, one line per declaration:
  repository, `path:line`, kind and name. A symbol matches when its name
  contains the query, ignoring case; exact matches rank first, then
  case-insensitive ones, then substrings, then fewer path components, path and
  line. Each file is read once, one at a time; the walk is the map's.
* **Tool `related_repos`** (`repo`, a directory name or a path under a root).
  The repositories it depends on and the repositories that depend on it, each
  with the package name that links them. The manifest is generated on every
  call from the workspace layout (a repository is the nearest directory with a
  `.git`, else each directory directly under a root) and from `Cargo.toml`,
  `package.json`, `pyproject.toml`, `go.mod` and Gradle files: repository A
  depends on B when A requires a name B provides (a build file's own name, or
  B's directory name). Names nobody provides are external and not listed.
* **Withheld.** A path with a component containing `nedap` (any case) is never
  read: a root there is refused, `admit` refuses it under a root, and every
  walk skips it, so neither tool can reach into one.

## ticket-sync

`ticket-sync` brings the GitHub issues in line with
`docs/specs/gold-band/tickets.yaml`. It reads the live state and prints the
plan; a second run against a converged state plans nothing.

```bash
cargo run --manifest-path scripts/Cargo.toml -p ticket-sync            # dry run
cargo run --manifest-path scripts/Cargo.toml -p ticket-sync -- --apply # write
```

It plans creations, title and body edits, sub-issue links (spec, branch,
ticket) and native blocked-by links, and lists the superseded issues it would
close. Closing is its own action: `--approve-plan` records an approval of the printed close plan (repository, issues, comments), and `--close` closes only under an approval of the plan it computes now, then uses the approval up; `--apply` never closes. Only graph issues are
ever unlinked; an unrelated issue under a graph parent is left alone. GitHub
sits behind the `Client` trait: tests fake it, and the real client drives `gh api`.

Coverage is measured with `cargo llvm-cov` (or `-C instrument-coverage` with
the matching `llvm-cov`); the unit tests cover every line except the one
executable line in `src/main.rs`.

## ticket-export

`ticket-export` exports a published issue and its sub-issues to the requirement
file Workflow B consumes, so no agent needs GitHub write access.

```bash
cargo run --manifest-path scripts/Cargo.toml -p ticket-export -- <REPO_DIR> <ISSUE_NUMBER> <OUT_FILE>
```

`<REPO_DIR>` is a repository holding the per-repository setup file
(`docs/agents/setup.yaml`, written by `scripts/repo_setup.py write`); the tracker
the issue is read from comes from that file, and a repository without a valid one
is refused. The file written holds the issue's title, body, acceptance criteria,
sub-issues and blocked-by list, and nothing else (no comments, no edit history).
A missing issue, a closed blocker, an unreadable tracker or a missing setup file
exits 1 with a message and writes no file. GitHub is reached only through a
GET-only trait (the real one runs `gh api -X GET` under a timeout); the tests
read recorded API answers from `crates/ticket-export/tests/fixtures`.

## ticket-publish

`ticket-publish` publishes the tickets Workflow C drafted, to the tracker the
repository's own setup file names, and only that tracker.

```bash
cargo run --manifest-path scripts/Cargo.toml -p ticket-publish -- check <REPO_DIR>
cargo run --manifest-path scripts/Cargo.toml -p ticket-publish -- publish <REPO_DIR> <TICKETS_FILE>
```

`check` exits 0 when `<REPO_DIR>/docs/agents/setup.yaml` is valid enough to
publish to (version 1, a GitHub tracker, a non-empty label vocabulary) and 3
otherwise; it sends nothing. `publish` reads `<TICKETS_FILE>`, JSON of the form
`{"tickets":[{"title","body","labels"}]}`, validates it against the setup file
first (every label must be in the file's `labels`) and refuses with exit 3,
creating nothing, when the setup file or any ticket is invalid. It then creates
one issue per ticket, in order, with `gh issue create` under a timeout, and
prints each URL as it is created; a failure part way stops there with exit 1.
Workflow C runs it through the gate shim, and only after the publication manual
check (docs/GOLD-BAND.md). Tests fake the tracker behind a trait and run the
real process seam against a fake `gh` script.

## transcript-mix

`transcript-mix` reads agent transcripts and reports, per agent, which tools
were used to find code (search, reads, the language-server tool, the knowledge
pages) and how many tokens each task spent exploring.

```bash
cargo run --manifest-path scripts/Cargo.toml -p transcript-mix -- \
    --root <DIR> --since 2026-11-01 --until 2026-12-01
```

`<DIR>` holds one directory per agent, each with `*.jsonl` transcripts (one
transcript is one task; Claude Code's line format). A task is in the window
when its first timestamp is at or after `--since` and before `--until`
(`YYYY-MM-DD` or RFC 3339; `--until` defaults to no limit). Exploration tokens
are the `output_tokens` of the assistant turns that call tools, up to the first
turn that calls an editing tool. The test fixtures are synthetic transcripts
under `crates/transcript-mix/fixtures`.

## agent-launcher

`agent-launcher` starts one Gold Band agent id with its reach level's tool
configuration, and refuses, before it starts anything, a level the agent cannot
hold on this OS. It reads the support record, `registry/reach-levels.yaml`:

* `agents:` each agent's `command` and, per reach level (`read`, `read-web`,
  `research`, `write-scoped`, `run-one-command`, `write`, `full`), its tool
  `args` and a `support:` cell for each OS (`linux`, `mac`), one of `verified`,
  `unverified` or `unsupported`, and a `check:` for each OS: the test that
  proves a `verified` cell (`<file>::<test>`) or the reason an `unsupported`
  one is refused. `validate` refuses a verified cell without its check. A cell
  is verified separately on each OS; a level proven on the Mac is never
  assumed on Linux.
* `ids:` each agent id, naming its `agent`, its `reach` level, an optional
  `skill_set`, the `hosts` it runs on and, for `research` and `write-scoped`,
  its `output_dir`.

```bash
cargo run --manifest-path scripts/Cargo.toml -p agent-launcher -- validate registry/reach-levels.yaml
cargo run --manifest-path scripts/Cargo.toml -p agent-launcher -- run registry/reach-levels.yaml <id> -- [agent args]
```

`validate` exits `1` for an id with no reach level, an unknown agent or level, a
`research` or `write-scoped` id without an output directory, an id whose cell
is not `verified` on a host it names, two ids for one agent, level and skill
set, or a cell missing an OS. `run` exits `1` and starts nothing when the cell
for this OS is not verified or the id is not declared for it; otherwise it
runs the agent with `AGENT_KIT_REACH` (and `AGENT_KIT_OUTPUT_DIR`) set and
exits with its status.

`run` also puts the operating-system write limit (ADR-0013) on the agent. The
launcher has one interface (`WriteLimit`) with a backend per OS; on Linux the
backend is Landlock, called directly through libc (`libc` is pinned at
0.2.186, published 2026-04-23). The writable paths come from ADR-0013's table:

| Reach level | The agent may write |
| :- | :- |
| every level | its state directories (`AGENT_KIT_STATE_DIRS`, colon separated; one that does not exist is skipped), a temporary directory made for the run (`TMPDIR` points at it, removed afterwards) and the `/dev/null` and `/dev/tty` devices |
| `read`, `read-web` | nothing more |
| `research`, `write-scoped` | its id's `output_dir` |
| `run-one-command` | its gate-output directory (`AGENT_KIT_GATE_DIR`) |
| `write` | the current directory, which must be a linked git worktree, its own git administration directory, and the shared `objects`, `refs` and `logs`; never the shared `hooks` or `config` |
| `full` | anything (no limit) |

Landlock only grants, so the files inside the worktree that tell git where its
directories are (`.git`, and the worktree's `commondir` and `gitdir`) cannot
be carved out: the launcher records them before the agent starts, puts back any
the agent changed, replaced or removed when it ends, and exits 1 saying so.
The `write` grant also covers the worktree's own git directory and the shared
`objects`, `refs` and `logs` (so a commit works and the agent can move
branches); `git fetch`, `gc` and anything else that needs a lock file in the
shared git directory itself fails under `write`. Before every `write` launch the launcher also checks the git layout is the one
git makes (`<common>/worktrees/<name>` with the common directory outside the
worktree, `gitdir` pointing back at the worktree, `objects`, `refs` and `logs`
not symlinks), so a pointer an agent changed and a killed launcher never put
back is refused rather than trusted. The launcher also refuses `write`
when the shared `hooks`, the shared `config` or the effective `core.hooksPath`
(every occurrence, in the repository config and the system and global git configs; relative paths against the worktree, `~/` expanded) resolve inside, or to a
parent of, anything it would grant. A guarded file that cannot be put back is
reported as such and the exit code is 1. The check reads paths before the
grants are opened, so a path swapped in that gap is not caught (the agent only
starts after both). The limit protects what git runs
outside the sandbox: git the agent runs itself (with `-c` or `GIT_CONFIG_*`)
stays inside the same Landlock limit. A directory granted from
configuration may not lie inside a `.git` directory. Also, `write` is refused in a primary checkout (its `.git`
directory holds hooks and config beside everything else) and in a repository
whose config enables `extensions.worktreeConfig` or includes another file (a `config.worktree` in the worktree's own
git directory could set `core.hooksPath`). The network is never limited, and
reads are limited only by the read limit below. The launcher refuses to start the agent, exit 1, when Landlock is
missing, disabled, or older than ABI 3 (Linux 6.2); it never runs an agent
without its limit. The tests run without root and need a kernel with Landlock
ABI 3.

On macOS the backend is `sandbox-exec` (`/usr/bin/sandbox-exec`, never looked up
on `PATH`). The launcher starts the agent as `sandbox-exec -p <profile> -D
name=path ... <agent command>`. The profile allows everything, denies every file
write, then grants the same directories (`subpath`) and devices (`literal`) as the
table above. Paths reach the profile only as `-D` parameters, made canonical
first (`/var` is `/private/var`, and Seatbelt checks the resolved path), and the
profile uses no regular expression. After the grants come the denials, which
Seatbelt can place inside a grant, where Landlock cannot: the three files that
locate the git directory (`.git`, `commondir`, `gitdir`) cannot be written,
replaced or renamed at all; the shared `hooks`, the shared `config` and every
effective `core.hooksPath` are denied; each granted directory itself cannot be
renamed or removed (its contents stay writable), so a granted directory cannot be
moved away and replaced; and `file-link` is denied (so `git clone --local` and hard-link based tools fail under `write`), since a hard link to a hook
made inside the worktree would otherwise be writable by its new name (the
profile checks paths, not inodes). A symlink may be made, but a write through
it is checked against where it leads. The layout check, the overlap check and the
guard that puts the pointer files back run exactly as on Linux, before and after
the agent. Before it starts the agent the launcher runs `sandbox-exec` once with
the finished profile and `/usr/bin/true`, so a profile that does not load, or a
`sandbox-exec` that cannot run from here (it cannot nest), refuses the launch.
It refuses, exit 1, when `sandbox-exec` is missing. The denial is `EPERM`
(Operation not permitted) rather than Landlock's `EACCES`. An agent command
that starts with `-` is refused, since `sandbox-exec` would read it as one of its
own options. The profile also denies the worktree's `config.worktree` and the
shared `objects/info/alternates`. Not covered, on either OS: the limit is a write
limit, not a containment boundary against a hostile agent. Anything the agent can
ask a process outside the sandbox to do (a launchd job, an Apple event, a
`systemd --user` unit) is outside it, and git state files inside the
granted git directory (`rebase-merge/git-rebase-todo` with an `exec` line), a
repository nested in the worktree, a `core.hooksPath` set through a global
git config's `include` (only the repository config's includes are refused),
and files in the worktree that the owner's own tools run (`.envrc`, a `Makefile`, an editor task) are writable. CI runs this crate's
tests on a macOS runner (the `Rust Tests (macos)` job), where the enforcement
tests start a stub agent under the real `sandbox-exec`. The same job runs
`code-nav`'s `read_reach` test (below), which starts the server through this
launcher at `read` reach.

**Claude's cells.** The record's `claude` agent holds all seven levels, each
verified on Linux and on macOS by `crates/agent-launcher/tests/claude_reach.rs`
(one test per level, `claude_reach_<level>` with `-` as `_`; the record cites each). The test
starts the launcher binary on the real record with the command swapped for a
stub shell script (never Claude, never `~/.claude` or a credential: `HOME` is a
scratch directory) under the real write limit, and checks three things: the
level's tool arguments reach the process (a `--tools` list, a `--disallowedTools`
deny list of every powerful tool the level lacks, `--permission-mode=dontAsk`,
`--strict-mcp-config`; `full` is `--permission-mode=bypassPermissions` alone);
the writes the operating system allows are exactly the table above; and the
record's cell for this OS is `verified` and cites that test. It runs in the
`Rust Tests (macos)` job and the Linux workspace job. What it does not prove is
how a live Claude Code reads those flags: that was checked against its
`--help`, and a live run needs the owner's credentials. `run-one-command`
pre-approves no Bash command; a node adds `--allowedTools 'Bash(<command>)'`
after `--`, and the OS limit confines where that command may write.

**Pi, OpenCode, Hermes and Antigravity** (`tests/other_reach.rs`, same method,
stub commands, one test per verified cell and one per refused example). A level an
agent cannot express from launcher arguments is `unsupported`, the record's
`check` field says why, and the launcher refuses an id naming it (nothing is
downgraded). Pi holds `read`, `write-scoped`, `write` and `full` through its
`--tools` allow-list (it has no web tool and no command allow-list, so
`read-web`, `research` and `run-one-command` are refused). OpenCode (its CLI has
no tool or permission flag) and Hermes (toolset fixed over ACP) hold `full`
only. Antigravity documents no tool, permission or sandbox flag, so all its cells are
refused on both macOS and Linux (the Linux command is declared, but it cannot
pin a level any more than the macOS one).

For an id that names a `skill_set`, `run` also exports `AGENT_KIT_SKILL_SET`
with that name and `AGENT_KIT_SKILLS_DIR` with the directory setup builds for
it, `<GOLD_BAND_HOME or $HOME>/.gold-band/skill-sets/<id>`; an id without one
exports neither.


**The read limit (ADR-0013, #666).** On the work Mac every launch of a
personal-instance agent (any agent but `claude-work` and `codex`) also denies
`file-read*` and `file-write*` under the employer roots, at every reach level,
`full` included, where the profile holds only this denial and the `file-link`
denial (a hard link would otherwise read a root's file by a new name). The
roots are `instances.gold-band.employer_roots` in the overlay
(`~/.config/agent-kit/overlay.yaml`, or the file `AGENT_KIT_OVERLAY` names;
the personal account's own overlay may use `instances.gold-band-personal`),
absolute or `~/` paths; no path is built in. A read, a listing, a stat, a link
or a move out of a root fails with `EPERM`; the agent's repository, the
toolchain and its state directories stay readable. The launch is refused (exit
1, nothing started) when the overlay names the work instance without roots,
when it cannot be read, when the run's own worktree or a granted directory lies
inside a root, and when the backend cannot apply the limit. Landlock cannot
deny a read, so on Linux, which holds no employer clones, a declared root is
refused. `agent-launcher read-limit-check` exits 0 when no read limit is needed
(the overlay names no work instance) or `sandbox-exec` can apply it, and 1 with
the reason otherwise; setup runs it on the work Mac and fails the run when it does not pass.
`tests/personal_read_limit.rs` proves the refusal on the `Rust Tests (macos)`
job.

## build-tracker

`build-tracker` is the port of the build tracker the `/flow` skill drives
(`skills/workstation/build-tracker/build-tracker.mjs`, built from
`council/ts/src/build-tracker-cli/` and `council/ts/src/contexts/build-tracker/`):
`init`, `sync`, `path`, `refresh`, `render` and `set`, with the same usage text,
exit codes (0, 1 for an error with its JSON object, 2 for bad arguments), state
file layout under `$XDG_STATE_HOME` (else `~/.local/state`)`/agent-kit/build-tracker`
and page. For every scenario under `crates/build-tracker/tests/fixtures/` the
stdout, stderr, exit code, state file and page are byte for byte what the node
runner wrote. Outside those scenarios the output is the same except for the
known differences below.

Known differences from the node runner:

- A number beyond 2^53, or beyond what a `u64` holds, in a spec, an assignment
  or a state file parses differently from a JavaScript `Number`.
- A lone UTF-16 surrogate escape in a state file reads as U+FFFD.
- Some V8 JSON error texts and rare errno wordings differ.
- A program's output is not capped at 64 MiB.
- When the rename that writes `--out` fails, the outcome is non-deterministic.
- `render --tickets FILE [--repo OWNER/REPO]` (one page for a whole spec from its
  ticket file and GitHub, grouped by branch, B0-07) exists only in the node
  runner. The port does not have it yet, so the node runner's `--help` text is
  longer than the port's and the `usage` golden predates it. The port gains the
  option, a reader for the ticket file's YAML subset and the regenerated `usage`
  golden when the node runners are ported (spec #392, story 119).

```bash
cargo run --manifest-path scripts/Cargo.toml -p build-tracker -- --help
```

The files, the clock, `gh` and `git` sit behind the traits in `src/ports.rs`;
`src/adapters/` holds the real ones. It takes no registry crate: the state file
keeps its keys in the order JavaScript enumerates them and its numbers print as
JavaScript prints them, so `src/json.rs` reads and writes that JSON itself.

The parity proof is `src/tests/scenarios.rs`. Each directory under
`scripts/crates/build-tracker/tests/fixtures/` is a scenario (inputs: a spec
repository, an initial state, what a fake `gh` answers; steps: command lines)
with an `expected.json` holding the exit code, stdout, stderr and every file
left under the state home for each step, **generated by running the node
runner** on those same inputs with the clock fixed:

```bash
node scripts/crates/build-tracker/tests/regenerate-fixtures.mjs [scenario ...]
```

The Rust tests replay every step through the library and compare byte for byte.
The only value that differs between runs is the page's `updated YYYY-MM-DD HH:MM
UTC` stamp: the node runner is run with its clock fixed (`fixtures/fake-clock.mjs`)
and the replay uses a fixed clock too; a separate test runs on the real clock and
compares the page with the stamp normalised. Regenerate the goldens only when the
node runner's behaviour changes on purpose.

## model-gateway

`model-gateway` is the Rust port of the model gateway
(`skills/workstation/model-gateway/model-gateway.mjs`, built from
`council/ts/src/model-gateway-cli/` and `council/ts/src/contexts/model-gateway/`),
ported command by command behind the same command line and JSON contract. Every
command is ported; the `model-gateway` command on PATH runs this binary when it is
installed and the node runner otherwise (see "Swapping the binary in", below).

The gateway's own tests are vitest tests over in-memory ports, so they cannot be
pointed at a binary. Parity is proved the way the build tracker's is: goldens
generated by the node runner, replayed through the library.

Ported (byte for byte on every golden scenario): the command shell (`--help`,
subcommand resolution, `--json`/`--input`, the error object and exit codes),
`workflow start|end|list` with the state file, its lock and its key order, and
`models`, `usage`, `grant write`, `grant list`, `grant reserve` and `worktrees clean` with what they need:
configuration and the shipped defaults (`skills/workstation/model-gateway/defaults.json`,
embedded in the binary) with this machine's override, the family allowlist and
the data policy (git remotes decide the repository's identity), each client's own
model listing and login status command, the Vibe probe and its ten-minute cache,
the ledger reads and the settling of dead reservations that `usage` and the grant
commands do first, grants with the refuse-inside-a-repository rule and their
lock, a start reserved against a grant's `max_starts` in one locked ledger update
(`grant reserve`; `tests/concurrency.rs` runs the real binary with parallel processes and
checks that exactly the grant's starts pass), and the kept-worktrees file with `git worktree
remove` and `git branch -D` (`worktrees clean`), and `run` for every client (vibe, agy, claude,
opencode, hermes, codex): launchers, the retry, climb and swap, cost estimation and cap or grant reservation,
the read-only proof (critique's `routing.json` is embedded like `defaults.json`), throwaway worktrees, the
run directory, `result.json` and the ledger line. A writing run is ported too: its worktree under the
repository's common directory on a branch of its own, entered in the kept list, the other checkouts of the
repository watched and held to the state the delegate found them in, the Codex git-writable directories and
the repository's brief, the staging and listing of the delegate's work, and the caller's check run with
`sh -c` in the worktree, held to the run's time limit and to what its grant leaves. `batch` is ported too: the
plan read and checked as a whole (its dependencies, every delegation's client model, data policy, workflow and
grant, and the sum of the metered estimates reserved together in one locked ledger update), a refused plan
reported with every problem and holding nothing, each delegation run through the same single-delegation path,
independent delegations in parallel at most `max_parallel_per_client` per client (a `std::thread::scope` whose workers report over a channel, with a
mutex-and-condvar slot table, and a slot of the client each attempt really runs on), a delegation that fails for
good skipping only what depends on it, and the reservations of delegations that never ran given back with reason
`not-run`. The binary swap is done (below). The engine guard is not part of `run` or `batch` and is not ported.

Known differences from the node runner: a zone-less date-time in a state file
reads as UTC, not local time (the goldens run with `TZ=UTC`); `Date.parse`'s
legacy non-ISO formats (`2026/11/03`, `2026-1-03`, a `+0200` offset) are not accepted; an empty
`HOME` resolves the state directory under `/` rather than the working directory;
a failure to remove the lock file is not reported; a lone UTF-16 surrogate
escape reads as U+FFFD; the V8 JSON error texts and errno
wordings are the same where `src/json.rs` and `src/adapters/fs.rs` reproduce
them, and differ for rare ones. Also different: a ledger line that is not a JSON
object is skipped (the node runner stops on it), a run line missing a field the
runner reads reads that field as empty or zero, the Vibe probe cache entry
is the same JSON but the file's change time is read as seconds times 1000 plus nanoseconds over a
million, so a cache written by the other runner may count as a miss and be probed again, the shipped
defaults are embedded in the binary instead of read from a file beside it, and a grant's random id
suffix comes from the standard library's hasher rather than from `randomBytes`; a client named like an object property (`constructor`) is an unknown client, where the node runner trips over the property; a kept worktree with no `path` (a hand-edited kept-worktrees file) is a failed entry with its own reason, where the node runner passes `undefined` to git and cannot delete it by hand, and a JSON `null` entry in that file is treated like any entry with no path; the OS error words of a worktree directory that cannot be deleted by hand differ.

Also different in `run`: the tools a client's `extras.env` adds are ordered by name (`BTreeMap`) rather than by insertion (only an in-process caller can send them); a surrogate pair in a client's output is cut by characters, not UTF-16 units, where the two differ; a `named_reset` past the last moment a JavaScript `Date` holds gives none, as the node runner gives an invalid date; an error that is not one of the gateway's own (a file that cannot be written) has the code `unexpected`; the process adapter calls `kill` through an `extern "C"` declaration (the only `unsafe`), since std cannot send SIGTERM; and the monotonic clock is `Instant`-based, frozen at 0 in the goldens as the node clock fake freezes `performance.now`.

```bash
node scripts/crates/model-gateway/tests/gen-scenarios.mjs        # the scenario inputs (gen-scenarios-clients.mjs: models, usage, grants; -reserve and -worktrees: grant reserve, worktrees clean; -run: read-only `run` through fake clients)
node scripts/crates/model-gateway/tests/regenerate-fixtures.mjs  # the goldens, from the node runner
cargo test --manifest-path scripts/Cargo.toml -p model-gateway   # SCENARIO=<name> replays one scenario
```

The scenarios for `models`, `usage` and the grant commands write fake client programs (shell
scripts under the sandbox's `bin/`, which is the whole `PATH` of those steps, plus a `git` link
where a step asks for one), so what `models` lists, what the Vibe probe cache holds and what the
status commands answer are the same in both runners; a grant id's random suffix, and the
change time and inode in the probe cache, are masked (`@RAND@`, `@N@`) in the goldens.

The `run-*` scenarios drive `run` through fake clients that log how they were launched (argv,
working directory, the environment that matters, stdin) and print canned output; a step may make
repositories with the host's git (`"git"`), snapshot more directories (`"roots"`), and points
`TMPDIR` into the sandbox. The random parts of run ids and temporary directories are numbered per
step by first appearance (`@RAND1@`, `@TMP1@`, in file names too), and the node runner's clock and
monotonic reading are fixed (a run's duration is 0). They are replayed with every other
scenario (`SCENARIO=run-vibe`, or `SCENARIO=run-` for all of them, replays only those).

### Swapping the binary in

`installer/model-gateway` (the `model-gateway` command setup puts on PATH) runs, in
this order: the Rust binary at `~/.local/share/agent-kit/model-gateway/model-gateway`
(`AGENT_KIT_MODEL_GATEWAY_BIN` names another), unless the node runner is asked for;
else the node bundle in the Claude Code home, as before. Setup runs
`scripts/install-model-gateway-rust.sh`, which builds the crate from this workspace
(`cargo build --release --locked -p model-gateway`, std only, so nothing in the lockfile
has an age to question), checks that the new binary answers `--help`, and moves it into
place atomically. A machine with no cargo, no workspace (a bundle install) or no clone
skips the step and keeps running the node runner; a failed build or a binary that does
not start leaves whatever was installed untouched, so a caller never sees a half
installed gateway and nothing changes for it until the binary exists.

Rollback to the node runner needs no setup run and no rebuild:

```bash
AGENT_KIT_MODEL_GATEWAY=node model-gateway usage --json '{}'   # one call, or export it for a shell
touch ~/.config/agent-kit/model-gateway-node                   # every call, and setup stops installing the binary
rm ~/.config/agent-kit/model-gateway-node                      # back to the Rust binary
```

The command does not probe the binary before running it: one that is installed but broken (a truncated file) fails its calls until the rollback above is used. Deleting `~/.local/share/agent-kit/model-gateway/model-gateway` has the same effect
until the next setup run. Both runners read and write the same state files (ledger,
workflows, grants, kept worktrees) in the same format, so switching either way in
the middle of a workflow loses nothing; the node runner is not removed.

## critique

`critique` is the Rust port of the critique runner
(`skills/workstation/critique/critique.mjs`, built from `council/ts/src/critique-cli/`
and `council/ts/src/contexts/critique/`), ported command by command behind the same
command line and JSON contract. The node runner stays the one the critique skills run
until the last slice swaps it (this page will say how to roll back then). The crate
takes no registry dependency: it reuses the ordered JSON, the ports and the adapters of
`crates/model-gateway` (a path inside this workspace).

* **Ported so far:** the command line (usage, `--input`/`--json`/standard input, error
  and exit codes, `--strict`), `allow`, `check-adrs`, `check-plan` and `check-rewrite` (the deterministic checkers; check-plan reads the routing the node runner reads, the shipped `routing.json` embedded in the binary with this machine's override merged over it and the whole result validated; its refactoring-plan format is a hand-written port; the regular expressions of the
  node runner are hand-written scanners here, since the crate takes no registry dependency), `budget` (the Vibe spend of a feature's flow keys, Vibe's plan windows as the gateway reads them, and what today's Antigravity launches would do to each stage), `check-tickets` (the ticket checker's cycles, dangling blockers, criterion counts and story coverage, over GitHub sub-issues and their native blockers or local ticket files), `cold-read`, `gate` with the `critique-commit-gate` and `critique-pr-gate` hooks (the commits, pushes, merges and `gh pr create` calls a command line runs, read by a port of the node runner's shell reader, each judged from git and the local review, triage and gate indexes; a merge is vouched for commit by commit through `git merge-tree`), `fan-out` (every critic of a prepared run in its own packet copy under the read-only tripwire, the same defect raised twice merged, each finding given the evidence check's pre-verdict, and a failed critic retried once through a fallback; the critics of a wave run one after another in roster order where the node runner starts them side by side), `facts` (code facts, import and source scans, history facts; the scans' regular expressions run on a small JavaScript-semantics engine, `js_regex.rs`), `record` (the verdicts weighed against fan-out's pre-verdicts and the refute round, the report and the summary, the ledger lines, and a feature review's gate and a ticket review's record), `refute` (the refute round: each accepted critical finding goes to the first routed client, base before deep, whose family neither raised it nor is the orchestrator's, through the agy, Codex, Claude or Vibe client; the answer counts only with evidence the frozen packet holds, and each refuter's launch is logged in both ledgers; a Vibe refuter searches through the runner's own search server as a Vibe critic does, and the refuters of a round run one after another where the node runner starts them side by side), `reviews`, `state` (one feature's flow state computed from the ledger, run directories, review and gate indexes and the tracker, cached per flow, with its single next step), `stats` (both ledgers merged as the node runner merges them, figures, calibration, removal candidates and the rendered report) `triage` (the rules, the gates and the triage model's question; a `gh` port and the tracker, which `check-tickets` reuses) and `setup` (`/flow setup`'s machine checklist: `list`, `check`, `quota` and `kill-switch`; the hooks-installed check reads Claude Code's `settings.json` and Codex's `hooks.json` and `config.toml`, and computes the hash Codex trusts each hook by; the kill-switch check asks the gate itself; the sign-in and version reads and the trivial probe go through the critic clients, and each launch is one invocation line on the ledger; a Vibe client's read-only proof runs the search-reach proof and then one live prompt through `run_typescript`. The proof's inputs hash the bundled `critique.mjs`, as node does, so a result recorded by either runner is current for the other). Every command of the node runner is ported. `search-server` is ported: the MCP stdio
  server reads standard input a line at a time and answers each request as it arrives, and a critic's `grep` pattern
  runs on `js_regex.rs`, which follows JavaScript's own case folding under `i`. A pattern using syntax the engine does
  not implement (a back reference, a lookbehind, a named group, an octal or `\c` escape) is answered as an invalid
  pattern where node would search with it, and text is matched by characters where node matches UTF-16 code units.
  Known differences: JSON syntax-error context is windowed in
  characters, not UTF-16 units, a lone surrogate in a string reads as U+FFFD, JSON nested hundreds of thousands deep overflows the stack instead of reading as the wrong shape, an empty or relative `HOME` is read as an absolute path, and an `--input` path is normalised lexically (a trailing `/` or `..` through a symlink). Sorting that follows `localeCompare(…, 'en')` (`collate.rs`, used by `reviews`) matches node for all ASCII, every one-character pair checked; text beyond ASCII sorts after all of ASCII by code point, where ICU sorts an accented letter with its base letter.
* **Infrastructure for the model commands** (no command uses it yet; unit-tested, not golden-tested): the run manifest
  (`manifest.rs`, `roster.rs`), the flow budget's invocation line and the gateway ledger record (`flow_budget.rs`),
  the Vibe client's `ask`, sign-in and readiness (`vibe_client.rs`, `clients.rs`: agy, codex and claude read as not wired
  until their commands arrive), the read-only toolset text (`critic_tools.rs`), the plan limit read through the
  gateway's code (`plan.rs`) and the shipped files read from `assets_root` (`assets.rs`). `Runtime.gateway` is the gateway's
  own runtime, which the ledger, the plan limit and the client launches go through.
  The findings a critic reports and the verdicts the orchestrator gives them (`findings.rs`, `verdicts.rs`: the reply's
  last fenced block read against the output contract, a verdict's reader, and the stage rules for what a verdict may
  say, a severity override, the candidate questions and the second-round count) are replayed over a generated corpus
  (`tests/gen-verdicts-corpus.mjs` writes `tests/fixtures/verdicts.json` from the node runner's own functions).
* **Search wiring.** A Vibe critic of `fan-out` or `refute` searches through the runner's own `search-server`
  (`search_client.rs`, `critic_run.rs`'s `CriticTools`, `vibe_search.rs`), as node's `CriticTools` does: the installed
  agent file and the home's `config.toml` must let it (`searchBlocked`), the server must pass the reach proof (made once
  per command, in `<run>/search-proof`: a packet with one file, a file outside it and two symlinks out of it, then the
  nine calls that try every way out), and the server must start on the critic's own copy; otherwise it reads alone and
  its run says why (`search_failure`). The server is stopped with `pkill -f` once the critic ends. The server command is
  the Rust runner's own executable (`Runtime.search`, set from `current_exe()`); a test runtime leaves it unset, so a
  Vibe critic reads alone there. **Known difference:** node's server command is `node critique.mjs search-server ...`, so
  the command line in the MCP config of a Vibe launch with search differs, and the goldens cannot cover such a launch.
  They cover the launches that read alone, and the unit tests and `tests/binary.rs` (the real binary serving the reach
  proof against a real packet) cover the rest.
* **Setup goldens.** `tests/gen-scenarios-setup.mjs` covers every setup action with fake `vibe` and `agy` programs in the sandbox's `bin/`. A Vibe client's read-only proof is not among them, for the reason the search wiring's goldens are not: its launch carries the search server's command line. The harness reads the hash of the bundled runner as `@RUNNER@`, since it changes with every build.
* **Goldens.** `tests/gen-scenarios.mjs` writes the scenario inputs and
  `tests/regenerate-fixtures.mjs` runs the node runner over each with fake programs in a
  temporary sandbox (fixed clock, fixed `HOME`, git repositories made in the sandbox) and
  records stdout, stderr, the exit code and every file left under the sandbox. The unit
  tests replay each scenario through the library and compare byte for byte:

```bash
node scripts/crates/critique/tests/gen-scenarios.mjs
node scripts/crates/critique/tests/gen-scenarios-rewrite.mjs   # check-rewrite's scenarios
node scripts/crates/critique/tests/gen-scenarios-plan.mjs   # check-plan's scenarios (plans, routing overrides)
node scripts/crates/critique/tests/regenerate-fixtures.mjs   # the goldens, from the node runner; the folder the runner sits in reads as /assets
cargo test --manifest-path scripts/Cargo.toml -p critique    # SCENARIO=<name> replays one scenario
```

### Swapping the critique runner in

The skills and the three critique hooks call the critique skill's launcher,
`skills/workstation/critique/critique` (`sh <critique dir>/critique <command> ...`),
not `critique.mjs` directly. The launcher runs, in this order: the Rust binary at
`~/.local/share/agent-kit/critique/critique` (`AGENT_KIT_CRITIQUE_BIN` names another)
with `AGENT_KIT_CRITIQUE_ASSETS` set to the skill directory, so it reads that
directory's prompts, roles, lenses, schemas and `routing.json`, unless the node runner
is asked for; else `node critique.mjs` beside it, as before. The Rust runner
answers every command itself.

Setup runs `scripts/install-critique-rust.sh`, which builds the crate
(`cargo build --release --locked -p critique`), checks that the new binary answers
`--help`, and moves it into place atomically. No cargo, no workspace or a cloud session
skips the step, and a failed build leaves whatever was installed untouched.

Rollback, without a setup run:

```sh
AGENT_KIT_CRITIQUE=node sh <critique dir>/critique ...   # one call or one shell
touch ~/.config/agent-kit/critique-node                 # every call, until removed
```

`tests/test_critique_rust_swap.py` covers the launcher and the installer with stub
programs; `tests/binary.rs` runs the real binary.

## Measuring line coverage

One command, from the repository root, on the Linux workstation and
in CI (Linux only: one code-nav-eval test needs `/dev/full`):

```bash
scripts/coverage.sh
```

It builds the workspace tests with rustc's own `-C instrument-coverage`, merges
the profiles with `llvm-profdata`, prints a per-file `llvm-cov` report and
exits `1` below 100 percent line coverage (`COVERAGE_MIN_LINES` overrides the
floor for experiments). `src/main.rs` files are excluded: they are the one
exit-wrapper line each crate documents. Test code is excluded too, since the
standard is coverage of production source: unit tests live in `src/tests.rs`
(`#[cfg(test)] mod tests;`, or `src/<module>/tests.rs`) and integration tests
under `tests/`, and any file matching `/tests.rs` or `/tests/` is ignored.
Keep test helpers and stubs in those files, never in a production file. No crate is added, so the cooldown is
untouched. It needs a rustup toolchain with the `llvm-tools` component (so
`llvm-profdata` and `llvm-cov` come from the LLVM that built that `rustc`; the
script refuses a mismatch) and `jq`.

On NixOS take rustup, a linker and jq from nix, and install the toolchain once:

```bash
nix shell nixpkgs#rustup nixpkgs#gcc nixpkgs#jq --command \
    rustup toolchain install stable --profile minimal -c llvm-tools
nix shell nixpkgs#rustup nixpkgs#gcc nixpkgs#jq --command scripts/coverage.sh
```

Do not use the nixpkgs `rustc`/`cargo-llvm-cov`: the latter fails its own tests
and the former's instrumented binaries segfault while writing the profile
(checked with rustc 1.99.0 / LLVM 21.1.8), and nixpkgs LLVM tools reject the
profiles of other rustc builds. CI runs the same script as the last step of the
workspace workflow, after `rustup component add llvm-tools`. From rustc 1.100
the closing brace of an `if` without an `else` counts as its false path, and a
closure that never runs counts as a missed line, so a test takes each false
path that can happen, and a branch whose false path cannot happen is written
without it.

## The cooldown at resolve time

`.cargo/config.toml` at the repository root sets the cooldown for every cargo
command run in the repository:

```toml
[registry]
global-min-publish-age = "7 days"
```

From Rust 1.100 the resolver then skips any version published less than 7 days
ago unless `Cargo.lock` already holds it, and says what it held back. Setup
writes the same key into `~/.cargo/config.toml` with the packages-proxy block,
and makes sure a stable toolchain is at least 1.100 (`rustup update stable`
when it is older; a default pinned to an older release is a failure). An older
cargo ignores the key with a warning.

The packages proxy is the second floor: its cargo index leaves out every
version younger than 7 days and keeps each entry's `pubtime`, so cargo's own
check works through it. Resolution goes through it wherever setup or
`setup-package-proxy` configured `~/.cargo/config.toml`. `cargo install` skips
cargo's cooldown, so the proxy is what holds it there.

The loopback crate-gate (`127.0.0.1:17342`) that used to replace crates.io here
is retired: the proxy and the cargo setting replace it. Stop a gate a machine
still runs with `systemctl --user disable --now crate-gate` (launchd on the
Mac) and delete `~/.config/systemd/user/crate-gate.service`.

### crate-age on a workstation

`scripts/install-crate-age.sh` builds crate-age from your standards clone
(`~/Projects/JorisJonkers-dev/standards`, or `STANDARDS_DIR`) with `git archive`
at the pinned commit `STANDARDS_REF` — the clone's working tree is never
touched — borrowing cargo from `nix shell` on NixOS when none is on `PATH`, and
installs it to `~/.local/bin`. The Mac installs it from the standards repo the
same way.

## The pre-commit hook

`.githooks/pre-commit` runs **crate-age** over any staged `Cargo.toml` or
`Cargo.lock` change. Enable it once per clone:

```bash
git config core.hooksPath .githooks
```

(plus `chmod +x .githooks/pre-commit` if the checkout lost the executable
bit). How the hook obtains crate-age — it never copies it in:

1. `$CRATE_AGE` — a full path to the crate-age executable;
2. `crate-age` on `PATH` — what `scripts/install-crate-age.sh` installs;
3. neither — no age can be proven, so the commit is **refused (exit 2)** with
   the reason printed.

`tests/test_crate_cooldown_hook.py` commits to a scratch repository against a
fake crate-age and asserts on whether the commit exists: exit 1, exit 2 and a
missing checker are all refused; exit 0 and a commit that touches no Cargo file
pass.

## The pull-request workflow

`.github/workflows/crate-cooldown.yml` runs on every pull request (and on
pushes to `main`), so every `Cargo.toml` or `Cargo.lock` change is checked. It
checks out this repository and the standards repo at a pinned commit (the
standards repo is private, so the job mints a release-app token for it; the app
must be installed on that repository), builds crate-age, runs it over the
checkout, and builds and tests the workspace with cargo resolving through the
packages proxy and the cooldown setting. A too-young or unprovable
version fails the job, so the same change a contributor cannot commit cannot
merge either. Move the pinned commit here and in `scripts/install-crate-age.sh`
together when the standards repo tags a release.

## The security-fix waiver

A crate version may need to land *before* it cools down — a security fix, for
which waiting seven days is the worse risk. The route:

1. Open the pull request. The cooldown gate fails it (exit 1: too young, or
   exit 2: age unproven), and cannot be argued away in the PR body.
2. A **named human owner reviews the security case** (advisory link, why the
   young version is the fix, why pinning lower does not work) and approves.
   The approval is recorded by that owner in
   [`docs/waivers/crate-cooldown.yaml`](waivers/crate-cooldown.yaml):
   package, exact version, reason, PR link, approver, approval date and an
   **expiry date** — never later than `approved_at` plus 7 days.
3. Only the approving owner merges the PR, with the failing gate overridden
   by that human approval, and the waiver entry is linked from the PR.
4. **The waiver expires.** On its `expires` date the entry is dead: it is
   removed from the ledger, and from then on the gate blocks that version
   again unless its seven days have passed. A waiver is never renewed.

How crate-age itself honours a waiver is defined by the standards repo
(ADR-0014); the ledger here is this repository's approval record for it.

## The Gold Band workflow lint

`crates/workflow-lint` is a library plus a CLI (`workflow-lint --instance gold-band|gold-band-personal <FILE>...`;
exit 0 clean, 1 problems, 2 bad usage or an unreadable or invalid file) that
later tickets call. Its one registry dependency, `serde_json`, is pinned
exactly at a version older than the cooldown. It reads Gold Band's workflow
JSON (version `0.1`), bare or in the authoring wrapper with `modelBindings`,
and refuses, one line per problem, `<node>: <rule>: <message>`:

* `opus-outside-judgment`: a node binds Opus and its id does not end in
  `-escalation`, `-acceptance` or `-planning`;
* `memory-write-outside-judgment`: a node binds a memory-writing agent and its
  id does not end in `-acceptance` or `-escalation` (ADR-0012);
* `loop-not-unrolled`: the edges contain a cycle, or an edge targets
  `$new-round`; a repair loop is unrolled into fixed nodes (`review-1`,
  `repair-1`, `review-2`);
* `banned-binding`: in the personal instance only (ADR-0011), a node binds an agent or model containing `codex`; the work instance allows it.

**How a binding declares a memory write.** Gold Band's JSON has no memory
field, so the lint defines the smallest declaration: an agent id (a worker's
`provider`, a strategy's provider fields, or a wrapper binding's `agentId`)
that contains the marker `memory-write`, case-insensitive, for example
`claude-acp+memory-write`. A binding without the marker reads memory only.
Fixture workflows live in `crates/workflow-lint/tests/fixtures/`.

## The Gold Band run-start shim

`crates/gold-band-run-start` is a library plus a CLI
(`gold-band-run-start check --instance gold-band|gold-band-personal ...`, the
flags are in [GOLD-BAND.md](GOLD-BAND.md)) that every Gold Band workflow runs in
its first node. Exit 0 clean, 1 refused, 2 bad usage or unreadable input; it
writes `<report-dir>/run-start.json` through `crates/gold-band-report`. Git, the
clock, the environment and the output streams sit behind one `Host` trait:
tests fake it, and the real host is exercised too, against a temporary git
repository, so only `src/main.rs` is outside the coverage count.

## The Gold Band gate shim

`crates/gold-band-gate` is a library plus a CLI with two subcommands,
`gold-band-gate run` (the wrapper) and `gold-band-gate verify` (the read-only
copier); the flags and exit codes are in [GOLD-BAND.md](GOLD-BAND.md). The
child process, the clock and the output streams sit behind one `Host` trait:
tests fake it for every outcome, and the real host is exercised too (a real
echo, exit code, signal death, hung command killed at the deadline, missing
and non-executable command), so only `src/main.rs` is outside the coverage
count. `tests/test_gold_band_gate.py` proves the shim headless.

## coverage-gate

`coverage-gate` fails unless every file a change touches has 100 percent line
coverage. Workflow B runs it as the `gate-coverage` node through the gate shim
(`gold-band-gate run ... -- coverage-gate --repo-dir DIR`), followed by the
`copy-coverage` copier.

```bash
coverage-gate --repo-dir DIR [--base REF] [--ignore TEXT]...
```

* **The change.** The added, copied, modified, renamed and untracked files
  between the merge base of `--base` (default `origin/main`) and the working
  tree. An uncovered file the change did not touch never fails the gate.
* **The tool** is the repository's own, looked for in `DIR` and one level
  below it: `cargo llvm-cov` for a `Cargo.toml`, `npx vitest run --coverage`
  for a `package.json` naming vitest, `pytest --cov` for a pyproject.toml,
  pytest.ini, tox.ini or setup.cfg naming pytest-cov. Each writes an lcov
  report, read by one parser. A repository with none of them is refused (exit
  2) with a message naming the three.
* **The verdict.** Exit 0 when every instrumented line of every changed file
  ran (a `src/main.rs`, a language without a tool, or a file `--ignore`
  names, is printed as not measured); a changed source file the report does
  not list fails, since nothing shows it ran; exit 1 and one
  `coverage-gate: FILE: uncovered lines N-M` line per file otherwise, or when
  the tool itself fails; exit 2 for bad usage or a change that cannot be
  listed.
* The git, tool and file-system calls sit behind a `Host` trait: the unit
  tests fake it, and `RealHost` is exercised against a real git repository.

## The Gold Band finding check

`crates/gold-band-findings` is a library plus a CLI, `gold-band-findings check`;
the flags, the rules and the exit codes are in [GOLD-BAND.md](GOLD-BAND.md). The
schema check is written by hand over serde_json against
`skills/workstation/critique/schemas/finding.schema.json`, so no schema crate
joins the lockfile. The unit tests cover every library line; only `src/main.rs`
is outside the coverage count.

## ci-wait

`ci-wait` waits until every check run of one commit is terminal and prints one
JSON object; it is usable from a direct session and as a Workflow B gate.

```bash
ci-wait --repo-dir DIR --sha SHA [--repo OWNER/NAME] [--timeout-secs N] [--poll-secs N]
```

* **Setup file.** `DIR/docs/agents/setup.yaml` is required (a repository
  without a valid one, or with a flaky job lacking a name, a reason or a
  reviewer, exits 2 before any GitHub call). `--repo` defaults to its tracker repo.
* **Verdict.** `{"verdict":"green"|"red","reason":...,"repo","sha","jobs":[{name,
  status,conclusion,reran}],"reruns":[{job,reason}]}`. Exit 0 only for green, 1
  for red (a failed job, a failed rerun, a timeout, no checks ever registered,
  an unreadable answer), 2 for bad usage or a refused repository. Green needs
  at least one check run, all passing (success, neutral or skipped) on two
  consecutive looks.
* **Reruns.** A failed job listed under `flaky_jobs` is rerun once per
  invocation (`POST repos/OWNER/NAME/actions/jobs/ID/rerun`); if it fails again
  the verdict is red. A failed job not listed is red at once and never rerun.
* **Tests.** The GitHub seam and the clock are traits faked in the unit tests;
  the real `gh` bridge is driven through a fake program, with no live call.
