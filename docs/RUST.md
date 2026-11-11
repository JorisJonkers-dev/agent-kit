# The Rust workspace and the 7-day crate cooldown

The estate's Rust lives in **one cargo workspace**, `scripts/Cargo.toml`, with
**one lockfile**, `scripts/Cargo.lock`, and **no git or path dependency**
outside the workspace. The rule and the 7-day crate cooldown come from the
standards repo (`JorisJonkers-dev/standards`, ADR-0014, whose `scripts/README.md`
documents its tooling); this page is how agent-kit follows it, on the Mac and
on the Linux workstation.

* **The cooldown.** No crate version younger than **7 days** may be used.
  Cargo resolves through **crate-gate**, a loopback mirror of the crates.io
  index at `127.0.0.1:17342` that hides every too-young version, so a normal
  `cargo build` cannot pick one up even by accident. **crate-age** is the
  check for after the fact: it exits `1` for a version proven younger than 7
  days, `2` for a version whose age cannot be proven, and `0` only when every
  locked version is proven old enough. The gate **fails closed**: an unprovable
  age blocks the change.
* **The workspace.** Fourteen members. `crates/agent-kit-setup` is the canary: a
  std-only crate (nothing in the lockfile has an age to question) whose
  `--version` proves the build and whose `validate <PATH>` checks a path.
  `crates/inventory-check` fails when an on-disk skill, hook, role file or MCP
  server is missing from docs/gold-band-port-inventory.md, has two targets or no
  lanes. `crates/gold-band-registry` is std-only too and validates the Gold Band
  instances and the overlay (docs/REGISTRY.md). `crates/code-nav-eval` runs the
  frozen code-navigation question set. `crates/code-nav` is the read-only
  code-navigation MCP server (below). `crates/workflow-lint` is the Gold Band
  workflow lint (below). `crates/ticket-sync` is the ticket sync tool (below);
  it takes serde, serde_json and serde_yaml, each pinned exactly.
  `crates/agent-launcher` is the reach-level launcher (below); it takes serde
  and serde_yaml, each pinned exactly.
  `crates/build-tracker` is the Rust port of the build tracker (below); it is
  std-only, with the ordered JSON its state file needs written in the crate.
  `crates/litellm-gate` is the node-side check that a LiteLLM per-key budget is
  not exhausted: one probe request through curl with the key on stdin, exit 1
  when the budget is spent or the proxy cannot be asked.
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
  gold-band-report. New Rust joins this
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
  first root.
* **Tool `search`** (`query`, optional `path` and `limit`). A line matches
  when it contains the query, ignoring case. **Rank:** score 1, plus 4 when the
  query is a whole identifier on the line, plus 2 when the file name contains
  it; highest first, ties to fewer path components, then path, then line.
  **Cap:** the default is 20 (`--limit` changes it), a call may ask for 1 to
  200, and a capped result says how many matches it left out. The walk skips
  symlinks, `.git`, `node_modules`, `target`, `.venv`, files over 1 MiB and
  anything that is not UTF-8 text.

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
close. Closing needs `--apply --approve-close` together. Only graph issues are
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
  `unverified` or `unsupported`. A cell is verified separately on each OS; a
  level proven on the Mac is never assumed on Linux.
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
exits with its status. The operating-system write limit (ADR-0013) is a
separate interface that plugs in beside it.

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
workspace workflow, after `rustup component add llvm-tools`.

## Cargo resolves through the gate

`.cargo/config.toml` at the repository root replaces crates.io with the gate
for every cargo command run in the repository, whatever the user's own
`~/.cargo/config.toml` says (the nearest config wins):

```toml
[source.crates-io]
replace-with = "crate-gate"

[source.crate-gate]
registry = "sparse+http://127.0.0.1:17342/"
```

* With the `[source.crate-gate]` table removed, cargo refuses to resolve
  (`could not find a configured source with the name crate-gate`); it does not
  fall back to crates.io.
* With the gate down, cargo fails with connection refused; it never reaches
  crates.io.
* With both present, the build resolves only through the gate.

`scripts/gate/verify-gate.sh` proves all three on the machine it runs on, with
a throwaway crate and a private `CARGO_HOME`. Run it after installing the gate.

### Linux workstation

The gate runs as a **systemd user service**. One command installs it:

```bash
scripts/gate/install-linux.sh
```

It reads the crate-gate and crate-age sources from your standards clone
(`~/Projects/JorisJonkers-dev/standards`, or `STANDARDS_DIR`) with `git archive`
at the pinned commit `STANDARDS_REF` — the clone's working tree is never
touched — builds both (borrowing cargo from `nix shell` on NixOS when none is on
`PATH`), installs `crate-gate` and `crate-age` to `~/.local/bin`, writes
`~/.config/systemd/user/crate-gate.service` (127.0.0.1:17342, floor 7 days,
`Restart=always`) and starts it. Check it with:

```bash
curl -s http://127.0.0.1:17342/health
systemctl --user status crate-gate
scripts/gate/verify-gate.sh
```

Building the gate needs a registry once, before the gate exists; that is the
packages proxy block setup already writes to `~/.cargo/config.toml`. The gate
is not part of `installer/setup-workstation.sh`: it is installed once per
machine, by hand or by the machine's own configuration.

### Mac

The Mac runs the same binary under launchd, installed by the standards repo's
`scripts/gate/install-macos.sh`, listening on the same `127.0.0.1:17342`. This
repository's `.cargo/config.toml` needs nothing more there.

## The pre-commit hook

`.githooks/pre-commit` runs **crate-age** over any staged `Cargo.toml` or
`Cargo.lock` change. Enable it once per clone:

```bash
git config core.hooksPath .githooks
```

(plus `chmod +x .githooks/pre-commit` if the checkout lost the executable
bit). How the hook obtains crate-age — it never copies it in:

1. `$CRATE_AGE` — a full path to the crate-age executable;
2. `crate-age` on `PATH` — what `scripts/gate/install-linux.sh` installs;
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
must be installed on that repository), builds crate-gate and crate-age, runs
crate-age over the checkout, starts the gate on the runner and builds and tests
the workspace with cargo resolving only through it. A too-young or unprovable
version fails the job, so the same change a contributor cannot commit cannot
merge either. Move the pinned commit here and in `scripts/gate/install-linux.sh`
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
