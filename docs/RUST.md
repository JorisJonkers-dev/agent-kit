# The Rust workspace

The estate's Rust lives in **one cargo workspace**, `scripts/Cargo.toml`, with
**one lockfile**, `scripts/Cargo.lock`, and **no git or path dependency**
outside the workspace. The rule comes from the standards repo
(`JorisJonkers-dev/standards`, ADR-0014); this page is how agent-kit follows
it, on the Mac and on the Linux workstation.

* **Resolution.** A registry dependency is pinned exactly. Cargo resolves
  through the packages proxy every workstation and CI job uses
  (`packages.jorisjonkers.dev/cargo`). There is no crate cooldown check in this
  repository (see "Resolving crates" below).
* **The workspace.** Thirty-one members. `crates/agent-kit-setup` is the canary: a
  std-only crate whose
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
  `crates/research-layout` is the papers layout validator and Workflow A's
  fetch shim and verifier (below); it takes serde and serde_json, each pinned
  exactly, and the workspace's own gold-band-report for its SHA-256.
  `crates/ci-wait` is the CI-wait gate (below); it takes
  serde_json and serde_yaml, each pinned exactly. `crates/rust-migration-guard`
  is the Rust migration guard (below), taking only serde_json. New Rust joins this
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

## Workstation secrets, kube and LiteLLM tools

Four crates replace the Python scripts of #812. Setup builds them with
`scripts/install-workstation-rust.sh` (release build, `--locked`) into
`~/.local/share/agent-kit/bin` before the tokens-and-keys step; setup skips that
step with a warning when `cargo` is missing. All four start `curl` with the
request, bearer key and cookies on its stdin, so no secret reaches argv.

* `agent-kit-keys --table registry/generated/workstation-secrets.json ensure|status|write-env`
  and `revoke NAME`: signs in to auth-api, mints and checks service tokens,
  mints per-host LiteLLM keys, stores everything through `installer/agent-kit-secret`
  (value on stdin; `AGENT_KIT_SECRET_TOOL` and `AGENT_KIT_PRIVATE_TOOL` name the
  helpers) and writes `~/.config/agent-kit/env.sh`. With no `LITELLM_ADMIN_TOKEN`
  it reads `LITELLM_MASTER_KEY` (environment, else the secret store item of that
  name) and mints the one-hour admin token itself; the paste prompt is the last resort.
* `litellm-harness [--table T] configure|run ...`: writes the `litellm-*` provider
  entries of Codex (TOML, `toml_edit` keeps the owner's formatting), Hermes (YAML),
  OpenCode (JSON) and Vibe, and runs a harness on a route with its key from the
  secret store. Setup installs `agent-kit-litellm` as a two-line launcher for it.
* `measure-litellm-budget [--agent vibe] [--url U] [--routes FILE]`: one
  `GET /key/info` (and `/team/info` for a team key), key redacted in the output.
* `workstation-kube install|check --device D ...`: a device's read-only kubeconfig
  and sudo-gated admin tokens.

The tests use a local `TcpListener` stub with the real `curl`, and fake `sh`
scripts for the secret store, `kubectl`, `sudo` and `ssh`.

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
git directory could set `core.hooksPath`). A reach level never limits the network, and
its reads are limited only by the read limit below (`confine`, after it, does both). The launcher refuses to start the agent, exit 1, when Landlock is
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

**The confinement (`agent-launcher confine`, #737).** A reach level's limit
leaves reads alone; a Pi critic needs the opposite, since its `read`, `grep`,
`find` and `ls` tools run inside the Pi (node) process itself. A sandbox that
wraps only the shell (`pi-bash-wrap`, `@yuru7/pi-sandbox`) never sees those
reads and keeps `/` readable; `confine` limits the whole process instead, and
everything it starts:

```bash
agent-launcher confine [--check] [--read PATH]... [--run PATH]... [--run-from PROGRAM]... \
    [--write PATH]... [--net-connect PORT]... [--env NAME]... -- COMMAND [ARGS...]
```

The launcher puts a Landlock ruleset on itself (on macOS it becomes `sandbox-exec` with the profile, which then execs the command) and then becomes the command
(exec), so the caller's signals and timeout reach the command and nothing is
left behind. The command inherits no more than it is given. Its environment is
`PATH`, `HOME`, `LANG`, `LANGUAGE`, `LC_*`, `TERM`, `TZ`, `TMPDIR`, `NO_COLOR`
and the certificate-store variables (`SSL_CERT_FILE`, `SSL_CERT_DIR`,
`NIX_SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS`), each when the caller has it, and
every `--env NAME` the caller names; no key, token or proxy password rides
along unnamed, and the Pi launch names only `PI_CODING_AGENT_DIR`. Every open
descriptor above the three standard ones is closed once the ruleset is on
(`close_range`, kernel 5.9), so a file, socket or pipe the caller left open is
not carried in. The ruleset handles reading
files, listing directories and executing as well as every write-side right, so
only these can be read:

| Grant | Rights | Paths |
| :- | :- | :- |
| `--read` | read, list | each path given (the critic's packet copy) |
| `--run` | read, list, execute | each path given (Pi's own `bin`, which holds `fd`) |
| `--run-from` | read, list, execute | what running PROGRAM takes (`pi`): its package (the directory right under `node_modules` or `node_modules/@scope`, else its own directory) and, for a script, its `#!` interpreter (`/usr/bin/env NAME` looked up on `PATH`) with that interpreter's install (the directory above its `bin`), links resolved; a directory that would hold the home directory or be `/` is narrowed to the file itself |
| `--write` | read, list, write, create, remove | each directory given (the run's agent directory, its `TMPDIR`) |
| system runtime | read, list, execute | `/nix/store`, `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, those that exist (`/nix/store` holds no secret by design; NixOS's `/etc` files and `/run/current-system` resolve into it) |
| system files | read | `/etc/ssl`, `/etc/pki`, `/etc/ca-certificates`, `/etc/resolv.conf`, `/etc/hosts`, `/etc/host.conf`, `/etc/nsswitch.conf`, `/etc/gai.conf`, `/etc/services`, `/etc/protocols`, `/etc/localtime`, `/etc/zoneinfo`, `/dev/urandom`, `/dev/random`, those that exist |
| devices | read, write | `/dev/null` |

The system files are what a traced Pi run (`strace -f -e trace=openat,execve`)
opened to resolve the proxy's name, check its certificate and read the time
zone. Nothing under `/proc` or `/sys` is granted (node runs without them), so
`/proc/<pid>/environ` of another process of the same user is unreadable, and
so is the command's own. With `--net-connect` the ruleset also handles TCP
(Landlock ABI 4, Linux 6.7): a connect reaches only the listed ports (443 for
the LiteLLM proxy) and no TCP port can be bound. Where the kernel has them, a
confinement also handles connecting to a pathname UNIX socket (ABI 9, so the
session bus, `systemd --user` and nscd are out of reach; name resolution falls
back to `/etc/resolv.conf`) and scopes signals and abstract UNIX sockets to
the confined processes (ABI 6). With `--net-connect` on a kernel of Landlock
ABI 10 UDP is limited too: datagrams reach port 53 alone (DNS) and
the only bind is the ephemeral source port (port 0) a first send takes. Below
ABI 10 the kernel cannot limit UDP and it stays unlimited, as TCP DNS
fallback to port 53 stays out of reach in both cases (only the listed TCP
ports connect).

Fail closed: when the backend cannot apply the confinement in full (no
Landlock, an ABI too old for `--net-connect`, a path that cannot be granted,
macOS), `confine` exits **125** and starts nothing. `--check` builds the same
ruleset, applies nothing and exits 0 (`confine: ok, ...`, then one `read`,
`run` or `write` line per grant) or 125 with the reason, so a caller can
refuse a critic before it launches one.
`tests/confine.rs` proves it on the Linux runners under the real kernel: the
granted directory reads and a file elsewhere (and one under `~/.ssh` when the
home has one) fails with `EACCES`, nothing outside is listed, another
process's and its own `/proc/<pid>/environ` are unreadable, only the
`--write` directory is writable, a connect to a port not listed and a bind
fail with `EACCES` while the listed port connects, UDP reaches port 53 and (on ABI 10) no other
port, a secret variable and an open descriptor of the caller are not there in the command,
the exit code is the
command's, a grant that cannot be made exits 125 without running it, and
`--run-from` lets a packaged script read its package while, without it, the
script cannot even be executed.

On macOS the same confinement is a Seatbelt profile built from
`(deny default)` and applied by `sandbox-exec` (ADR-0016, #904): the same
`--read`, `--run`, `--run-from` and `--write` grants, the system runtime as
traced on macOS (`/System` for dyld and the shared cache, `/usr`, `/bin`,
`/sbin`, `/private/etc/ssl`, `/private/var/db/timezone`, the resolver
socket, and stat of each grant's ancestors, all listed in the ADR), a connect
only to each `--net-connect` port (any host on that port, as Landlock), the
mDNSResponder socket for names, and no bind. The environment and descriptor
closing are the same; a program that is not granted is refused by
`sandbox-exec` with its exit 71, not 125, and a profile that cannot be applied
(the probe against `/usr/bin/true` fails, or the launcher is already inside a
Seatbelt sandbox, which cannot nest) exits 125. `tests/confine.rs` runs on the
`Rust Tests (macos)` job: the granted directory reads and the home directory,
a sibling checkout and a file elsewhere fail with "Operation not permitted",
only `--write` is writable, a connect to a port not listed and a bind fail with
`EPERM` while the listed port connects, a UDP send is refused, and a secret
variable and an open descriptor of the caller are not there. Not covered: a
confined process keeps what it inherits (open descriptors, its environment),
and the readable and writable grants are whole trees.

## build-tracker

`build-tracker` is the port of the build tracker the `/flow` skill drives
(it replaced a node bundle, which is retired, #808):
`init`, `sync`, `path`, `refresh`, `render` and `set`, with the same usage text,
exit codes (0, 1 for an error with its JSON object, 2 for bad arguments), state
file layout under `$XDG_STATE_HOME` (else `~/.local/state`)`/agent-kit/build-tracker`
and page. For every scenario under `crates/build-tracker/tests/fixtures/` the
stdout, stderr, exit code, state file and page are byte for byte what the retired node
runner wrote. Outside those scenarios the output is the same except for the
known differences below.

Known differences from the node runner:

- A number beyond 2^53, or beyond what a `u64` holds, in a spec, an assignment
  or a state file parses differently from a JavaScript `Number`.
- A lone UTF-16 surrogate escape in a state file reads as U+FFFD (and prints
  back as the escape it came from, so a line carrying one round-trips).
- JSON nested deeper than 10,000 levels is refused with an error, where V8
  would read it.
- Some V8 JSON error texts and rare errno wordings differ.
- A program's output is not capped at 64 MiB.
- When the rename that writes `--out` fails, the outcome is non-deterministic.

`render --tickets FILE [--repo OWNER/REPO]` (one page for a whole spec from its
ticket file and GitHub, grouped by branch) is ported too: `src/yaml_subset.rs`
reads the ticket file's YAML subset, `src/tickets_file.rs` checks it and builds the
state, and the `render-tickets` scenario (47 steps, error paths included) and the
regenerated `usage` golden hold it to the node runner's bytes. On
`docs/specs/gold-band/tickets.yaml` against the live repository both write the same
page but for its `updated` stamp. The issues are read eight at a time, as the node
runner does. That page needs no overlay: it is the combined multi-branch page.

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
left under the state home for each step. The goldens are frozen fixtures: they
were generated once by the node runner (retired in #808, with its regeneration
scripts) and are edited by hand only when the Rust tool's behaviour changes on
purpose.

The Rust tests replay every step through the library and compare byte for byte.
The only value that differs between runs is the page's `updated YYYY-MM-DD HH:MM
UTC` stamp: the replay uses a fixed clock; a separate test runs on the real clock and
compares the page with the stamp normalised.

### Swapping the build tracker in

`skills/workstation/build-tracker/build-tracker` is the launcher the skill's text and
`/flow` call (`sh <build-tracker dir>/build-tracker <command> ...`). It runs the Rust
binary installed as `~/.local/bin/build-tracker`; without one it says so and exits 127
(there is no node fallback any more, #808). `AGENT_KIT_BUILD_TRACKER_BIN` names another binary.

Setup runs `scripts/install-build-tracker-rust.sh`, which builds the crate
(`cargo build --release --locked -p build-tracker`), checks that the new binary answers
`--help`, and moves it into place atomically. No cargo, no workspace or a cloud session
skips the step, and a failed build leaves whatever was installed untouched.

`tests/test_build_tracker_setup.py` covers the launcher and the installer with stub
programs.

## model-gateway

`model-gateway` is the Rust port of the model gateway
(it replaced a node runner, which is retired, #808), behind the same command line
and JSON contract. The `model-gateway` command on PATH runs this binary (see
"Swapping the binary in", below).

The gateway's tests replay frozen goldens, the way the build tracker's do. They
were generated once by the node runner; the scripts that made them are gone.

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
opencode, hermes, codex, and one Pi client per LiteLLM route, `pi-<route>`): launchers, the retry, climb and swap, cost estimation and cap or grant reservation,
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

Pi (`src/pi_launch.rs`, `src/launchers.rs`, `src/outputs.rs`): one client per LiteLLM route
(`pi-vibe`, `pi-vibe-study`, `pi-opencode-go`, `pi-local`, `pi-public`, `pi-openrouter`, `pi-free`), all started by the one `pi`
program. A Pi run goes through `agent-launcher confine`, as the Pi critics do: the launch reads (or, for
a writing run, writes) its working directory, runs Pi, writes its own agent directory and reaches the
proxy's port, and nothing else. The agent directory is made per launch under the run directory, holds the
route's provider from the home's `models.json` with its key read from the secret store just now (mode
0600), and is removed when the launch ends. A launch that cannot be confined (no provider, no key, no
launcher) never starts: it is `launch-failed` with the reason in `stderr.txt`. `models` lists what the
home's `models.json` gives each route's provider, judged by the live catalogue's canonical groups
(`litellm_catalogue.rs`: usable when a deployment is, free when public or every deployment tried first is
free); the tier and notes come from the defaults entries for client `pi`, which `config_parse` expands into
one entry per `pi-*` client. A client with `data_scope: public` (`pi-public`, `pi-openrouter`, `pi-free`) is never sent a repository, and the last two run models priced at zero only (`FREE_ONLY_CLIENTS`).
The goldens `models-pi`, `run-pi`, `run-pi-unconfined` and `run-write-pi` use a fake `pi` and a fake
`agent-launcher` that records its arguments and the agent directory it was given. Different from the node
runner: with no `HOME` the home's Pi directory resolves under `/`, not the working directory.

Known differences from the node runner: a zone-less date-time in a state file
reads as UTC, not local time (the goldens run with `TZ=UTC`); `Date.parse`'s
legacy non-ISO formats (`2026/11/03`, `2026-1-03`, a `+0200` offset) are not accepted; an empty
`HOME` resolves the state directory under `/` rather than the working directory;
a failure to remove the lock file is not reported; a lone UTF-16 surrogate
escape reads as U+FFFD (and prints back as the escape it came from); JSON
nested deeper than 10,000 levels is refused with an error; the V8 JSON error texts and errno
wordings are the same where `src/json.rs` and `src/adapters/fs.rs` reproduce
them, and differ for rare ones. Also different: a ledger line that is not a JSON
object is skipped (the node runner stops on it), a run line missing a field the
runner reads reads that field as empty or zero, the Vibe probe cache entry
is the same JSON but the file's change time is read as seconds times 1000 plus nanoseconds over a
million, so a cache written by the other runner may count as a miss and be probed again, the shipped
defaults are embedded in the binary instead of read from a file beside it, and a grant's random id
suffix comes from the standard library's hasher rather than from `randomBytes`; a client named like an object property (`constructor`) is an unknown client, where the node runner trips over the property; a kept worktree with no `path` (a hand-edited kept-worktrees file) is a failed entry with its own reason, where the node runner passes `undefined` to git and cannot delete it by hand, and a JSON `null` entry in that file is treated like any entry with no path; the OS error words of a worktree directory that cannot be deleted by hand differ.

Also different in `run`: the tools a client's `extras.env` adds are ordered by name (`BTreeMap`) rather than by insertion (only an in-process caller can send them); a surrogate pair in a client's output is cut by characters, not UTF-16 units, where the two differ; a `named_reset` past the last moment a JavaScript `Date` holds gives none, as the node runner gives an invalid date; an error that is not one of the gateway's own (a file that cannot be written) has the code `unexpected`; the process adapter calls `kill` and `signal` through `extern "C"` declarations (the only `unsafe`), since std cannot send SIGTERM to a process group or catch a stop of the gateway; and the monotonic clock is `Instant`-based, frozen at 0 in the goldens as the node clock fake freezes `performance.now`. Not in the node runner at all: a launch is streamed and its client leads a process group of its own, a run's result carries `exit_code`, `signal`, `stderr_tail` and `failure`, `stderr.txt` is written for every launched run, and the run directory holds the run event log `events.jsonl` (ADR-0017; the goldens mask the process ids it names).

```bash
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

`installer/model-gateway` (the `model-gateway` command setup puts on PATH) runs the Rust
binary at `~/.local/share/agent-kit/model-gateway/model-gateway`
(`AGENT_KIT_MODEL_GATEWAY_BIN` names another); without one it says so and exits 127 (the
node runner is retired, #808). Setup runs
`scripts/install-model-gateway-rust.sh`, which builds the crate from this workspace
(`cargo build --release --locked -p model-gateway`, std only), checks that the new binary answers `--help`, and moves it into
place atomically. A machine with no cargo or no workspace skips the step and has no gateway
until the bootstrap of #807 lands; a failed build or a binary that does
not start leaves whatever was installed untouched.

The command does not probe the binary before running it: one that is installed but broken (a truncated file) fails its calls. Deleting `~/.local/share/agent-kit/model-gateway/model-gateway` leaves the command with no gateway
until the next setup run.

## critique

`critique` is the Rust port of the critique runner
(it replaced a node runner, which is retired, #808), behind the same command line and
JSON contract. The crate
takes no registry dependency: it reuses the ordered JSON, the ports and the adapters of
`crates/model-gateway` (a path inside this workspace).

* **Ported so far:** the command line (usage, `--input`/`--json`/standard input, error
  and exit codes, `--strict`), `allow`, `check-adrs`, `check-plan`, `check-rewrite` and `check-routing` (the deterministic checkers; check-routing fails when a profile of the shipped `defaults.json`, or a Pi client a stage slot or fallback of the shipped `routing.json` names, has a model the catalogue snapshot `registry/litellm-catalogue.json` marks `blocked`, `error` or `not-chat`, judging a Pi model by its canonical group's deployments, and CI runs it as `critique check-routing --json '{"catalogue":"registry/litellm-catalogue.json"}'`; check-plan reads the routing the node runner reads, the shipped `routing.json` embedded in the binary with this machine's override merged over it and the whole result validated; its refactoring-plan format is a hand-written port; the regular expressions of the
  node runner are hand-written scanners here, since the crate takes no registry dependency), `budget` (the Vibe spend of a feature's flow keys, Vibe's plan windows as the gateway reads them, and what today's Antigravity launches would do to each stage), `check-tickets` (the ticket checker's cycles, dangling blockers, criterion counts and story coverage, over GitHub sub-issues and their native blockers or local ticket files), `cold-read`, `gate` with the `critique-commit-gate` and `critique-pr-gate` hooks (the commits, pushes, merges and `gh pr create` calls a command line runs, read by a port of the node runner's shell reader, each judged from git and the local review, triage and gate indexes; a merge is vouched for commit by commit through `git merge-tree`), `fan-out` (every critic of a prepared run in its own packet copy under the read-only tripwire, the same defect raised twice merged, each finding given the evidence check's pre-verdict, and a failed critic retried once through a fallback; the critics of a wave run one after another in roster order where the node runner starts them side by side), `facts` (code facts, import and source scans, history facts; the scans' regular expressions run on a small JavaScript-semantics engine, `js_regex.rs`), `record` (the verdicts weighed against fan-out's pre-verdicts and the refute round, the report and the summary, the ledger lines, and a feature review's gate and a ticket review's record), `refute` (the refute round: each accepted critical finding goes to the first routed client, base before deep, whose family neither raised it nor is the orchestrator's, through the agy, Codex, Claude, Pi or Vibe client; the answer counts only with evidence the frozen packet holds, and each refuter's launch is logged in both ledgers; a Vibe refuter searches through the runner's own search server as a Vibe critic does, and the refuters of a round run one after another where the node runner starts them side by side), `reviews`, `state` (one feature's flow state computed from the ledger, run directories, review and gate indexes and the tracker, cached per flow, with its single next step), `stats` (both ledgers merged as the node runner merges them, figures, calibration, removal candidates and the rendered report) `triage` (the rules, the gates and the triage model's question; a `gh` port and the tracker, which `check-tickets` reuses) and `setup` (`/flow setup`'s machine checklist: `list`, `check`, `quota` and `kill-switch`; the hooks-installed check reads Claude Code's `settings.json` and Codex's `hooks.json` and `config.toml`, and computes the hash Codex trusts each hook by; the kill-switch check asks the gate itself; the sign-in and version reads and the trivial probe go through the critic clients, and each launch is one invocation line on the ledger; a Vibe client's read-only proof runs the search-reach proof and then one live prompt through `run_typescript`. The proof's inputs hash the bundled `critique.mjs`, as node does, so a result recorded by either runner is current for the other). Every command of the node runner is ported. `search-server` is ported: the MCP stdio
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
  (`tests/fixtures/verdicts.json`, frozen from the retired node runner's own functions).
* **Search wiring.** A Vibe critic of `fan-out` or `refute` searches through the runner's own `search-server`
  (`search_client.rs`, `critic_run.rs`'s `CriticTools`, `vibe_search.rs`), as node's `CriticTools` does: the installed
  agent file and the home's `config.toml` must let it (`searchBlocked`), the server must pass the reach proof (made once
  per command, in `<run>/search-proof`: a packet with one file, a file outside it and two symlinks out of it, then the
  nine calls that try every way out), and the server must start on the critic's own copy; otherwise it reads alone and
  its run says why (`search_failure`). The server is stopped with `pkill -f` once the critic ends. The server command is
  the Rust runner's own executable (`Runtime.search`, set from `current_exe()`); a test runtime leaves it unset, so a
  Vibe critic reads alone there. The goldens came from the retired node runner, whose server command was `node critique.mjs search-server ...`, so
  the command line in the MCP config of a Vibe launch with search differs, and the goldens cannot cover such a launch.
  They cover the launches that read alone, and the unit tests and `tests/binary.rs` (the real binary serving the reach
  proof against a real packet) cover the rest.
* **Pi critics.** A routing client of kind `pi` (`provider` `litellm-<route>`, `model`, `family`, `binary` defaulting to
  `pi`) runs a critic or refuter through Pi on a LiteLLM route (`pi_client.rs`, node's `adapters/pi/index.ts`). It is
  not started through the gateway, which has no Pi client: `pi -p --mode json --no-session --offline
  --provider <provider> --model <model> --tools read,grep,find,ls` with extensions, skills, prompt templates, themes,
  context files, MCP and trust-gated project files switched off and the runner's own system prompt, in the critic's
  packet copy with the prompt on stdin, under the slot's time limit as the outer timer. Pi's read tools run inside the
  Pi process, so the whole process runs confined (`agent-launcher confine`, above): `agent-launcher confine --read
  <packet copy> --run-from <binary> [--run ~/.pi/agent/bin] --write <critic>/pi-agent --net-connect <proxy port> --
  pi ...`, with `PI_CODING_AGENT_DIR` and `TMPDIR` set to `<critic>/pi-agent` and the home's Pi `bin` (Pi's `fd`) first
  on `PATH`. The launcher is `AGENT_KIT_LAUNCHER_BIN`, else `agent-launcher` on `PATH` (setup installs it as
  `~/.local/bin/agent-launcher`, `scripts/install-critique-rust.sh`). `<critic>/pi-agent` is made for the one launch
  (mode 0700): a `settings.json` of `{}` and a `models.json` (mode 0600) holding only the route's provider from the
  home's `models.json` (`PI_CODING_AGENT_DIR`, else `~/.pi/agent`), its `baseUrl`, `api`, the routed model's entry and
  the key as a literal, read just before the launch by running the home's `!<program> get <NAME>` without a shell (a
  literal key is taken as it is; any other command is refused). The directory is removed when Pi ends. So the critic
  can read nothing secret but that route's own budgeted key; a hostile model could have Pi send that key back to the
  proxy, and nothing else. A Pi critic that cannot be confined never starts: readiness (prepare, refute) also needs the
  home's provider with an `http(s)` `baseUrl`, the launcher and a passing `agent-launcher confine --check`, else the
  client is unavailable with the reason; at launch, a missing route, launcher or key, or the launcher's exit 125, fails
  the critic as `unconfined`. The JSON event stream is still checked afterwards as a second layer: another tool, or a
  `path`, `glob` or find `pattern` that reaches past the packet (absolute, `~`, or a `..` that climbs out, resolved
  lexically), voids the review as `unsafe-tool`; a last assistant message that stopped on an error or was aborted fails
  it although Pi exits 0. Readiness also reads `pi auth check --provider <provider> --json` and `pi --offline
  --list-models <model>`, neither of which calls a model, and the ledger lines carry kind `pi`, charged Pi's own
  `usage.cost.total`. The runner's search server is not attached: Pi takes MCP servers only from its config files, and
  its own `grep`, `find` and `ls` search the packet. `fan-out-pi`, `refute-pi` and `setup-pi` replay the launches byte
  for byte through a fake `agent-launcher` and secret store: the launcher keeps its
  grants beside the packet (`launcher.txt`) and the fake `pi` its argv, stdin and what its agent directory held.
* **Setup goldens.** The setup scenarios cover every setup action with fake `vibe` and `agy` programs in the sandbox's `bin/`. A Vibe client's read-only proof is not among them, for the reason the search wiring's goldens are not: its launch carries the search server's command line. The harness reads the hash of the bundled runner as `@RUNNER@`, since it changes with every build.
* **Goldens.** Frozen fixtures under `tests/fixtures/`: scenario inputs and the stdout, stderr,
  exit code and every file left under a temporary sandbox (fixed clock, fixed `HOME`, git
  repositories made in the sandbox). They were generated once by the node runner, which is
  retired (#808) with its generator scripts. The unit tests replay each scenario through the
  library and compare byte for byte:

```bash
cargo test --manifest-path scripts/Cargo.toml -p critique    # SCENARIO=<name> replays one scenario
```

### Swapping the critique runner in

The skills and the three critique hooks call the critique skill's launcher,
`skills/workstation/critique/critique` (`sh <critique dir>/critique <command> ...`),
not a binary directly. The launcher runs the Rust binary at
`~/.local/share/agent-kit/critique/critique` (`AGENT_KIT_CRITIQUE_BIN` names another)
with `AGENT_KIT_CRITIQUE_ASSETS` set to the skill directory, so it reads that
directory's prompts, roles, lenses, schemas and `routing.json`; without the binary
it says so and exits 127 (the node runner is retired, #808). The Rust runner
answers every command itself.

Setup runs `scripts/install-critique-rust.sh`, which builds the crate
(`cargo build --release --locked -p critique`), checks that the new binary answers
`--help`, and moves it into place atomically. No cargo, no workspace or a cloud session
skips the step, and a failed build leaves whatever was installed untouched.

`tests/test_critique_rust_swap.py` covers the launcher and the installer with stub
programs; `tests/binary.rs` runs the real binary.

## Rust migration guard

agent-kit is going 100 percent Rust (#802). `crates/rust-migration-guard`
(#804) is the CI job `Rust Migration Guard`, needed by `Pipeline Complete`, that
keeps Python and TypeScript from growing while the children of #802 remove them.

```bash
rust-migration-guard [--repo-dir DIR] [--allowlist PATH] --event-path FILE
rust-migration-guard [--repo-dir DIR] [--allowlist PATH] --base REF --body-file FILE
```

CI passes `$GITHUB_EVENT_PATH` (base is `pull_request.base.sha`, body is
`pull_request.body`; a push to main has no pull request and passes). The second
form is the local mode: run it from a branch against `origin/main` and a file
holding the body you intend to use. It needs only git and the checkout.

It watches `.py .pyi .ts .tsx .mts .cts .js .mjs .cjs` files in
`git diff -M BASE...HEAD`, except under `node_modules`, `target`, `.venv`,
`venv`, `__pycache__` or `dist-packages` (vendored or built output; no tracked
file lives there). Fixture and golden directories are not excluded: a script
there is still a script. The allowlist is `ci/rust-migration-allowlist.txt`,
one path per line, grouped under a comment naming the child ticket that removes
the files; it starts with every such file on main.

| Change | Result |
| --- | --- |
| a watched file is added, or renamed to a new path, off the allowlist | fails |
| a watched file off the allowlist is modified | fails |
| an allowlisted file is modified, or renamed onto an allowlisted path | fails without a debt line |
| a path is added to the allowlist | fails without a debt line |
| a file is deleted or an allowlist line is removed | passes |

The debt line is a line of its own in the pull request body:

```text
rust-migration-debt: #810 one-line fix for the converge crash until the port lands
```

It needs a `#<child ticket>` and a non-empty reason. The guard reads the body
from the event payload of the run, so after editing the body push a commit (a
re-run of the old run still carries the old body). Delete the allowlist line in
the same pull request that deletes a file; never regenerate the list to make a
new file pass.

## ci-helpers

`crates/ci-helpers` is the CI helpers that were four Python scripts until #813
(parent #802), one binary with one subcommand each. It reads YAML with
`serde_yaml` and JSON with `serde_json`, both already in the lockfile; the
SHA-256 for the checksum check is written in the crate, so no hashing crate
comes with it.

| Subcommand | Was | What it does |
| --- | --- | --- |
| `agents-md [--require] [REPO_ROOT ...]` | `scripts/check_agents_md.py` | Fails when a path, link or `path::symbol` an AGENTS.md names no longer exists. Problems go to stderr as `<root>/AGENTS.md:<line>: <message>`; exit 0 clean, 1 problems, 2 a root that is not a directory. |
| `marketplace-bump (--base-ref REF \| --base-registry FILE) [--registry FILE] [--git-base URL]` | `scripts/check_marketplace_bump.py` | Reads each plugin the registry installs from a bumped marketplace (and each bumped skill source) at the old and the new commit; a hook, MCP or LSP server, script or shell line only the new commit has fails the run. Upstream code is read, never run. Appends the report to `$GITHUB_STEP_SUMMARY` when it is set. |
| `skill-sources [--root DIR] [--validate \| --update SKILL \| --sync-issue] [--issue-title T]` | `scripts/check_skill_sources.py` | Checks each estate skill's `sources.yaml` against GitHub through `gh`. A failed request is a failure and never drift; `--sync-issue` keeps one open drift issue and leaves it untouched after a failed run. |
| `litellm-tiers (--write [--ref REF] \| --check [--upstream REF]) [--path FILE]` | (new, #905) | Snapshots litellm's `tier_groups` (`catalog/routing.json`) into `registry/litellm-tiers.json` through `gh api`, and checks the snapshot's shape and, with `--upstream`, its drift from litellm. `render_registry.py` renders the tier profiles, keys and Pi providers from the snapshot. |
| `verify-release-asset stamp-version TAG \| check-version TAG \| check-checksum TARBALL SHA256_FILE` | `scripts/verify_release_asset.py` | Stamps the release tag into setup-workstation.sh and checks a published asset against it and against its checksum. |

What changed against the Python, and what did not:

* The messages, exit codes and report lines are the ones the pytest suite
  pinned; those tests are ported to the crate (`src/tests/`), with real `git`
  repositories for the marketplace review and a stand-in `gh` (a trait in the
  unit tests, a shell script for the subprocess runner) for the drift check.
* `--update` stamps today's date in UTC (the Python used the local date) and
  writes the file with `serde_yaml`, which prints the same block style
  PyYAML did.
* The drift issue's body points at `ci-helpers skill-sources --update <skill>`.

Callers: the `rust-workspace` job in `.github/workflows/ci.yml` runs
`skill-sources --validate` and `agents-md`; the `marketplace-pins` job runs
`marketplace-bump`; `.github/workflows/skill-sources.yml` runs
`skill-sources --sync-issue`; `scripts/publish-installer-artifacts.sh` runs
`verify-release-asset` through `$CI_HELPERS` (a built binary, which the
publish workflow builds) or, without it, `cargo run -p ci-helpers`. Cargo
resolves through the packages proxy in every one of them.

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
Keep test helpers and stubs in those files, never in a production file. It needs a rustup toolchain with the `llvm-tools` component (so
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

## Resolving crates

Cargo resolves through the packages proxy (`packages.jorisjonkers.dev/cargo`)
wherever setup or `setup-package-proxy` configured `~/.cargo/config.toml`, with
the token in `~/.cargo/credentials.toml`. The proxy's own release cooldown is
the proxy's concern and is unchanged. The repository itself sets no cargo
cooldown, runs no crate-age check, and has no pre-commit hook or waiver file for
one: the crate cooldown (standards ADR-0014) was removed from agent-kit by owner
decision.

The loopback crate-gate (`127.0.0.1:17342`) that once replaced crates.io here is
long retired. Stop a gate a machine still runs with
`systemctl --user disable --now crate-gate` (launchd on the Mac) and delete
`~/.config/systemd/user/crate-gate.service`.

## The pull-request workflow

The `Rust Workspace` job in `.github/workflows/ci.yml` runs on every pull
request and push to `main`, and `Pipeline Complete` needs it. With cargo
resolving through the packages proxy it builds the workspace, tests it, runs the
Gold Band port inventory check, validates the reach-level record and runs
`scripts/coverage.sh`, the 100 percent line-coverage gate.

## The Gold Band workflow lint

`crates/workflow-lint` is a library plus a CLI (`workflow-lint [--shape] --instance gold-band|gold-band-personal <FILE>...`;
exit 0 clean, 1 problems, 2 bad usage or an unreadable or invalid file) that
later tickets call. Its one registry dependency, `serde_json`, is pinned
exactly. It reads Gold Band's workflow
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
`--shape` adds the `shape` rule group, Gold Band's editor validation ported at commit
987fc3e2 ([GOLD-BAND.md](GOLD-BAND.md), "How the workflows are validated").
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

## research-layout

`crates/research-layout` checks and indexes the `papers/` directory of the
private research repository (#356) and runs Workflow A's two deterministic
steps (#456); what the workflow does with them is in
[GOLD-BAND.md](GOLD-BAND.md#workflow-a-in-the-personal-instance).

```bash
research-layout check <PAPERS_DIR>
research-layout index <PAPERS_DIR>
research-layout fetch --plan FILE --out DIR [--max-sources N] [--max-bytes N] \
    [--timeout-secs N] [--curl PROG]
research-layout adopt --plan FILE --fetched DIR --papers DIR
research-layout verify --papers DIR --fetched DIR [--base REF]
```

* **`check`** refuses an unlisted file, a listed file that is missing, a theme
  folder without its `README.md`, a stale `INDEX.md`, a top-level name outside
  the layout, a broken catalog entry or tier, a note citing an unknown id or an
  entry below its `min_tier`, a quote not found in its stored page, and any
  path that leaves the papers directory, each as `research-layout: <path>:
  <rule>` on stderr. **`index`** rewrites every `INDEX.md` from `catalog.json`;
  a second run changes nothing.
* **`fetch`** reads a plan (`{"sources": [{"id", "url"}, ...]}`) and refuses it
  whole, fetching nothing, when it lists more than `--max-sources` sources
  (default 8, at most 32), an id that is not `[a-z0-9][a-z0-9-]*`, a duplicate
  id, or a url that is not https on a public host name (no IP literal,
  `localhost`, `.local`, `.internal`, dotless host or user info). Each source is
  fetched with `curl` (https only, also on redirects, at most five,
  `--max-time` and `--max-filesize` from the flags), its text extracted (tags,
  scripts, styles and comments dropped, block tags as line breaks, entities
  decoded, whitespace collapsed per line) and stored as `<out>/<id>.txt`.
  `<out>/fetch-log.json` records each source with `file`, `sha256` and `bytes`,
  or its `error`; a failed source does not stop the others. Exit 0 when at
  least one page is stored, 1 when the plan is refused or none was, 2 for an
  unreadable plan or output directory.
* **`adopt`** copies every page the fetch log lists as stored, byte for byte,
  into the plan's `theme` folder (one or two `[a-z0-9-]` parts) as
  `<theme>/<id>.txt`. A file already there with the same bytes counts as
  copied; one with other bytes is refused (`exists`), never overwritten. Exit 0,
  1 for a bad theme, a bad log entry, a missing page or a refused file, 2 for an
  unreadable plan or log.
* **`verify`** runs `check` and adds: `<id>: page-not-as-fetched` for an entry
  whose `url` the log fetched but none of whose files is the stored page byte
  for byte; for each note under `notes/` that differs from `--base` (default
  `origin/main`) or is untracked, `no-quote`, `quote-too-short` (under 8
  characters) and `quote-page-not-fetched: <page>` (the page is not one the
  fetch stored); and `notes: no-run-note` when the run added no note. It only
  reads; git is asked for the changed notes. Exit 0 ok, 1 a rule failed, 2 an
  unreadable fetch log or a git failure.

The unit tests drive `fetch` through a stub `curl` script and `verify` through
the real `git` over a fixture repository; they cover every line except
`src/main.rs`.

## pr-composer

`crates/pr-composer` holds the two helpers of the pr-composer skill, ported from
`skills/workstation/pr-composer/scripts/` (#813). Setup installs it on PATH
(`workspace_tools`), and the skill calls it by name:

```bash
pr-composer check-body -t TEMPLATE -b BODY [--skipped "Screenshots,Migration"]
pr-composer mine-prs [--out PROFILE.json] [--md] DUMP.json
```

* **`check-body`** compares a drafted PR body with the repository's template:
  invented and missing sections, changed heading levels, a changed section
  order, dropped or truncated checklists (all failures), unfilled `«…»`
  placeholders and leftover HTML comments (warnings). Exit 0 clean, 1 with the
  violations printed, 2 for a bad command line or an unreadable file. The text
  is the Python helper's, byte for byte.
* **`mine-prs`** turns a `gh pr list --state merged --json ...` dump into a
  house-style profile: titles, bodies, people, labels and size. It writes the
  profile JSON (`--out`, default `<dump>.style.json`, the same layout
  `json.dump(indent=2)` wrote) and with `--md` the summary beside it.
* **No regex crate.** The lockfile carries none, and the helpers' behaviour on
  odd input follows Python's backtracking (a heading whose `\s+` runs across
  lines, a lazy `.+?`), so `src/regex.rs` is a small backtracking matcher for the
  subset of Python `re` the helpers use, and the patterns are ported as written.
  The only dependency is `serde_json`, already pinned for the workspace.
* **Checked against Python.** `tests/fixtures/expected/*.txt` are what the
  Python helper printed for the fixtures. Before the scripts were deleted, 600
  random template and body pairs and 400 random PR dumps gave the same output
  from both, except where Python's set iteration order broke a tie (the order
  of equally frequent reviewers or sections, which was arbitrary there and is
  first-seen here).

## estate

`crates/estate` is one binary, `estate`, with the four mechanical helpers of the
estate skills (#813). Each replaces a Python script that sat in its skill's
`scripts/` directory; the skill now calls `estate <subcommand>` on PATH, where
setup installs it (`workspace_tools`, so it also ships in the release
tarball). It takes `serde_json` only.

```bash
estate audit REPO_DIR [--latest-sha SHA] [--json]
estate verdict [--signals FILE]
estate intent scaffold --project NAME --owner OWNER --host HOST --audience AUDIENCE \
    --image IMAGE --process PROCESS --port N --out DIR
estate intent validate PATH [EXTRA_PATH ...]
estate plan --project P --action pause|rollback|resume [--target V] [--reason R] \
    [--pin V] [--proof-names V | --no-proof]
```

* `audit` (estate-bootstrap) is the offline check of a repository: workflow pins,
  the one `Pipeline Complete` job, the renovate preset and retired references.
  Output is one `<file>: <gap> (source: <doc>)` line per gap, or `no gaps`;
  `--json` prints the list. Exit 0 when it ran, 2 for a missing directory or a
  `--latest-sha` that is not 40 lowercase hex characters.
* `verdict` (estate-delivery-status) reads the five signals as JSON and prints
  `VERDICT: landed|mixed|not landed`, then a line per signal. Exit 0 for any
  well-formed input, 2 for malformed input.
* `intent` (estate-project-intent): `scaffold` writes a Project Intent and its
  `base.env` without overwriting; `validate` runs `deploy-kit validate` (limit
  300 seconds) and mirrors its exit status, 2 when deploy-kit is missing.
* `plan` (estate-rollback) prints the plan and the exact `gh workflow run`
  command. Its one `gh` call is a GET of the pin file; it never dispatches.
  Exit 0 plan printed, 1 refused.

Differences from the scripts: messages name `estate <subcommand>` instead of
`<script>.py`, the tool finds `gh` and `deploy-kit` on PATH itself, and the
output of a tool it runs is passed through after the tool finishes, stdout then
stderr. Every other line of output is the script's. The crate tests carry the
scripts' pytest cases and also run the refused and landed worked examples
printed in the skills' SKILL.md.

## projects-layout-guard

`crates/projects-layout-guard` is the `PreToolUse` hook of
[ADR-0009](adr/0009-projects-layout-guard.md) (#813, part of #802), ported from
`hooks/projects-layout-guard.py` with the same decisions and the same deny JSON.
It is std-only apart from `serde_json`.

* **Shape.** The payload comes on stdin. The guard tokenises a Bash command the
  way Python's `shlex` did (a `#` starts a comment, a newline is whitespace),
  follows `cd`, `bash -c`, `env` style wrappers and git's global options, and
  denies a `git clone` or `gh repo clone` of a GitHub repository whose target is
  not `~/Projects/<owner>/<repo>`, a `git worktree add` outside
  `~/Projects/<owner>/worktrees/<repo>/`, and `EnterWorktree` with `name`.
* **Fail open.** An unparsable payload or command, an unknown repository, a
  `git` that cannot run or takes over two seconds, or any other error of its own
  allows the call. `PROJECTS_LAYOUT_OFF=1` or a machine without `~/Projects`
  makes it a no-op. `~user` is not expanded, and an unset `HOME` allows the call.
* **Install.** It is a `workspace_tools` entry, so setup puts it in
  `~/.local/bin` and the release tarball carries it. The skill's
  `hooks/projects-layout-guard.sh`, the file the agents' settings name, runs it
  (`WORKSPACE_TOOLS_BIN_DIR` moves the directory) and allows the call while it
  is not installed.
* **Tests.** The pytest cases became Rust tests in `src/tests.rs`: decisions
  against a temporary `~/Projects` with real `git init` repositories, and a fake
  `git` for the timeout.

## Release binaries

Every release carries prebuilt, statically linked builds of the command-line
tools setup installs from source today (#806, part of #802). They are the first
step of the bootstrap that replaces `cargo build` on a clean machine; the
bootstrap shell and NixOS packaging that consume them are described under
"Bootstrap" below (#807). There is no single `agent-kit` binary yet; the multicall crate
is a later child of #802, and the tarball will carry it when it exists.

* **What ships.** The `workspace_tools` of `registry/estate-tooling.yaml`
  (agent-launcher, gold-band-gate, gold-band-run-start, gold-band-findings,
  gold-band-proof, research-layout, ticket-publish, workflow-lint,
  coverage-gate, ci-wait, litellm-gate, projects-layout-guard, estate, pr-composer, agent-kit-merge) plus critique, model-gateway and
  build-tracker. The list is `TOOLS` in `scripts/release-binaries.sh`.
* **Targets.** `x86_64-unknown-linux-musl` (hosted `ubuntu-latest`),
  `aarch64-unknown-linux-musl` (the native `ubuntu-24.04-arm` runner) and
  `aarch64-apple-darwin` (`macos-latest`). Each is built natively on a runner of
  its own architecture, so no cross linker is needed: the workspace has no C
  dependency, and the musl targets link statically with the toolchain's own C
  runtime. A Linux binary that is not statically linked fails the build.
* **Assets.** Per target `agent-kit-tools-<tag>-<target>.tar.gz` (a directory of
  the same name holding `bin/<tool>`, `VERSION` with the tag and target, and a
  `SHA256SUMS` of its files) and `agent-kit-tools-<tag>-<target>.tar.gz.sha256`
  as the runtime-home package has; plus one release-wide `SHA256SUMS` over the
  tarballs (`sha256sum -c` format). Verify with
  `grep ' agent-kit-tools-<tag>-<target>.tar.gz$' SHA256SUMS | sha256sum -c -`
  (`shasum -a 256 -c -` on macOS).
* **The trigger.** The `Release` workflow builds only after release-please
  creates a release. The builds are `--locked`, from the repository and through
  the packages proxy.
* **The smoke test.** After the assets are attached, a job per target on a
  runner of that target downloads them from the release, verifies them against
  `SHA256SUMS`, and runs every binary with `--help`. The tools have no common
  `--version`, so the check is that each starts and answers (exit 0 to 2 with
  output); a missing loader, a signal or silence fails it.
* **Dry run.** Run the `Release` workflow by hand (Actions, Run workflow, or
  `gh workflow run release.yml --ref <branch>`). release-please is skipped, the
  tag is `dry-run-<sha7>`, the same build and smoke jobs run, and the
  tarballs and `SHA256SUMS` are uploaded to the run as the `release-assets`
  artifact; nothing is attached to a release. The same three commands run
  locally: `scripts/release-binaries.sh build TAG TARGET dist`, `sums dist`,
  `smoke TAG TARGET dist`.
* **Limits.** The workflow cannot be proven on a real release without cutting
  one; the dry run covers everything except the `gh release upload` and
  `gh release download` steps. The aarch64 Linux build assumes the
  `setup-package-proxy` action works on the arm runner.

## Bootstrap

A machine with no Python, Node or cargo installs from the release with
`scripts/bootstrap.sh` (#807, part of #802), POSIX sh needing curl or wget,
tar and sha256sum or shasum:

```bash
curl -fsSL https://github.com/JorisJonkers-dev/agent-kit/releases/latest/download/bootstrap.sh | sh -s -- [setup options]
```

* **What it does.** Picks the target from `uname`, downloads
  `agent-kit-tools-<tag>-<target>.tar.gz`, checks its sha256, unpacks it, checks
  the tarball's own `SHA256SUMS` and `VERSION`, installs every tool atomically,
  then downloads the same release's `setup-workstation.sh` and runs it with the
  arguments it was given. A failed check installs nothing.
* **Where tools go.** `~/.local/bin` (`AGENT_KIT_BIN_DIR`), except `critique` and
  `model-gateway`, which go where their launchers look
  (`~/.local/share/agent-kit/<tool>/`). A pin file
  (`~/.config/agent-kit/critique-node`, `model-gateway-node`,
  `build-tracker-node`) keeps the node runner, as for the source installers.
  Without cargo setup's own installers skip; with cargo they may replace these
  with a local build.
* **Stamping.** The release attaches `bootstrap.sh` with its tag and the
  release-wide `SHA256SUMS` lines stamped in (`scripts/release-binaries.sh stamp`,
  run by the `checksums` job), so the tarball is checked against a value that
  travelled with the script. The unstamped copy in the repository fetches
  `SHA256SUMS` from the release, which only HTTPS secures. `setup-workstation.sh`
  is fetched over HTTPS and is not covered by the stamp.
* **Overrides.** `AGENT_KIT_TAG`, `AGENT_KIT_RELEASE_BASE` (a directory of the
  assets, `file://` works), `AGENT_KIT_SKIP_SETUP=1`.
* **NixOS.** `nix/agent-kit-tools.nix` packages the same tarball:
  `pkgs.callPackage ./agent-kit-tools.nix { tag = "v1.2.3"; sums = builtins.readFile ./SHA256SUMS; }`
  with the release's `SHA256SUMS`. The Linux binaries are static, so nothing is
  patched.
* **Developer fallback.** `cargo build` from a checkout, as before.
* **Test.** `scripts/test-bootstrap.sh` (CI, Shell Lint job) runs the bootstrap
  against a fake release over `file://`: install paths, pins, stamping, a
  tampered tarball, a bad tag, a PATH with no Python, Node or cargo, and the
  derivation's evaluation when nix is present. It needs no network.

## agent-kit-merge

`crates/agent-kit-merge` is the config-and-merge library (#809) and the
`agent-kit-merge` binary over it. It replaces the `*-merge-mcp`,
`package-proxy-merge`, `instruction-block` and `codex-add-url-mcp` Python
scripts; the later converge port (#810) reuses the library. It takes
`toml_edit` (the format-preserving TOML editor `tomlkit` was), `serde_yaml` and
`regex`, each pinned exactly; the JSON it needs (ordered, written as Python's
`json.dumps(indent=2)` writes it) is in the crate.

```bash
agent-kit-merge antigravity-mcp FRAGMENT [--config PATH] [--remove NAME ...]
agent-kit-merge opencode-mcp    FRAGMENT [--config PATH] [--remove NAME ...]
agent-kit-merge hermes-mcp      FRAGMENT [--config PATH] [--remove NAME ...]
agent-kit-merge vibe-mcp        FRAGMENT [--key mcp_servers|hooks] [--config PATH] [--remove NAME ...]
agent-kit-merge codex-add-url   CONFIG NAME URL
agent-kit-merge instruction-block FILE NAME [--remove]     # body on stdin
agent-kit-merge package-proxy   FILE NAME [--remove] [--validate toml|ini] [--refuse REGEX] [--mode MODE]
agent-kit-merge converge mcp    --keep PATH --claude-home DIR [--claude DIR ...] [--codex PATH] [--vibe PATH] \
                                [--vibe-private PATH] [--opencode PATH] [--declared SURFACE=NAME,... ...] \
                                [--retired NAME,...] [--private NAME,...] [--check]
agent-kit-merge converge plugins --keep PATH [--declared-markets NAME,...] [--declared-plugins ID,...] \
                                [--retired-markets NAME,...] [--retired-plugins ID,...] [--check]
agent-kit-merge converge gold-band-roles --keep PATH --registry PATH [--profiles DIR] [--critique DIR] \
                                [--closing PATH] [--check]
agent-kit-merge converge gold-band-workflows --keep PATH --registry PATH [--store PATH] [--workflows DIR] \
                                [--check]
```

`converge mcp` and `converge plugins` are the first slice of the converge port
(#810): they replace `scripts/converge.py mcp` and `plugins`, with the same
output lines and exit codes, and run `claude` and `codex` the way the script
did. One layout difference: removing a Vibe server keeps the remaining
`[[mcp_servers]]` where they were, where `tomlkit` moved them to the end of the
file. `converge gold-band-roles` is the second slice: the personal instance's
`context/profiles`, rendered byte for byte as the script did (the critique
sources concatenated verbatim, the fixed front matter stamp), with the same
keep, retired and built-in rules, exit codes and running-instance guard
(`AGENT_KIT_GOLD_BAND_RUNNING`, else the process table: `/proc`, or `ps` on
macOS). `--registry` is required; the critique directory and closing file
default to the kit the registry sits in. `converge gold-band-workflows` is the third: the personal instance's
`context/workflows.json`, with the same add, in-place change, keep, work-agent
skip, built-in refusal, null-insensitive comparison, atomic write with one
`.bak`, and the same output. The editor's shape rules and the critique rules
run in process through the `workflow-lint` library, so `scripts/gold_band_shape.py`
and its parity test are gone, and the critique rules are always checked (the
script skipped them with a warning when no `workflow-lint` binary was built).
`gold-band` is still `scripts/converge.py`.

Behaviour, exit codes and output lines are those of the scripts. JSON and TOML
merges write the bytes the Python tools wrote, so a re-run over a file they
wrote changes nothing; the Hermes YAML merge keeps the same keys and values but
`serde_yaml` lays the file out its own way (no 80-column wrapping of long
strings, a `yes` key stays a string). Setup runs the tool through the shell
function `agent_kit_merge`: `AGENT_KIT_MERGE` when set, else
`agent-kit-merge` in the setup bin directory or `~/.local/bin`, else the kit's
own release build, made once on first use (the package proxy step runs before
the workspace tools are installed). The Python tests build it into
`scripts/target` through `tests/setup_harness.py`.
