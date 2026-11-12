# Building the pinned Gold Band on Linux

[Gold Band](https://github.com/diodeme/Gold-Band) is a third-party (AGPL) app.
Setup makes the pinned commit **`987fc3e298b15d725a7f0e9b609b0f77dba63a74`**
(tag `v0.19.1`) present on a Linux workstation: the desktop app
(`gold-band-desktop`, a Tauri 2 app) and the headless `gold-band` binary, both
built from source, both installed to `~/.local/bin`. Upstream packages are not
used. Gold Band's code is never changed and never copied into this repository.

The surface is declared in `registry/estate-tooling.yaml` (`gold_band:`); the
builder is `scripts/gold-band/build-linux.sh`. The Mac build is a separate
ticket.

## What a setup run does

1. **No clone, no surface.** Setup builds from an existing clone at
   `~/Projects/diodeme/Gold-Band` (it never clones). Without one it prints
   `gold band: skipped, no clone at ...` and the run still succeeds.
2. **Read the pin without touching the clone.** The tree at the pinned commit
   is extracted with `git archive` into
   `~/.cache/agent-kit/gold-band/<commit>/src`.
3. **crate-age over Gold Band's `Cargo.lock`.** Exit `1` (a locked version is
   younger than 7 days) stops the build: the crate is printed, setup reports a
   failure, nothing is built or installed. Exit `2` (age unprovable) fails
   closed too, with exactly two exceptions that have no publish date by
   nature: Gold Band's own path crates, and the `tao` git patch, accepted only
   while the lockfile pins it to rev `c704261c`. An HTTP 429 from crates.io is
   reported as a rate limit to retry, never as a pass.
4. **Through the crate gate.** Gold Band ships a `.cargo/config.toml` that
   points crates.io at a third-party mirror (rsproxy.cn), which would bypass
   the gate. In the build copy only, it is replaced by a config that resolves
   through the gate at `127.0.0.1:17342` (`scripts/gate/install-linux.sh`
   installs the gate and crate-age; see [RUST.md](RUST.md)). One exception, stated rather than hidden: Gold Band's
   `[patch.crates-io] tao` is a git dependency at rev `c704261c`, which cargo
   fetches from github.com directly (the gate only mirrors crates.io); the
   builder refuses any other tao source. The web UI's npm
   packages come through the packages proxy that setup already configures.
5. **Build** `npm ci`, `npm run web:build` (the UI the desktop app embeds),
   then `cargo build --release -p gold-band --bin gold-band` and
   `cargo build --release -p gold-band-desktop`. The lockfile at the pin is
   stale for Gold Band's own two crates (`0.19.0` recorded, `0.19.1` in the
   manifests), so the build runs without `--locked`; afterwards the builder
   proves that no third-party entry of the lockfile changed.
6. **Install and report.** Both binaries go to `~/.local/bin`, and
   `~/.local/state/agent-kit/gold-band/build.json` records version, tag, build
   commit, build time, the age-check result and each binary's sha256. A run
   that finds that record matching the pin does nothing (`GOLD_BAND_FORCE=1`
   rebuilds). `setup-workstation.sh --check` only reports.

Exit status of the builder: `0` built, already built, or skipped; `3` a crate
too young or unprovable (build stopped, setup fails); `1` any other failure.

## System build dependencies (NixOS)

Do not install these system-wide. `scripts/gold-band/shell.nix` is a
`nix-shell` the builder runs the build in (`nix-shell scripts/gold-band/shell.nix
-I nixpkgs=flake:nixpkgs`). Its packages are what Tauri 2 on Linux needs:

| Need | nixpkgs attribute |
| --- | --- |
| Rust toolchain (edition 2024) | `cargo`, `rustc`, `gcc` |
| Web UI build | `nodejs_22` |
| Linking | `pkg-config`, `wrapGAppsHook3` |
| WebView | `webkitgtk_4_1` (webkit2gtk-4.1), `libsoup_3` |
| Toolkit | `gtk3`, `glib`, `cairo`, `pango`, `gdk-pixbuf`, `atk`, `librsvg` |
| TLS, tray, secrets | `openssl`, `libayatana-appindicator`, `libsecret`, `dbus` |

On other distributions the equivalents are the Tauri 2 prerequisites
(`webkit2gtk-4.1`, `gtk3`, `libsoup3`, `librsvg`, `libayatana-appindicator`,
`openssl`, a C toolchain) plus Rust and Node; when `cargo`, `node` and
`pkg-config --exists webkit2gtk-4.1` already work on `PATH`, the builder uses
them directly and skips `nix-shell`.

## Build results

Linux x86_64 (NixOS), 2026-11-10, from commit `987fc3e298b15d725a7f0e9b609b0f77dba63a74`:

* **Age check:** crate-age proved 766 of 768 locked registry versions at least
  7 days old and refused the two with no publish date by nature:
  `gold-band-desktop 0.19.0` (path) and `tao 0.35.3` (git, rev `c704261c`).
  No crate was younger than 7 days.
* **Build:** `npm ci` and the web build, then both cargo builds, all crates
  resolved through the gate; the lockfile changed only in Gold Band's own two
  versions.
* **Installed:** `~/.local/bin/gold-band` (29,941,328 bytes, sha256
  `3d833cea...0f38e12`; `--help` lists `run` and `continue`) and
  `~/.local/bin/gold-band-desktop` (97,125,168 bytes, sha256
  `0c532532...2ef3b`; every shared library resolves with `ldd`).
  The desktop window itself was not launched here (no display session).
* **Reported version:** the CLI has no `--version`; the version (0.19.1) is the
  source manifest's, recorded with the commit in the build record.


## The personal instance's agent map

After the build, setup converges the `agents` map of the personal instance's
`settings.json` (`${GOLD_BAND_HOME:-~}/.gold-band/settings.json`; the app keeps its data in
`.gold-band` under `GOLD_BAND_HOME`, not in `GOLD_BAND_HOME` itself) onto the
registry (`scripts/converge.py gold-band`). The app's map replaces its own
built-in list, so the result is exactly the declared agents plus the ones the
keep file (`~/.config/agent-kit/keep.yaml`) names:

* `gold_band.agents` in `registry/estate-tooling.yaml` declares each agent, and
  each id must be one `gold_band_instances.gold-band-personal.agents` names.
  OpenCode is the first. For an id the app ships itself, `adapter.command` and
  `adapter.args` are not written: the app strips them when it saves.
  `vibe` runs Vibe's ACP server (`vibe-acp`) on the personal login Vibe keeps
  itself. `vibe-study` runs `vibe-study --acp`: the launcher reads the Study
  account key from the secret store at launch and hands it to `vibe-acp`, so no
  key is written to a Gold Band file. With the key missing it starts nothing
  and says to run `vibe-study --login`.
* An agent declares its launch command per OS with `os:`, a map from `linux`
  and/or `mac` to a partial override of `binary`, `command_from_binary` and
  `adapter` (the adapter's keys win). Antigravity (`agy`) declares `mac` only:
  its adapter command is the installed `agy`'s absolute path, so no Homebrew
  prefix is written in the registry, and Linux skips it
  (`skipped: agy (no launch command declared for linux)`) until #431 settles
  whether it has a Linux build. Setup refuses a Linux launch whose binary,
  command or arguments name a macOS path (Homebrew, `/Applications`,
  `/Library`). Hermes runs `hermes-acp` on both OSes; the local Hermes CLI is
  still declared (only the cluster Hermes is retired), and its toolset cannot
  be restricted over ACP, so only the `write` reach is possible for it.
* A Claude agent (`claude`, `claude-work`) declares `config_dir`, a path that is
  absolute or starts with `~/`. It is registry-only: setup writes it as the
  entry's `adapter.env.CLAUDE_CONFIG_DIR`. Gold Band sets every `adapter.env`
  entry on the adapter process on top of the environment it inherits, and the
  adapter hands its environment to Claude Code, so a `CLAUDE_CONFIG_DIR` in the
  launcher's own environment cannot switch the identity. The personal agent
  (`claude`) holds `~/.claude-personal`, the work agent (`claude-work`,
  `instance: gold-band`) holds `~/.claude`. Setup refuses a Claude agent with
  no `config_dir`, two agents on one directory, and an `adapter.env` that
  names `CLAUDE_CONFIG_DIR` by hand. The work agent is converged on the work
  instance with `converge.py gold-band --instance gold-band`; setup runs the
  personal instance on Linux and the work instance on the work Mac (below).
* The work instance holds exactly `claude-work` and `codex` (`instance:
  gold-band`, adapter `@agentclientprotocol/codex-acp`, pinned). Codex in an
  adapter command is refused for every agent of the personal instance, and
  allowed for the work instance's own.
* An undeclared agent is removed unless the keep file names it. On the personal
  instance an id that names Codex goes even when kept; any id in
  `gold_band.retired_agents` is removed even when kept.
* An agent whose `binary` is not on PATH is skipped and reported
  (`skipped: <id> (<binary> is not installed)`); its existing entry is left alone.
* Every other key of `settings.json` keeps its value and its order. A second
  run prints `settings unchanged` and writes nothing. A change keeps one
  `settings.json.bak`.
* Nothing is written while the desktop app runs (setup stops and says so), and
  a machine without Gold Band gets no settings file. `--check` only reports.

## The work instance on the Mac

On macOS setup converges the work instance when the per-machine overlay
(`~/.config/agent-kit/overlay.yaml`) names it under `instances.gold-band`; no
overlay, or one that names only the personal instance, leaves it alone. The call
is `converge.py gold-band --instance gold-band --settings
${GOLD_BAND_HOME:-$HOME}/.gold-band/settings.json --overlay <overlay>`. The
`agents` map ends as exactly `claude-work` and `codex` (the keep file is not read for
the work instance); the
overlay's `settings` are set at their dotted paths in `settings.json` and its
`mcp_servers` become `contextServers` entries (upserted by id; an entry the
overlay no longer names is left for the owner to remove). The overlay is
validated by the `gold-band-registry` crate first (so `cargo` or `GOLD_BAND_REGISTRY_BIN` is needed) and refused whole when it
fails, and it may not set `agents` or `contextServers` as a setting. Nothing in
it is copied into the registry, and the registry holds no employer value. The
work account's own `settings.json` is the target: the personal instance runs
under the second macOS user.

## Skill sets per agent id

An agent can be given its own set of skills. `gold_band.skill_sets` in
`registry/estate-tooling.yaml` names each set and the local skills it holds, and
an agent names one with `skill_set`:

```yaml
gold_band:
  skill_sets:            # name -> list of local skill names
    estate: [estate-bootstrap, estate-rollback]
  agents:
    - id: opencode
      skill_set: estate  # registry-only field, never written to settings.json
```

* A skill name must be a `name` in the registry's top-level `local_skills` list;
  its source is that entry's `path` under the kit, a directory holding
  `SKILL.md`. `skill_set` is registry-only: it is never written to
  `settings.json`.
* Setup builds ONE directory per agent id that names a set, at
  `<settings.json's directory>/skill-sets/<agent id>/` (`--skills-root` moves
  it), holding exactly that set's skills, each a verbatim copy of the skill
  directory under the kit. Skills not in the set are removed, changed files are
  rewritten, missing ones are added, and an up-to-date directory is not touched.
  Directories are created `0o700`, and a symlink is copied as a symlink.
* Nothing is shared: the agent's entry gets `primaryAgentDir` set to the
  absolute path of its own directory and `compatibleAgentDirs` set to `[]`. An
  agent that names a set may not also declare `primaryAgentDir`,
  `projectPrimaryAgentDir` or `compatibleAgentDirs`.
* A directory under the skills root for an id that is not installed with a set
  is removed, unless the id is skipped (its binary is missing, so its existing
  directory stays) or the keep file names it. Only direct children of the skills
  root are ever removed.
* `--check` reports `added skills <id>`, `changed skills <id>` and
  `removed skills <id>` and writes nothing; a normal run prints the same lines
  after doing the work. The skill directories are written before
  `settings.json`, so an unchanged agent map never skips them.
* A `skill_set` that is not a string or names an undeclared set, a `skill_sets`
  value that is not a list of strings, a duplicate skill, a skill that is not in
  `local_skills`, a skill source without `SKILL.md` and a set name that is not a
  Gold Band id are all refused before anything is written.

## The run-start shim

Every Gold Band workflow opens with a cheap, read-only node that runs one
pinned wrapper, `gold-band-run-start` (`scripts/crates/gold-band-run-start`,
ADR-0014). It decides from the working copy and the owner's records, never
from model output, whether the run may go on. The node is a worker whose
agent runs the wrapper and answers with the report as its output artifact. Its
success condition is `exit_code` equal to `0`, and it has no failure edge, so a
refused check ends the run in failure before any model node starts.

```bash
gold-band-run-start check --instance gold-band|gold-band-personal \
    --repo DIR --workflow-name NAME --run-id ID --attempt N \
    --workflow FILE --families FILE [--allowlist FILE] [--grants FILE] \
    [--now RFC3339] --report-dir DIR
```

It runs three checks and prints every problem on stderr, one per line,
prefixed `gold-band-run-start:`:

* **`repository-rule`.** Against every remote (`git remote -v`). The work
  instance (`gold-band`) allows only a repository whose every remote is owned
  by `nedap` (any case; https, ssh and scp URLs) and refuses one with no
  remote. The personal instance (`gold-band-personal`) refuses any
  `nedap`-owned remote and allows no remote at all. A remote with no
  `host/owner/repo` is refused: its owner cannot be shown.
* **`family-allowlist`.** The family of every agent id the workflow binds (a
  worker's `provider`, the `modelBindings` joined by `executionSlotId`, an
  ai-dynamic node's strategy) against the repository's entry in
  `$XDG_CONFIG_HOME/agent-kit/critique-allowlist.json` (or
  `~/.config/agent-kit/`), and the entry's `code` permission, since a workflow
  with a worker sends code. `history` is never required. No entry means the
  personal default: every family, code. On a `nedap` repository the file is not
  read: only `claude-work` and `codex` are allowed. An agent id missing from the
  families file (`<agent id><TAB><family>` lines) is refused.
* **`grant`.** Work instance only: every bound agent id containing `codex` needs
  a live grant for the run `<workflow name>@<host/owner/repo>@<run id>` in the
  gateway's grants record (`model-gateway-grants.json`): client `codex`, that
  workflow, not ended, and not past `expires_at`. The refusal names the nodes
  and prints the `model-gateway grant write --json` call to ask the owner with.
  The grant's `max_starts` counts Codex node starts in the per-node grant
  check, not here.

Exit codes: `0` every check passed, `1` refused, `2` bad usage or unreadable
input (a missing file, malformed JSON, a directory that is not a repository).
The report, `<report-dir>/run-start.json`, is written in all three cases when
the report directory is usable, through the shared `gold-band-report` crate:
`run_id`, `attempt`, `command`, `exit_code`, `output_hash` (SHA-256 of the
printed output), `timestamp` (UTC) and, only when the run did not pass,
`failed_check` (`repository-rule`, `family-allowlist`, `grant`, `usage` or
`input`). `tests/test_gold_band_run_start.py` proves the refusals headless
against the pinned `gold-band` binary with stub agents.

## The grant-check node

In the work instance every node that binds Codex is preceded by a grant-check
node that runs the same pinned wrapper "for the grant alone" (ADR-0011):

```bash
gold-band-run-start grant-check --repo DIR --workflow-name NAME --run-id ID \
    --attempt N --node NODE --report-dir DIR [--gateway CMD]
```

It names the run's gateway workflow `<workflow name>@<host/owner/repo>@<run
id>` (the `@` is allowed in a gateway workflow name for this) and calls the
TypeScript gateway, `model-gateway grant reserve`, which holds one Codex node
start against that run's grant. A Gold Band grant counts node starts in its own
field, `max_starts` (`grant write --max-starts N`, or `"max_starts"` in the
object), because the gateway never sees Gold Band spend; `cap_usd` stays dollars
and is never read as a start count, and a grant without `max_starts` makes the
reservation fail with `no-max-starts` (spec #392 story 17). The
reservation is decided and written inside the gateway's ledger lock, so two
parallel Codex nodes cannot both take the grant's last start; a reservation
holds for about a minute and then settles, and a settled start still counts, so
none is given back (a wrapper that reserves and then crashes before it reports still used a
start). The gateway keys a start by workflow, node and attempt, so a node whose
agent runs the wrapper twice still holds one. The grant is read just before the
ledger lock is taken, so a grant that lapses in those few milliseconds is caught
by the next node's check, not this one.

Only the gateway's own `status: ok` passes (exit 0, the report
`<report-dir>/grant-check.json`, the grant and the starts left printed). The
node fails otherwise, which a workflow routes by a failure edge to a manual
check, so a run never goes on to Codex past its grant: exit 1 with
`failed_check` `grant-lapsed` (no grant, or it lapsed by time or workflow
end), `grant-no-start-left` (a live grant with every `max_starts` start taken)
or `grant-reserve` (any other gateway answer, such as `no-max-starts` for a
grant written for dollars, a gateway that cannot be started or prints no JSON); `repository-rule` when the repository has no remote
to name the run by. The gateway is `--gateway`, else `GOLD_BAND_MODEL_GATEWAY`,
else `model-gateway` on `PATH`. `tests/test_gold_band_grant_check.py` proves it
against the real gateway bundle in a throw-away configuration: a grant that
lapses mid-run pauses the headless run at the next Codex node's manual check
(exit 2), and parallel wrapper processes cannot pass on the last start. The
proof harness (`docs/gold-band-proof.md`) gives an agent id `grant-<name>` the
`grant` role and a `grant-lapse-<name>` scenario for a checked-in workflow.

## The gate shim

Gold Band has no command node, so a deterministic gate (build, tests, lint,
coverage, crate age) runs through a shim, never through a change to Gold Band
(`scripts/crates/gold-band-gate`). Green means the command ran and
exited zero. Three nodes make the shape, and
`tests/fixtures/gold-band/gate-proof.json` is a working example:

1. **The gate node**, a cheap worker whose agent id runs only the one pinned
   wrapper. It answers with the wrapper's exit status, and its success
   condition is `exit_code` equal to `0`.
2. **The copier node**, a read-only worker that runs `gold-band-gate verify`
   and answers with the report it prints. Its success condition is `exit_code`
   equal to `0`, and when `verify` refuses the node answers with a non-zero
   `exit_code`, so a refusal cannot be masked.
3. **The next node**, reached only when both succeeded. Neither shim node has
   a failure edge, so any failure ends the run.

Workflow B's coverage gate is one such pair (`gate-coverage`, `copy-coverage`):
the wrapper runs `coverage-gate --repo-dir DIR` (docs/RUST.md), which demands
100 percent line coverage on the files the change touches.

```bash
gold-band-gate run --run-id ID --attempt N --timeout-secs S \
    --report-dir DIR -- COMMAND [ARGS...]
gold-band-gate verify --run-id ID --attempt N --command "COMMAND ARGS" \
    --report-dir DIR [--max-age-secs N] [--now RFC3339]
```

**How a gate runs.** `run` clears `DIR/gate-report.json` and
`DIR/gate-output.txt` from any earlier attempt first, then runs the command with
stdout and stderr captured, in order, into `DIR/gate-output.txt` (once the command ends, at most 8 MiB
are kept). It then writes `DIR/gate-report.json` through `gold-band-report`:
`run_id`, `attempt`, `command` (the command and its arguments joined by single
spaces), `exit_code`, `output_hash` (SHA-256 of the captured output),
`timestamp` and, only for a timeout or a command that could not start,
`failed_check` (`timeout` or `cannot-start`). A command that runs longer than
`--timeout-secs` (1 to 604800) is killed with its process group, so a hung
command never hangs the wrapper.

**Exit codes.** The wrapper exits with the report's `exit_code`, so the gate
node fails when the command does: the command's own exit code; `124` when it
timed out; `127` when it could not start (not found, not executable);
`128` plus the signal number when a signal ended it. `2` is bad usage or a
report directory it cannot prepare or write (the command did not run, or no
report exists). The timeout and cannot-start reasons are also written into
`gate-output.txt`, so they are part of the hashed output.

**What the copier checks.** `verify` only reads `DIR`. It prints every problem
on stderr, one per line, `gold-band-gate: <check>: <why>`, and exits `1`:

* `missing`: there is no report.
* `stale`: the timestamp is older than `--max-age-secs` (default 3600) or more
  than a minute in the future.
* `other-run` and `other-attempt`: the report is for another run id or attempt,
  for example a concurrent run's report.
* `command-mismatch`: the report ran a command other than `--command`.
* `hash-mismatch`: the SHA-256 of `gate-output.txt`, recomputed, is not the
  report's `output_hash`, or the file is missing or over 8 MiB.
* `failed-command`: `exit_code` is not `0` (`124` is named as a timeout), or a
  `failed_check` is present.

On success it exits `0` and prints the validated report on stdout, re-serialised, for the node
to answer with. Exit `2` is bad usage or a report that cannot be read or is
malformed: the report is untrusted, so it is size-bounded (64 KiB) and every
field is type-checked.

**The report directory must not be writable by the gate agent id.** The whole
shim rests on it: an agent that could write `gate-report.json` and
`gate-output.txt` could forge a pass with a matching hash. The wrapper and the
copier do not enforce this. It is enforced by the launcher's operating-system
write limit on the agent id (ticket #432): the directory is outside what the
gate agent id can write, and the launcher must still let the wrapper write it,
so the wrapper has to run outside that limit (the launcher's design, not this
crate's). Until that limit is in place a gate is only as
trustworthy as the agent that runs it. `tests/test_gold_band_gate.py` proves,
headless, that a green, red, unstartable and hung command end as they should
and that a deleted report, rewritten output, a rewritten command, a stale
report and a report from another run or attempt each end the run in failure.
Its stub does the tampering itself, since the limit is not in play there.

## The finding check node

Workflow B's critics answer in the critique runner's finding schema
(`skills/workstation/critique/schemas/finding.schema.json`). A critic can be
wrong or invent a quote, so a finding drives a repair only when it can be
checked. The finding check node is a gate node between the critics and the
repair nodes: the pinned `gold-band-gate run` wrapper around
`gold-band-findings check`, followed by the read-only copier
(`gold-band-gate verify`). The crate is `scripts/crates/gold-band-findings`;
Workflow B wires the node after every critics round (see "Workflow B's critics,
repairs and escalation" below).

```bash
gold-band-findings check --run-id ID --attempt N --diff FILE --ticket FILE \
    --report-dir DIR FINDINGS_FILE...
```

Each `FINDINGS_FILE` is one critic's whole answer. A finding is **kept** when it
validates against the schema and has at least one `quote` evidence item, every
quote of which is found in the diff or the ticket (runs of whitespace count as
one space; a quote shorter than 8 characters is too short to count). Every other
finding is a **hint**, with its reason: `schema`, `no-quote`, `quote-too-short`
or `quote-not-found`. A hint is kept in the report and never sent back to the
writer.

It writes `DIR/findings-check.json` and prints it: `run_id`, `attempt`,
`actionable` (the number kept), `hint_count`, `kept` and `hints`. A kept entry is
`{source, finding}`; a hint is `{source, id, reason, detail, finding}`, where
`source` is the index of its findings file. A repair node is reached only when
`actionable` is above zero, so a run with hints alone does not go back to the
writer. Exit `0`: the check ran. Exit `1`: a critic's answer is not a finding
document (not JSON, an unknown key, `findings` not an array, or a missing or empty
`no_findings_reason` where it is required or given), so it is not
read as "no findings". Exit `2`: bad usage, an unreadable input or an unwritable report directory. The
old `findings-check.json` is removed before each run.

## Workflow B's critics, repairs and escalation

`installer/gold-band/workflows/workflow-b.json` (spec #392, stories 78 and 90;
issue #451) continues the main path (test author, one writer, the build, test,
lint and coverage gates) with the review and the repairs. Everything below is
fixed nodes: the file has no cycle and no `$new-round` edge, so its cost is known
before it runs.

1. **Critics.** `critics-1` is one `ai-dynamic` node with a dynamic strategy.
   Its candidate workers are Vibe GLM (`vibe`, `glm-5-3`) and Claude (`claude`,
   `sonnet`), both different families from the writer's (Vibe-study, DeepSeek)
   and the test author's (Vibe, Mistral). The routing prompt tells the planner to
   fan out exactly two workers, one per provider, with the `pf-diff-reviewer`
   profile, and each critic writes its whole answer in the critic finding schema
   to `findings/<its node id>.json` in its worktree, committed so the merge
   brings it back. The node's limits are explicit: at most two fanned-out nodes,
   two in parallel, one group deep, no nested dynamic node and five dynamic nodes
   in all (the bootstrap, two critics, the merge and the acceptance).
2. **Finding check.** `gate-findings-N` and `copy-findings-N` are the finding check
   node above, then `verdict-N`, a read-only worker that reads
   `findings-check.json` and answers `{"actionable": N}`. Its success condition is
   `actionable` equal to `0`, so a run with hints only goes on and never repairs.
   The gate wrapper's arguments (the findings files, the diff and the ticket) are
   pinned by the live wiring ticket (#455), as for every other gate agent id.
3. **Repairs.** A verdict that fails goes to `repair-1`, the writer's agent and
   model, which fixes the kept findings only. The build, tests, lint and coverage
   gates run again (`gate-build-r1` and so on), then `critics-2`, its finding
   check and `verdict-2`. A second failure goes to `repair-2`, the same chain and
   `critics-3`. Two repairs is the cap: there is no `repair-3`.
4. **Escalation.** When `verdict-3` fails, `workflow-b-escalation` runs, the one
   node in the personal instance that binds Opus (its id ends in `-escalation`,
   as the workflow lint requires). It judges the ticket and the last findings,
   and either outcome of it goes to `escalation-check`, a manual check, so the run
   pauses (exit 2) for the owner and never ends on a control limit.

A failing gate or copier, in any round, has no failure edge and ends the run in
failure, as on the main path. `control.max_attempts` stays `1`: Gold Band counts a
failure-triggered jump per edge (from one node to another), and in a graph with no
cycle every edge runs at most once, so the cap is never exceeded and the run cannot
end on the control limit before it reaches the escalation. The CI wait is added by a
later ticket.

## The critique stage templates

`installer/gold-band/workflows/critique-<stage>.json` wraps each critique
stage as a Gold Band workflow whose nodes run the critique runner, so the
runner stays the executor: it keeps its read-only tripwire, packet
confinement, time limits and family allowlist, and Gold Band only sequences
the stages and pauses on the manual check (spec #392, stories 102 and 111).
The stages are `decisions`, `spec`, `tickets`, `ticket-review`,
`feature-review`, `adr-review`, `decision-recovery` and `architecture-review`;
`critique-spec-stage.json` is the older template that sends the spec to the
critics directly and loses the runner's guarantees, so it stays for personal
repositories only.

Every template has the same path:

1. `gate-critique-<stage>-start`, the run-start shim: it refuses the run when
   the repository rule or the family allowlist (including its code
   permission) forbids an agent id the workflow binds, before any model node
   runs;
2. `prepare`, `fan-out`, then (after the orchestrator's `<stage>-verdicts`
   node) `refute` and `record`, each a gate node, the pinned
   `gold-band-gate run` wrapper around `critique <command> --strict`,
   followed by the read-only copier that verifies its report. `--strict`
   makes a refusal (`"status": "refused"`) exit 4, so a refused prepare ends
   the run instead of reading as a pass;
3. `<stage>-verdicts` answers with the findings it could not settle. An empty
   list goes straight to `refute`; any other answer goes to the
   `<stage>-needs-human` manual check first, and the run pauses there.

Nothing in a template reimplements a runner guarantee. A critic that writes
outside its packet is voided by the runner's tripwire, a search outside the
packet returns nothing, and a critic past its time limit is stopped by the
runner; `council/ts/src/critique-cli/template-run.test.ts` proves each through
the same `--strict` command line a gate node runs. The headless proof
(`gold-band-proof prove`) checks each template's sequencing only, against stub
agents: a clean run ends in success, a finding pauses at the manual check, and
a gate that fails (the stub makes it) ends the run in failure.

Two layers refuse on the allowlist, and they differ on an absent entry. The
run-start node checks the family of every bound agent id and applies the
personal default when the repository has no entry; the runner's `prepare`
refuses a repository with no entry (`not-opted-in`) and, under `--strict`,
exits 4. The gate agent ids are pinned to their commands, and the run
arguments they need, by the later wiring tickets: the node goals name the
command only.

## The Wave 3 templates over the runners

`critique.json`, `critique-stats.json`, `flow.json` and `build-tracker.json` in
`installer/gold-band/workflows/` are the four Wave 3 rows of
`docs/gold-band-port-inventory.md` (issue #627, spec #392 story 106). Each node
runs the same command the direct lane runs, through the gate shim, and no
template holds logic of its own:

| Template | Direct-lane command | Nodes |
| --- | --- | --- |
| `critique` | `critique prepare`, `fan-out`, `refute`, `record` | the run-start gate, prepare and fan-out shims, the orchestrator's `critique-verdicts` and its `critique-needs-human` manual check, then the refute and record shims. The eight stage templates are this shape bound to one stage. |
| `critique-stats` | `critique stats` (`/critique-stats`) | the run-start gate, one `critique stats --strict` shim |
| `flow` | `critique state` (`/flow`) | the run-start gate, one `critique state --strict` shim |
| `build-tracker` | `build-tracker refresh` | the run-start gate, one `build-tracker refresh` shim |

The gate adds `--strict` to a `critique` command, as the stage templates do, so a refusal fails the run instead of reading as a pass. Every shim is the pinned `gold-band-gate run` wrapper and the read-only copier
that verifies its report; the run arguments are pinned to the gate agent ids by
the wiring tickets, as in the stage templates. The steps only the owner can take
in `/flow` (the grill, the spec, the tickets, the implementation) stay the
owner's commands. The headless proof (`gold-band-proof prove`) checks each
template's sequencing against stub agents: a clean run ends in success, the
`critique` template's finding pauses at its manual check, and a failing gate
ends the run in failure.

## Workflow C in the personal instance

`installer/gold-band/workflows/workflow-c.json` is Workflow C, grill to tickets
(spec #392, stories 93 and 104; issue #459), for the personal instance. It
pauses on a manual check wherever the owner has to settle something:

1. `gate-workflow-c-start` (the run-start shim, as in the critique templates)
   and `gate-workflow-c-setup` (the gate shim around `ticket-publish check`)
   run before any model node. A repository without a valid
   `docs/agents/setup.yaml` ends the run in failure here.
2. Up to three grill rounds, unrolled (`grill-round-1` to `grill-round-3`).
   A round answers with its `open_questions`; an empty list goes on to the
   spec, any other answer pauses on that round's `grill-check-<n>` manual
   check, and the owner's answers feed the next round. After round three the
   check hands over to the spec regardless.
3. `spec-draft`, then the spec review (`/review-spec`), then `tickets-draft`
   and the ticket review (`/review-tickets`). The review nodes are the
   `critique-spec` and `critique-tickets` templates' own, unchanged (the
   runner stays the executor, behind the gate shim), including the
   `<stage>-needs-human` manual check for a finding the orchestrator cannot
   settle. The tickets are drafted as `tickets.json`; no node holds a tracker
   credential.
4. `publication-check`, a manual check on every run. Only after the owner
   passes it does `gate-workflow-c-publish` run `ticket-publish publish`
   through the gate shim, which creates the tickets in the tracker the
   repository's setup file names and refuses a label the file does not list.
   The publish gate has no other incoming edge (a test asserts it), and the
   publisher refuses a repository without the setup file again by itself.
   Publishing is not idempotent: it does not look for issues that already
   exist, so after a publish that stopped part way, check the tracker before
   running it again.

The spec and ticket drafts are written by a different family (Vibe, Mistral)
from the orchestrator that rules on their review (Vibe-study, DeepSeek) and
from the critics the runner picks. No node writes memory and none binds Codex
or Opus; the grill rounds use Vibe GLM. The work-instance variant is a
separate ticket.

A headless run cannot continue past a manual check (`gold-band continue`
refuses a run that waits for user input), so the proof proves each stretch of
the workflow on its own: see "Segments" in [gold-band-proof.md](gold-band-proof.md).
The proof pauses (exit 2) at a round with an open question
(`open-questions`), at each needs-human check (`needs-human`, once for the spec
review and once, in the `after-spec-needs-human` segment, for the ticket review)
and at the publication check (`clean`); the publish gate and its failure are
proved in the `after-publication-check` segment. What the proof does not check:
that `ticket-publish` creates real issues (its tests fake `gh`), and that a
real model asks sensible questions.

## Wave 4: the human-gate workflows

`grill-me`, `grill-with-docs`, `brainstorming`, `to-spec`, `to-tickets` and
`pr-composer` (issue #628) are checked-in templates under
`installer/gold-band/workflows/`. Gold Band has no per-node loop counter, so each
file carries its question rounds **unrolled**: one round node and one
manual-check node per round, up to the round cap its description states. A round
answers with `open_questions`; an empty list goes on, any other answer pauses on
that round's check for the owner's answers (in the app or over IM), and the next
round reads them. After the last round the check hands over regardless.

| Workflow | Round nodes | Cap | After the rounds |
| --- | --- | --- | --- |
| `grill-me` | `grill-round-<n>`, `grill-check-<n>` | 5 | `grill-summary` |
| `grill-with-docs` | `grill-round-<n>`, `grill-check-<n>` | 5 | `docs-update` writes the answers that change a decision into CONTEXT.md and the ADRs |
| `brainstorming` | `explore-round-<n>`, `explore-check-<n>` | 4 | `design-draft`, the `design-approval` manual check on every run, then `design-record` |
| `to-spec` | `spec-round-<n>`, `spec-check-<n>` | 3 | `spec-write` |
| `to-tickets` | `tickets-round-<n>`, `tickets-check-<n>` | 3 | `tickets-write` writes `tickets.json`; nothing is created in a tracker |
| `pr-composer` | `pr-confirm-<n>`, `pr-apply-<n>` | 3 | the push-and-create gate |

`pr-composer` orients, gathers context, asks for linked issues and threads
(`pr-links`, with `pr-links-check` as its manual check), drafts the body and runs
the template check as a gate shim, then renders the preview. The first
confirmation round pauses on `pr-confirm-1` on every run (a later one only
after the owner asked for an edit); `pr-apply-<n>` answers with an
empty `open_questions` only when the owner's answer is an explicit affirmative
for this preview, and otherwise applies the owner's edits and re-renders for the
next round. `gate-pr-composer-publish` (the push and the pull request create) has
no incoming edge except from the `pr-apply-<n>` nodes, each of which follows its
own confirmation check (a test removes the confirmation nodes and asserts the gate
is unreachable). After round three without a confirmation the run ends in failure
with nothing pushed or created. The proof only ever runs the gate shim's stub, so
no proof pushes, creates or posts anything.

The proof of each is segmented, as for Workflow C: it pauses (exit 2) at a round
with an open question, at the check after it, and so on, and ends in its expected
state in the segment after the last check (see [gold-band-proof.md](gold-band-proof.md)).

## The generated roles

Gold Band reads a role as a **profile**: a Markdown file with YAML front
matter in `${GOLD_BAND_HOME:-~}/.gold-band/context/profiles/<name>.md` (not
`presets/profiles`, which its profile resolver never looks at). After the agent
map, setup converges that directory onto the registry's declared roles
(`scripts/converge.py gold-band-roles`), on both instances, in the account that
runs setup:

* `gold_band.roles` in `registry/estate-tooling.yaml` declares each role: its
  file stem, its front matter id (`pf-...`), a name and summary, its role file
  under `skills/workstation/critique/roles/` and the lens files it uses. Six are
  declared: the decision-gap critic, the spec red team, the ticket linter, the
  diff reviewer, the feature reviewer and the adversarial verifier.
* Each profile is the role file, the shared preamble and output contract, the
  finding schema and the role's lenses, copied verbatim, then
  `scripts/gold-band/roles-closing.md`. Templating is off
  (`dynamicTemplate: false`) and the text is copied, never formatted, so braces
  survive byte for byte. The timestamps are fixed, so a second run prints
  `profiles unchanged` and writes nothing.
* The closing section says what the role cannot enforce: the runner's read-only
  proof, packet confinement, quote check, schema check, refute round, family
  allowlist and `nedap` refusal, and plan-limit routing do not exist on a Gold
  Band path.
* A profile file the registry does not declare is removed unless the keep file
  (`~/.config/agent-kit/keep.yaml`) names its stem (`my-role` or `my-role.md`).
  A stem or id in `gold_band.retired_roles` is removed even when kept. Gold
  Band's own `pf-builtin-*` profiles are never touched.
* Nothing is written while the desktop app runs (setup stops and says so), a
  machine without Gold Band gets no profiles directory, and `--check` only
  reports.

## Quiet settings and the updater gap

Setup replaces `settings.json` atomically (a staged 0600 file renamed over the
old one, one `.bak` kept, the data directory 0700) and keeps every key it does
not declare. It also turns the app's outbound components off, in the pinned
app's `SettingsConfig` (`src/config/mod.rs`):

* `desktopMetricsEnabled` (line 881) is set to false.
* `desktopMulticaEnabled`, the task-board integration (line 892), is set to false.
* Each `imIntegrations.channels[].enabled`, the chat connectors (line 888,
  `src/im/model.rs:485`), is set to false.

A key that is absent stays absent; the app defaults each to off.

Two components cannot be switched off from settings:

* **Updater.** The app has no updater switch. The only setting is
  `desktopUpdaterUrlOverride` (`src/config/mod.rs:877`), an endpoint override.
  Dropping it would not turn the updater off and could send a machine pinned
  to a mirror back to the public endpoint, so setup leaves it untouched.
  "Updater off" is a gap to raise upstream.
* **Feedback.** The app has no feedback setting, so there is nothing to write.
