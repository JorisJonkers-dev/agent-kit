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
* **The workspace.** Six members today. `crates/inventory-check` fails when a
  skill directory has no row in `docs/gold-band-port-inventory.md` (#351).
  `crates/agent-kit-setup` is the setup
  canary: a std-only, dependency-free crate (nothing in the lockfile has an
  age to question) whose `--version` is what setup calls, and whose `validate
  <PATH>` checks a path. New Rust — the Gold Band launcher, the registry
  validation, the gate wrappers — joins this workspace as a new member, never
  as its own Cargo project.
* **The registry.** `crates/gold-band-registry` validates the Gold Band agent
  declarations — flattened into its line protocol by Python (#332) — and
  refuses a declaration that breaks one of the owner's hard rules (no Codex,
  no literal secrets, a constrained reach enforced by the reach support
  record, #339): every OpenRouter and every OpenCode model (an `opencode/`
  one included) is allowed (owner decision, 2026-11-08), the per-key LiteLLM
  budgets are the guard, and the model gateway keeps its own refusals. Its
  `launch <FILE> <ID>` subcommand is the reach-level launcher (#339): it
  refuses an id whose level the support record has not verified — before
  starting anything — and execs the id's adapter command otherwise, so the
  caller sees the agent's own exit code. The one line the unit tests cannot
  execute is the successful `exec` itself (a started agent replaces the
  process), the named exception above; `tests/test_gold_band_launch.py`
  proves it end to end with a fake agent.

  * `crates/ticket-export` writes requirement.md from one GitHub issue and
    its sub-issues through read-only GitHub access, so a workflow can start
    from a ticket without any agent holding write access (#355).
  * `crates/workflow-lint` reads a Gold Band workflow definition — bare
    or in the `--workflow` authoring wrapper — and refuses it when it
    breaks the model and loop rules every port shares: no banned agent or
    model, Opus only on a `-escalation` or `-acceptance` judgment node, and
    every back-edge or `$new-round` edge bounded by
    `control.max_attempts`/`control.max_rounds` (#371).

  * `crates/research-layout` validates the papers directory — catalog
    entries, quotes against stored pages, source tiers — regenerates its
    generated `INDEX.md` files, and mounts the papers repository as a
    private git submodule (#356).

* **The Gold Band allowlist check.** `crates/gold-band-allowlist` checks a
  Gold Band workflow definition against its repository's family allowlist
  entry before anything runs — the family allowlist, the code permission
  and the `nedap` and no-remote refusals, at setup and again at each run
  start through `exec` (#352, ADR-0009).

* **The Gold Band roles.** `crates/gold-band-roles` renders the critique
  pipeline's role and lens files into the six Gold Band profiles ("roles")
  of `~/.gold-band/context/profiles/`, rule for rule with templating off,
  and converges that directory: a retired role disappears, a kept
  hand-made one survives, and a Gold Band built-in is never touched
  (#345); `docs/gold-band-roles.md` maps each role to the model families
  that may run it.

```bash
cargo build --manifest-path scripts/Cargo.toml         # release canary in scripts/target/release/
cargo test --manifest-path scripts/Cargo.toml          # the canary's unit tests
```

The workspace aims at **100 percent line coverage** of every crate: the unit
tests in `src/lib.rs` execute each of its lines, with two kinds of exception
that each crate names in a comment beside the code: the single executable line
in `src/main.rs` (the exit wrapper), and the thin bridge that starts a real
process (`gh`, `git`, the command `exec` runs), which the tests replace with a
fake behind a trait. Coverage is not measured by a tool yet (`cargo llvm-cov`
is not installed); until it is, these claims rest on reading the tests.

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
