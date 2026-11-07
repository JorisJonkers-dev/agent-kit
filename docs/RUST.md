# The Rust workspace and the 7-day crate cooldown

The estate's Rust lives in **one cargo workspace**, `scripts/Cargo.toml`, with
**one lockfile**, `scripts/Cargo.lock`, and **no git or path dependency**
outside the workspace. The rule and the 7-day crate cooldown come from the
standards repo (`JorisJonkers-dev/standards`, ADR-0014, whose `scripts/README.md`
documents its tooling); this page is how agent-kit follows it.

* **The cooldown.** No crate version younger than **7 days** may be used. On a
  workstation, Cargo resolves through the **crate-gate** proxy at
  `127.0.0.1:17342` (`~/.cargo/config.toml` points Cargo at it), which hides
  every too-young version, so a normal `cargo build` cannot pick one up even by
  accident. **crate-age** is the fallback check for after the fact: it exits `1`
  for a version proven younger than 7 days, `2` for a version whose age cannot
  be proven, and `0` only for a proven-old-enough version. The gate **fails
  closed**: an unprovable age blocks the change.
* **The workspace.** One member today, `crates/agent-kit-setup`, the setup
  canary: a std-only, dependency-free crate (nothing in the lockfile has an
  age to question) whose `--version` is what setup calls, and whose `validate
  <PATH>` checks a path. New Rust — the Gold Band launcher, the registry
  validation, the gate wrappers — joins this workspace as a new member, never
  as its own Cargo project.

```bash
cargo build --manifest-path scripts/Cargo.toml         # release canary in scripts/target/release/
cargo test --manifest-path scripts/Cargo.toml          # the canary's unit tests
```

The workspace keeps **100 percent line coverage** of every crate: the unit
tests in `src/lib.rs` execute each of its lines (the single executable line in
`src/main.rs`, the exit wrapper, is the one they cannot).

## Setup calls the crate

`scripts/converge.py` — the Python step the generated setup script runs on
every set-up — opens each invocation with one call to the crate's built binary
for its version, which is what proves the Rust path end to end from setup
itself:

```
agent-kit-setup: agent-kit-setup 0.1.0 (/…/agent-kit/scripts/target/release/agent-kit-setup)
```

When the binary has not been built the call is **skipped, with the reason said
out loud** (`skipped: not built yet; run cargo build --manifest-path
scripts/Cargo.toml`, or `skipped: no Rust toolchain (cargo) on PATH`) — and a
binary that exists but cannot answer a version **fails the run closed**. The
`tests/test_converge_canary.py` unit cases cover every branch against a fake
kit, and its end-to-end case builds and reports the real crate, skipping with
that same reason when the machine has no Rust toolchain.

## The pre-commit hook

`.githooks/pre-commit` runs **crate-age** over any staged `Cargo.toml` or
`Cargo.lock` change. This repository had no pre-commit mechanism, so this is
the smallest one: enable it once per clone:

```bash
git config core.hooksPath .githooks
```

(plus `chmod +x .githooks/pre-commit` if the checkout lost the executable
bit). How the hook obtains crate-age — it never copies it in:

1. `$CRATE_AGE` — a full path to the crate-age executable;
2. `crate-age` on `PATH` — installed once from its source in the standards
   repo, `scripts/crates/crate-age` (see that repo's `scripts/README.md`);
3. neither — no age can be proven, so the commit is **refused (exit 2)** with
   the reason printed.

## The pull-request workflow

`.github/workflows/crate-cooldown.yml` runs on every pull request (and on
pushes to `main`): it checks out this repository and the standards repo,
builds crate-age from `standards/scripts/crates/crate-age` with the runner's
cargo, and points it at the checkout. A too-young or unprovable version fails
the job, so the same change a contributor cannot commit cannot merge either.
A second job runs the workspace's `cargo test`. The workflow's checkout of the
standards repo follows its `main` until that repo publishes a tag; pin the
checkout the day it does.

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
   **expiry date** — no more than `approved_at` plus the days left of the
   cooldown, and 7 days out at the very most.
3. Only the approving owner merges the PR, with the failing gate overridden
   by that human approval, and the waiver entry is linked from the PR.
4. **The waiver expires.** On its `expires` date the entry is dead: it is
   removed from the ledger, and from then on the gate blocks that version
   again unless its seven days have passed. A waiver is never renewed — once
   expired, the normal cooldown applies again, and the next young version
   needs a new, separately approved entry.

How crate-age itself honours a waiver is defined by the standards repo
(ADR-0014); the ledger here is this repository's approval record for it.
