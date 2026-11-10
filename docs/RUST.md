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
* **The workspace.** Five members. `crates/agent-kit-setup` is the canary: a
  std-only crate (nothing in the lockfile has an age to question) whose
  `--version` proves the build and whose `validate <PATH>` checks a path.
  `crates/inventory-check` fails when an on-disk skill, hook, role file or MCP
  server is missing from docs/gold-band-port-inventory.md, has two targets or no
  lanes. `crates/gold-band-registry` is std-only too and validates the Gold Band
  instances and the overlay (docs/REGISTRY.md). `crates/code-nav-eval` runs the
  frozen code-navigation question set. `crates/workflow-lint` is the Gold Band
  workflow lint (below). New Rust joins this workspace as a new member, never as
  its own Cargo project.

```bash
cargo build --manifest-path scripts/Cargo.toml
cargo test --manifest-path scripts/Cargo.toml
```

The workspace aims at **100 percent line coverage** of every crate: the unit
tests in `src/lib.rs` execute each of its lines, with one exception that each
crate names in a comment beside the code, the single executable line in
`src/main.rs` (the exit wrapper).

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

`crates/workflow-lint` is a library plus a CLI (`workflow-lint <FILE>...`;
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
* `banned-binding`: a node binds an agent or model containing `codex`.

**How a binding declares a memory write.** Gold Band's JSON has no memory
field, so the lint defines the smallest declaration: an agent id (a worker's
`provider`, a strategy's provider fields, or a wrapper binding's `agentId`)
that contains the marker `memory-write`, case-insensitive, for example
`claude-acp+memory-write`. A binding without the marker reads memory only.
Fixture workflows live in `crates/workflow-lint/tests/fixtures/`.
