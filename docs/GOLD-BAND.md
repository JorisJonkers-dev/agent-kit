# Building the pinned Gold Band on Linux

[Gold Band](https://github.com/diodeme/Gold-Band) is a third-party (AGPL) app.
Setup makes the pinned commit **`987fc3e298b15d725a7f0e9b609b0f77dba63a74`**
(tag `v0.19.1`) present on a Linux workstation: the desktop app
(`gold-band-desktop`, a Tauri 2 app) and the headless `gold-band` binary, both
built from source, both installed to `~/.local/bin`. Upstream packages are not
used. Gold Band's code is never copied into this repository; the build copy gets
one small documented patch to the editor's entry rule (see "The build patch").

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
3. **Through the packages proxy.** Gold Band ships a `.cargo/config.toml` that
   points crates.io at a third-party mirror (rsproxy.cn). In the build copy
   only, it is replaced by a config that resolves through the packages proxy,
   using the token setup wrote to `~/.cargo/credentials.toml`. Gold Band's
   `[patch.crates-io] tao` is a git dependency at rev `c704261c`, which cargo
   fetches from github.com directly (the proxy only serves crates.io). The web
   UI's npm packages come through the packages proxy that setup already
   configures.
4. **Build** `npm ci`, `npm run web:build` (the UI the desktop app embeds),
   then `cargo build --release -p gold-band --bin gold-band` and
   `cargo build --release -p gold-band-desktop`. The lockfile at the pin is
   stale for Gold Band's own two crates (`0.19.0` recorded, `0.19.1` in the
   manifests), so the build runs without `--locked`.
5. **Install and report.** Both binaries go to `~/.local/bin`, and
   `~/.local/state/agent-kit/gold-band/build.json` records version, tag, build
   commit, build time, the age-check result, the sha256 of the build patches
   (`patches_sha256`) and each binary's sha256. A run
   that finds that record matching the pin and the patches, with both installed
   binaries' sha256 equal to the recorded ones, does nothing (`GOLD_BAND_FORCE=1`
   rebuilds). `setup-workstation.sh --check` only reports.

6. **Make it findable and runnable (every run, built or not).** The builder
   installs `~/.local/bin/gold-band-launch`, `${XDG_DATA_HOME:-~/.local/share}/gold-band/runtime.nix`,
   `applications/gold-band.desktop` (`Name=Gold Band`, `Exec` the launcher's
   absolute path, `Icon=gold-band`, `Categories=Development;`,
   `StartupWMClass=gold-band-desktop`) and the pinned source's
   `src-tauri/icons` as `gold-band.*` in `icons/hicolor`: `32x32.png`,
   `64x64.png`, `128x128.png`, `128x128@2x.png` (256x256) and `icon.png`
   (512x512), each under the directory of its real size, and
   `logo-source.svg` under `scalable/apps`. A file that differs, hand-installed
   or not, is replaced; one that matches is left alone (`up to date`). The
   launcher execs `gold-band-desktop` directly where `pkg-config` finds
   `webkit2gtk-4.1` (or there is no `nix-shell`); on NixOS it enters
   `runtime.nix`, which puts WebKitGTK, GTK, libsoup 3 and the rest on
   `LD_LIBRARY_PATH` and their share directories on `XDG_DATA_DIRS`. After a
   change the icon cache is refreshed with `gtk-update-icon-cache -f -t` when
   that tool exists (a failure is not fatal); where it does not (NixOS), a stale
   `hicolor/icon-theme.cache` is removed so it cannot hide the new icons.
   `--check` reports `would install` and writes nothing. `build-linux.sh
   --retire [--check]` removes all of these files and keeps the binaries and
   the clone. `GOLD_BAND_SHARE` moves the data home.

Exit status of the builder: `0` built, already built, or skipped; `1` any failure.

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

Convergence also writes `settingsSchemaVersion`, because Gold Band reads a
`settings.json` without it as a legacy file and the app refuses to start
(`cannot migrate unsupported managed agent`). The value is
`GOLD_BAND_SETTINGS_SCHEMA_VERSION` in `scripts/converge.py` (12 at `v0.19.1`,
Gold Band's `CURRENT_SETTINGS_SCHEMA_VERSION`), declared beside
`GOLD_BAND_PIN_TAG`; a test ties that tag to the registry's and the builder's.
Bump both with the pin. A file already at that value or higher keeps its value;
it is never lowered.

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
  `adapter` (the adapter's keys win). Antigravity (`agy`) declares `mac` and
  `linux`: its adapter command is the installed `agy`'s absolute path, so no
  Homebrew prefix is written in the registry. Setup refuses a Linux launch whose binary,
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

The shipped registry declares one set, `estate`: the four estate skills
(`estate-bootstrap`, `estate-project-intent`, `estate-delivery-status`,
`estate-rollback`). `opencode`, `vibe` and `vibe-study`, the personal
instance's declared agents, name it, so each gets those four skills in its own
directory and an id that does not name it gets none.

Three general sets follow, one skill each: `agent-session-bootstrap`,
`kb-first` and `token-economy`. An agent's `skill_set` is one set name or a
list of them (the skills are merged, and two named sets may not hold the same
skill), so the three personal-instance agents with a skill directory name
`[estate, agent-session-bootstrap, kb-first, token-economy]`. `claude`, `pi`,
`agy` and `hermes` keep their own agent directories and resolve to no set.
The repo group's Claude and Codex renderings of the three skills are
byte-identical to the workstation sources a set copies, so both groups are
covered; setup writes only under the instance's `skill-sets/` directory and
never touches the direct-lane copies.

## The personal instance's workflow templates

Gold Band keeps its workflow templates in one user-level file,
`${GOLD_BAND_HOME:-~}/.gold-band/context/workflows.json` (a `templates` list, each
entry a `workflow` with its `modelBindings`, shared by every workspace). The
checked-in workflows under `installer/gold-band/workflows/*.json` are installed
into that file on the Linux workstation by a setup run
(`agent-kit-merge converge gold-band-workflows`), after the agent map and the roles:

* **Personal instance only.** The file is the personal instance's. A workflow
  whose bindings (or a worker's `provider`) name an agent of the work instance
  (`gold_band_instances.gold-band.agents` in the registry: `claude-work`,
  `codex`) is not installed and is reported as `skipped <id> (bound to the work
  instance's ...)`; today that is `workflow-b-work` and `workflow-c-work`. Setup
  does not touch the work instance's store, on the Mac or anywhere else.
* **One template per file.** A file's name is its workflow's `id` and becomes the
  template's `id` and `name`. A template the store lacks is added at the end
  (`added <id>`). One whose `workflow` or `modelBindings` differ from the file is
  replaced in place (`changed <id>`), keeping its position, its name and
  `createdAt`; the comparison ignores `null` fields, which the app drops when it
  re-saves. An up-to-date store prints `workflows unchanged` and is not written.
* **The keep file wins.** A workflow whose id (or `<id>.json`) is in
  `~/.config/agent-kit/keep.yaml` is never added or overwritten
  (`kept <id> (keep file)`), so a template you edited in the app stays yours.
  Nothing is ever removed: the app's two built-in templates, templates you made
  and every other key of the file keep their place and value. A built-in under a
  shipped id stops the run before anything is written.
* **Shape-checked.** A template that fails Gold Band's editor validation rules
  (below) is refused like a malformed source file: `cannot converge: a template
  fails Gold Band's workflow shape rules`, one `<file>: <node>: <rule>: <message>`
  line per finding, exit 3, nothing written. The templates in
  `installer/gold-band/workflows/known-shape-failures.txt` are tolerated for now
  (#781) and each prints `warning: <id> fails the editor's shape rules ... installing
  it anyway`, so setup keeps working today.
* **Atomic.** The new store is written beside the old one (`0600`, fsynced) and
  renamed over it, with one `workflows.json.bak` holding what it replaced, so the
  app never reads half a file. A file that is not a template store, or a source
  file that is malformed, is refused and nothing is written. A missing store is
  created (the app then adds its built-ins when it loads it).
* **Never while the app runs.** Setup stops with `Gold Band is running` (exit 1)
  and writes nothing, as for the agent map; a machine without Gold Band gets no
  file. `setup-workstation.sh --check` (or `agent-kit-merge converge gold-band-workflows
  --check`) prints `would add <id>` and `would change <id>` and writes nothing;
  under `setup-workstation.sh --check` such a line is a failure, so a machine
  whose templates are missing or drifted does not pass the check.

Why a machine can lack them (#768): the step lives in setup, which refuses to
write while the desktop app runs and stops there, and `--check` used to report
`would add` as an ordinary log line. The converge itself writes the file the app
reads (`context/workflows.json`; `auto-templates.json` and the per-project
folders hold other data), and the app keeps templates it does not own when it
re-saves the file (it only upserts its built-ins), so a converged store survives
a restart. To apply by hand with the app closed:

```sh
cd ~/Projects/JorisJonkers-dev/agent-kit && ./installer/setup-workstation.sh
# or only this step:
agent-kit-merge converge gold-band-workflows \
  --keep ~/.config/agent-kit/keep.yaml --store ~/.gold-band/context/workflows.json \
  --workflows installer/gold-band/workflows --registry registry/estate-tooling.yaml
```

The installer bundle ships `installer/gold-band/workflows`, so a setup run from a
release bundle installs the same templates (`tests/test_publish_installer_artifacts.py`).
`tests/test_gold_band_workflows_converge.py` proves each rule against throw-away
stores.

## How the workflows are validated

Gold Band v0.19.1 validates a workflow in one place only: the React editor
(`web/src/components/WorkflowEditor.tsx`, `validateWorkflowForSave`, about lines
3395-3680). The Rust runtime and the `gold-band` CLI never run it (there is no
`validate` subcommand), so `gold-band-proof` runs a template headless and passes
while the same template opens in the app with errors such as `Workflow has multiple
entry nodes` or `spec-write requires a success expression when output validation is
enabled`. The validator cannot be called headless: it is a function of the
editor's UI state (live agent bindings, the profile catalog, the template store),
not a library or an endpoint, and a request for a `validate` command (#418) was
closed as not pursued. So the kit ports the rules.

`workflow-lint --shape` (`scripts/crates/workflow-lint/src/shape.rs`, the `shape`
rule group; `workflow-lint --shape --instance gold-band|gold-band-personal FILE...`)
ports them from Gold Band commit `987fc3e298b15d725a7f0e9b609b0f77dba63a74`
(the pinned build; `workflowGraph.ts` for `isBackwardEdge` and the success-edge
topological order, `deriveWorkflowEntryCandidateIds` for the entry count). A finding
reads `<file>: <node or workflow>: <rule>: <message>`, with the editor's own message,
and the exit code is the crate's (0 clean, 1 findings, 2 unreadable). Rules:

* **Entry**: exactly one node with no incoming edge, where a non-success edge that
  goes backward in the success-edge topological order and closes a loop (its target
  reaches its source over success edges) does not count as incoming; any other
  failure edge does, so a failure target is not an entry (the build patch below;
  unpatched v0.19.1 also ignored a backward failure edge to such a node)
  (`entry-multiple`, `entry-missing`); at least one edge to `$end` (`end-required`);
  at least one node (`nodes-required`).
* **Success expression**, for a worker with `output` or `success_condition`:
  `output.artifact` set, a `success_condition` present and parseable, the output
  `schema` in the simplified shape and containing the condition's path
  (`output-artifact-required`, `success-expression-required`,
  `success-expression-invalid`, `legacy-schema`, `schema-missing-path`); and no
  `manual_check` beside it (`manual-check-exclusive`). A `failure` edge needs a
  worker with a `manual_check`, `output` or `success_condition`
  (`failure-edge-unsupported`).
* **Nodes**: unique, non-empty ids that are not `$end`, `$entry` or `$new-round`
  (`node-id-required`, `duplicate-node-id`, `reserved-node-id`); an outgoing edge
  (`dangling-node`); an `executionSlotId`, unique across workers (`slot-required`,
  `slot-duplicate`) and a `profile` (`profile-required`).
* **Edges**: both endpoints exist (`edge-source-required`, `edge-source-missing`,
  `edge-target-required`, `edge-target-missing`), `on` is `success` or `failure`
  (`edge-outcome-invalid`), one edge per `(from, on)` (`duplicate-edge-outcome`),
  no success edge to `$new-round` (`success-new-round`), a `$new-round` edge names
  its start (`new-round-entry-required`, `new-round-entry-missing`), `$end` and
  `$new-round` are never a source (`terminal-edge-source`), and a `continue` session
  never targets them (`continue-terminal-target`).
* **Control**: `max_attempts` and `max_rounds`, when set, are above 0; the workflow
  has an `id`.

**Skipped, because they need the running app:** that a worker's agent and model
exist and passed the agent doctor, its permission mode and config options, that its
role is still in the profile catalog, that the workflow id is unique in the template
store, and every rule of an `ai-dynamic` node. The lint runs without bindings, so
those stay with the app. The existing rule groups (`opus-outside-judgment`,
`memory-write-outside-judgment`, `loop-not-unrolled`, `banned-binding`) are unchanged
and still run alongside it.

**Where it runs.** CI's `Gold Band Workflow Shape` job builds `workflow-lint` and runs
`scripts/check-workflow-shape.sh` over every file in `installer/gold-band/workflows/`
(`*-work.json` as the work instance, the rest as the personal one). Setup's
`agent-kit-merge converge gold-band-workflows` applies the same rules in process, through
the `workflow-lint` library (`shape_of_definition`, and the base and critique rules), so
there is one implementation and no Python twin.
The fixtures are under `scripts/crates/workflow-lint/tests/fixtures/shape-valid` and
`shape-refused` (one per rule, including the two owner-reported errors).

**Known failures (#781).** Every worker that has `output` carries a `success_condition`
of `{"path": "$.ok", "equals": true}` over its declared `{"ok": true}` output, and
`grant-refused` in the work variants has its edge to `$end`. The `entry-multiple`
findings (a human check reached only by a failure edge) are cleared by the build patch
below, not by the templates: no change of nodes can satisfy the unpatched rule. What
may still be listed in `installer/gold-band/workflows/known-shape-failures.txt` (one
file per line, a comment pointing to #781) is named there with its reason: the two
spec critics of `critique-spec-stage` output a findings list that no condition can
accept in every case. CI and setup tolerate exactly the listed files: any other
template, and any new one, must pass; a listed template that now passes fails CI with
"remove it from the list".

### Every written artifact is critiqued by another family (#793)

`workflow-lint --critique --instance gold-band|gold-band-personal FILE...` runs the
`critique` rule group (`scripts/crates/workflow-lint/src/critique.rs`; its module
header is the definition). It is its own flag and its own list, run by the same CI step
(`scripts/check-workflow-shape.sh` does `--shape` and then `--critique`). Findings read
`<file>: <node>: <rule>: <message>`. Gold Band's editor keeps no field for any of this,
so nothing is declared in a template: a node is recognised from what the shipped
templates already carry.

* **Writer.** A `worker` whose `output.artifact` key, minus a leading `repair-<N>-`, ends
  in `write`, `draft`, `summary`, `update`, `record`, `implementation` or `written`
  (`spec-write`, `tickets-draft`, `grill-summary`, `docs-update`, `design-record`,
  `implementation`, `repair-1-implementation`, `tests-written`), and whose binding has a
  known model family. Gate and copy shims (`stub-model`, `pinned-command`), question
  rounds and verdict nodes are not writers. A new template names its artifact with one of
  those endings to be covered.
* **Critique record node.** The runner chain's `gate-critique-<stage>-record`, or workflow-b's
  `verdict-<N>`.
* **`critic-required`.** Every path from a writer to `$end`, over success and failure edges,
  crosses a record node. A path through `grant-refused` (a refused grant ends the run
  without handing the artifact on) does not count. The finding names the shortest
  unreviewed path.
* **Critic.** An `ai-dynamic` node (candidates: its `availableAgents`), a worker bound to a
  review profile (any profile but `pf-builtin-dev`; its own binding), or the runner's
  `gate-critique-<stage>-fan-out` (candidates: that stage's clients in
  `skills/workstation/critique/routing.json`, slots `base` and `deep` for the personal
  instance, `work` and `deep` for the work one; routing fallbacks are not followed).
* **`critic-family`.** No candidate of a critic is of the family of the nearest writer
  upstream of it (a writer with a path to the critic that crosses no other writer). The
  family comes from `routing.json`, the file the critique pipeline routes with, and there
  is no table of the lint's own: the binding's model (provider prefix cut, `.` read as `-`)
  matched on whole `-` segments against each client's `model` and `gateway.model`, else
  the agent id's client kind when all its clients share a family (`claude-work` is
  `anthropic`). A model with no family is never compared. A per-machine routing override
  is not read by the lint; the run-start shim reads it (below).
* **The orchestrator's family is not in a roster (#796).** The runner's `prepare` takes the
  orchestrator's family and drops every client of it from the stage's roster, so the lint
  leaves the family of the stage's `<stage>-verdicts` worker out of the fan-out's candidates.
  This is how a template excludes its writer's family from the roster it reviews with: it binds
  the writer and the stage's orchestrator to one family. `workflow-c` binds `spec-draft` and
  `tickets-draft` to Vibe GLM (Zhipu) and `spec-verdicts` and `tickets-verdicts` to Vibe-study GLM (a
  separate agent id, since the headless proof gives one agent id one role); `workflow-c-work`
  binds all four to Codex, so its work roster is the work Claude agent alone (the work instance
  has two families, so one writer family leaves one critic family, never two). Nothing else in
  the template declares it, and a template whose orchestrator is of another family than its
  writer is refused if the roster holds the writer's family.

**Known failures.** `installer/gold-band/workflows/known-critique-failures.txt`
(separate from the shape list, dated) names each violating template with the ticket that
fixes it, one `<file> #<ticket>` per line. CI tolerates exactly those: an unlisted
violator fails, a listed template that now passes fails with "remove it from
`known-critique-failures.txt`", and a line without a ticket fails. Today: `to-spec`,
`to-tickets`, `brainstorming`, `grill-me` and `grill-with-docs` have no critic (#795).
`workflow-c` and `workflow-c-work` passed `critic-family` once their writers and orchestrators
were bound to one family (#796; before it a Mistral writer met `pi-vibe-mistral-large` and a
Claude writer met `claude-work`). `workflow-b` and the critique templates pass both rules.

`agent-kit-merge converge gold-band-workflows` runs the same check in process, with the
`workflow-lint` library (its base rules and the critique rules, as `workflow-lint --critique`
runs them): listed templates install with `warning: <id> breaks the critique rules ...
installing it anyway`, any other violator refuses the run before anything is written.

## The generated critique block (#795)

A template does not hand-copy the critique chain. `critique-block`
(`scripts/crates/critique-block`, std plus the model gateway's ordered JSON; there is no
Python or TypeScript generator) writes one shared block per stage into a template, from the
recipes in `installer/gold-band/critique-blocks.json`, and rewrites the template in place:

```bash
critique-block generate --recipes installer/gold-band/critique-blocks.json --dir installer/gold-band/workflows
critique-block check    --recipes installer/gold-band/critique-blocks.json --dir installer/gold-band/workflows
```

`check` fails when a template differs from what the recipes generate (the crate's tests run it
over the shipped templates, so CI holds it). A recipe names the template, the stage (`spec` or
`tickets`), the writer node, the node the accepted artifact goes to (`next`), and the bindings:
the orchestrator, an optional needs-human override, the escalation judge and its check, an
optional pre-check command, and, for the work instance, the grant refusal node and a grant-check
node to clone for every node bound to Codex. The generator is idempotent: it removes the nodes it
owns (including a template's own hand-written chain for the stage) and generates them again.

```text
writer -> [pre-check] -> CRIT(1) -> accept-1 --success--> next
                                    accept-1 --failure--> revise-1 -> [pre-check] -> CRIT(2)
                                      -> accept-2 --success--> next
                                         accept-2 --failure--> escalation -> escalation-check -> $end
```

* `CRIT(N)` is the stage template's runner chain (`critique-<stage>.json`: prepare, fan-out, the
  orchestrator's `<stage>-verdicts` and its needs-human manual check, refute, record), each gate with
  its copier. Round 2 appends `-2` to every id and artifact and asks `prepare` for fresh critics
  over the first round's run directory (`previous_run_dir`); `workflow-lint --critique` reads the
  `-2` ids as the same stage with the same roster.
* `accept-<stage>-N` is a shim agent (pinned like the gate shims, wiring named in its goal) that
  reads round N's record and answers `{"open_blocking": n}`; the round is accepted when `n` is 0.
* `<stage>-revise-1` is the writer's own binding applying the accepted findings of round 1 and
  nothing else; fresh critics review the result before any exit.
* After a second failed round `<stage>-escalation` (the Opus judge, allowed by the lint's
  `-escalation` rule) states what is still open and `<stage>-escalation-check` is a manual check
  whose only way on is `$end`: a failed second round never reaches `next`, so nothing passes
  automatically and the run ends needing the owner.
* The most a stage runs is two rounds, the loops are unrolled (the lint refuses cycles), so the cost is
  static. The only edges into `next` come from an accept node. For `tickets` the pre-check is
  `critique check-tickets --strict` (cycles, dangling blockers, uncited stories) before each round's
  critics, so revised tickets are checked mechanically too.
* The writer and the orchestrator are bound to one family (see "Every written artifact is critiqued by
  another family"); the revise node has the writer's binding, so it is of that family as well.

Applied to `to-spec`, `to-tickets`, `workflow-c` and `workflow-c-work` (the spec and the tickets
stage; Workflow C's runner chain was advisory before, with no step that applies a finding).
`brainstorming`, `grill-me` and `grill-with-docs` stay in `known-critique-failures.txt`; #798 and #794 consolidate
them into the generated workflows. The headless proof (`gold-band-proof prove`) gains the `accept`
role: `accept-<stage>-<n>` answers `{"open_blocking": 1}` under the verdict `actionable=K` while
`n <= K`, so `actionable-1` proves the revise path (the second critic round runs, then the next stage)
and `actionable-2` proves the escalation (the run pauses at the escalation check, exit 2, and the
next stage is never reached).

## The build patch: failure targets are not entries

Unpatched v0.19.1 refuses 21 of the 24 shipped templates (Save, run-mode start and
task-list start). `deriveWorkflowEntryCandidateIds` ignores every non-success edge
that goes backward in the success-edge topological order, so a node reached only by
a failure edge (the human check `spec-check-N` after `spec-round-N --failure-->`)
counted as a second entry whenever its id sorted before the failing node: "Workflow
has multiple entry nodes". The failure -> human-check loop is the intended design
and no change of nodes satisfies the rule, so setup patches the build copy.

`scripts/gold-band/patches/0001-entry-rule-failure-targets.patch` changes that one
function: a backward non-success edge is ignored only when it closes a loop (its
target reaches its source over success edges, the retry back to the entry that
Gold Band's own test `accepts a failure back edge into the initial entry node`
keeps). Any other failure edge counts as incoming. A node with no incoming edge at
all is still an entry and a workflow with none still errors. The patch carries its
own vitest file (`web/tests/workflow-entry-failure-targets.test.ts`).

* `build-linux.sh` applies every `patches/*.patch` (name order, `git apply`) to the
  build copy before `npm run web:build`. A patch that does not apply (the pin moved)
  stops the build with exit 1 and a message; nothing is installed. The clone is
  never touched. The pin is unchanged.
* The sha256 of the patches is recorded in `build.json` (`patches_sha256`, `none`
  without patches); a record with another or no value is not "built", so the next
  setup run rebuilds. `--check` prints the hash. After this lands, run once:
  `GOLD_BAND_FORCE=1 scripts/gold-band/build-linux.sh` (needs crates.io not
  rate-limited, #772).
* `workflow-lint --shape` (and setup, through the same library) implements the patched rule.
* Verification (needs Gold Band's source and `npm ci`, so not in CI): apply the
  patch to the pinned source, copy `scripts/gold-band/patches/templates-entry-rule.test.ts`
  to `web/tests/`, and run
  `GB_TEMPLATES=<kit>/installer/gold-band/workflows npx vitest run --config web/vitest.config.ts web/tests/templates-entry-rule.test.ts web/tests/workflow-entry-failure-targets.test.ts web/tests/workflow-editor-new-round.test.ts`.
  Unpatched, 21 templates fail it; patched, all 24 pass the entry rule.
* Trade-off: a failure edge back to a node that does not reach its source over
  success edges now makes that node a non-entry; with no other entry the editor
  reports `entry-missing`, as it does for any workflow without one.

**Dropping it.** Upstream proposal: JorisJonkers-dev/agent-kit#787. When a Gold Band
release carries the same rule, move the pin, delete the patch (and its test file), run
`scripts/gold-band/build-linux.sh`, and check `shape.rs` against the new
`deriveWorkflowEntryCandidateIds`. If the pin moves
without that, the build stops at the patch step: regenerate it against the new
source (`git diff` of `deriveWorkflowEntryCandidateIds`) or drop it.

Longer term (also #780): a check that runs the pinned app's own validator, so that a
pin bump cannot drift from the port. Until then, re-read `validateWorkflowForSave`
when the Gold Band pin moves.

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
    [--routing-override FILE] [--now RFC3339] --report-dir DIR
```

It runs four checks and prints every problem on stderr, one per line,
prefixed `gold-band-run-start:`:

* **`repository-rule`.** Against every remote (`git remote -v`). The work
  instance (`gold-band`) allows only a repository whose every remote is owned
  by `nedap` (any case; https, ssh and scp URLs) and refuses one with no
  remote. The personal instance (`gold-band-personal`) refuses any
  `nedap`-owned remote and allows no remote at all. A remote with no
  `host/owner/repo` is refused: its owner cannot be shown.
* **`family-allowlist`.** The family of every model the workflow binds (#783): each
  (agent id, model) pair of a worker (`provider` and `model`), of the `modelBindings`
  joined by `executionSlotId` (`agentId` and `modelId`) and of an ai-dynamic node's
  strategy (`provider` and `model`, `bootstrapProvider` and `bootstrapModel`, the
  `acceptanceModel` on the strategy's provider, and every `availableAgents` entry's
  `provider` and `model`), against the repository's entry in
  `$XDG_CONFIG_HOME/agent-kit/critique-allowlist.json` (or
  `~/.config/agent-kit/`), and the entry's `code` permission, since a workflow
  with a worker sends code. `history` is never required. No entry means the
  personal default: every family, code. The family is the model's, so a Pi agent
  (one id, several families) is checked per model: `pi` on `litellm-vibe/glm-5.3` is
  `zhipu` and on `litellm-vibe/mistral-large-4.0` is `mistral`, and a candidate list
  is refused for each entry whose family the repository does not allow. It resolves
  through the `model-family` crate, the same resolution as `workflow-lint --critique`
  and read from the same `routing.json` (with the machine override of `critic-family`
  merged over it): the model with its provider prefix cut and `.` read as `-`, matched
  on whole `-` segments, longest first, against each client's `model` and
  `gateway.model`; else the agent id's client kind when all its clients share one family
  (`claude-work` is `anthropic`). A binding the routing cannot place falls back to its agent
  id in the families file (`<agent id><TAB><family>` lines); one neither places is refused.
  On a `nedap` repository the file is not read: only the agent ids `claude-work` and `codex`
  are allowed, and (per model) only their families, `anthropic` and `openai`, so
  `claude-work` bound to a Mistral model is refused too.
* **`critic-family` (#796).** The `critic-family` rule of `workflow-lint --critique` over the
  routing this machine runs: the routing agent-kit ships with the machine override
  (`--routing-override FILE`, else `$HOME/.config/agent-kit/critique-routing.json`, the file the
  runner reads) merged over it. A run whose critics can be of the family of the writer they
  review is refused (`failed_check: critic-family`, one line per critic), so an override that
  puts a writer's family back into a stage's roster stops the run before any model node. A
  definition with no edge list has nothing to pair and is left to Gold Band's validator.
  The runner drops the orchestrator's family itself (see "Every written artifact is
  critiqued by another family"). An unreadable or malformed override file named by the flag
  is exit 2.
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
`failed_check` (`repository-rule`, `family-allowlist`, `critic-family`, `grant`, `usage` or
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

## Workflow B v2: slices, a test critic, gate repairs and a code review

`installer/gold-band/workflows/workflow-b.json` (spec #392, stories 78 and 90;
issues #451 and #797) is the Pocock `implement`, `tdd` and `code-review` loop as
fixed nodes: the file has no cycle and no `$new-round` edge, so its cost is known
before it runs. The shape, in order (85 nodes):

1. **The seams and the plan.** `slice-plan` (the test author, Mistral on Pi) reads
   the ticket and the spec it names, takes the test seams from the spec's Testing
   Decisions section and writes `slice-plan.json`: at most three vertical slices, each one
   behaviour at a named seam. Every later test author node is told to read the
   same seams and test through their public interface only.
2. **Three unrolled slices** (`test-author-s1`..`s3`). A slice is `test-author-sN`
   (ONE failing test, run to see it fail for the right reason; no assertion on a private
   function, a call count or an internal field, no value copied from the code's own
   output), then `test-critic-N`, then the finding check, then `writer-sN` (DeepSeek on Pi:
   red, then the least change that turns the test green, then a tidy while
   green; earlier slices stay green). The plan may hold fewer than three slices; an
   empty slice's nodes answer `ok` and change nothing. All tests are no longer
   written before the writer starts (the horizontal slicing the `tdd` skill names as an
   anti-pattern).
3. **The test critic** (`test-critic-N`, GLM on Pi with the `pf-diff-reviewer` profile) is
   a third family, neither the test author's (Mistral) nor the writer's (DeepSeek), and runs on
   the slice's test before its writer starts. It looks for tests coupled to the
   implementation and tautological tests (the `test-seams` lens, in the goal), and writes
   the critic finding schema to `findings/test-critic-N.json`. `gate-findings-N`,
   `copy-findings-N` and `verdict-N` keep only findings with a checkable quote (the finding check
   node above); a failed verdict sends the test back to the test author once
   (`test-revise-N`), then the writer starts. The revised test is read by the fresh critics
   of the code review (item 5), so no revision exits unreviewed.
4. **Gate repair edges.** After the last slice, `gate-build`, `gate-test`, `gate-lint` and
   `gate-coverage` each run through the gate shim, each followed by its copier. A failed gate (not a copier:
   a report that does not verify still ends the run) goes to `repair-1`, the writer's family,
   which reads the failing gate's output and fixes the cause; the four gates run again (`-r1`),
   a failure goes to `repair-2` and the gates (`-r2`), and a failure there goes to
   `gates-escalation` (Opus) and the manual check `gates-check`, after which the review
   runs anyway. Two repairs is the cap.
5. **The code review.** `self-review` is the writer's own review of its change on the two axes (Standards and
   Spec); it never blocks. Then the runner's `ticket-review` stage runs as a generated block
   (`critique-block`, recipe in `installer/gold-band/critique-blocks.json`): `prepare`, `fan-out`,
   the orchestrator's `ticket-review-verdicts` and its `needs-human` manual check, `refute`, `record`
   and `accept-ticket-review-N`, so the read-only tripwire, the quote check and the refute round
   are the runner's own, not bespoke critics. The two code-review axes are two entries of the
   stage's `base` roster in `skills/workstation/critique/routing.json`, each its own critic
   with its own lens (`lenses/spec-conformance.md` and `lenses/standards.md`) from a
   competitor family: Gemini (`spec-conformance`, `failure-modes`, `security`) and GLM
   (`standards`). The orchestrator is bound to the writer's family (DeepSeek), so the runner
   drops that family from the roster (the lint's #796 rule); the `deep` depth adds Mistral, the test
   author's family, which is why the fan-out goal says not to ask for it.
6. **Repair, fresh critics, escalation.** An accepted round goes to the final gates; a round with
   open blocking findings goes to `ticket-review-revise-1` (the writer), then a second round with
   fresh critics (`-2` of every stage node, `previous_run_dir` set so findings are not raised again). A
   second failed round goes to `ticket-review-escalation` (the one other Opus node, its id ends
   in `-escalation` as the lint requires) and its manual check, and the run pauses (exit 2). Every
   path from a writing node to `$end` crosses a review record (`workflow-lint --critique`).
7. **The final gates** (`-r3`) run once more after the review. A failure there goes to the
   `final-check` manual check: nothing repairs a reviewed change automatically, since a repair would
   need a third review round.

`control.max_attempts` stays `1`: Gold Band counts a failure-triggered jump per edge, and in a graph
with no cycle every edge runs at most once, so the cap is never exceeded and the run cannot end
on the control limit before it reaches an escalation. The CI wait is added by a later ticket.

**Proof (headless, stub agents).** `workflow-lint --shape` and `--critique` pass (the entry rule
is the patched one: the manual checks reached by failure edges are not entries), and `gold-band-proof
prove` against the real Gold Band binary passes 36 scenarios for the personal template and 44 for the
work one: `clean`; `needs-human` (the orchestrator's finding pauses); `actionable-1` (a test critic
round and a review round, each repaired, exit 0) and `actionable-2`, `actionable-3` (both review rounds
fail: the Opus escalation's manual check, exit 2); `gate-fail-<gate>` for the four first-pass gates (a
repair follows, exit 0) and the four final gates (`-r3`, the final check, exit 2); a gate failing in every
repair round (`gate-fail-build-and-build-r1`, exit 0; `...-and-build-r2`, the gate escalation, exit 2) for
each of the four; `gate-fail-findings-N` and the runner's four gates (no failure edge, exit 1); and
`after-<check>.…` for the stretch after each manual check. The work template adds `grant-lapse-<node>` for
the eight Codex nodes on the clean path (writers, test critics, self-review, the review orchestrator); the
other four grant checks (`repair-1`, `repair-2`, `ticket-review-revise-1`, `ticket-review-verdicts-2`) sit
behind a failure edge and are proved by `tests/test_gold_band_workflow_b_work.py`, which runs each Codex
node's stretch. `scripts/crates/critique-block` proves the runner's nodes in the template are what the recipe
produces (`critique-block check`).

## Workflow B live in the personal instance

A live run (issue #455) binds the model nodes to the personal instance's own
agents, as setup converged them, and every shim node to a **pinned shim agent**:

```bash
gold-band-proof run --workflow installer/gold-band/workflows/workflow-b.json \
    --pins installer/gold-band/pins/workflow-b-agent-kit.json \
    -- --requirement-file requirement.md
```

* **The run home.** `--pins` builds a home under
  `$XDG_STATE_HOME/agent-kit/gold-band/live/<workflow id>-<nanos>/`: its
  `settings.json` is the instance's (`--instance-home`, else `GOLD_BAND_HOME`,
  else `HOME`) with one agent added per `gate-*`, `copy-*` and `verdict-*` id,
  and `context` links to the instance's generated profiles. The instance's own
  `settings.json` is never written, so setup's agent map stays exactly what the
  registry declares. A model agent the workflow binds that the instance does not
  hold, a shim id the instance already declares, a shim the pins file names no
  command for, and a grant check (the work instance's) each refuse the run
  (exit 5) before Gold Band starts.
* **A wall-clock limit.** `--timeout SECS` with `--pins` kills the live run's
  process group when the limit passes and exits 6, so no agent outlives it.
  Gold Band has no per-node time limit, and the pins file's `timeout_secs` bounds
  one gate command only; without `--timeout` a live run is unbounded. Always pass
  it, and kill by pid (`pgrep`, then `kill`) if it must stop earlier.
* **The pinned shim agent** (`gold-band-proof pinned-acp`) is deterministic: it
  never calls a model. Its role and its one command are fixed in the adapter's
  environment, so nothing in a prompt can change what it runs. A gate runs
  `gold-band-gate run` around its command and answers `{"exit_code": N}`; a
  copier runs `gold-band-gate verify` for the same command and answers with the
  report; a verdict reads `findings-check.json` of its round and answers
  `{"actionable": N}`. The run id and attempt come from the hidden runtime blocks
  Gold Band puts first in the prompt (`<project>/<task>/<run>`, `attempt-NNN`),
  never from the requirement or goal after them, and the reports go to
  `<run home>/reports/<project>/<task>/<run>/<shim>/`. A prompt with no readable
  place, or a verdict whose report is missing or names another run, is answered
  with a protocol error: Gold Band pauses the run, so it never reads as a pass or
  as a finding to repair. A session runs its command once; Gold Band's finalize
  turn gets the same answer.
* **The pins file** (`installer/gold-band/pins/<workflow>-<repository>.json`)
  belongs to the repository under test: `{"version": 1, "timeout_secs": N,
  "shims": {"build": [...], ...}}`. A shim is found by its name (`build-r1`),
  then without the repair suffix (`build`), then without the round
  (`findings-2` to `findings`). Placeholders: `{repo}` (the node's working
  directory), `{report_dir}`, `{run_id}`, `{attempt}`, `{tools}` (the directory
  of the `gold-band-proof` binary, where the workspace's other tools are built,
  so a gate never runs a tool from the working copy under test), `{base}` (the
  commit the repository was at when the run started) and `{requirement}`.
* **agent-kit's pins** run the Rust workspace: `cargo build --all-targets`,
  `cargo test`, `cargo clippy --all-targets` (clippy's deny-by-default lints
  only: the workspace is not warning-clean, so `-D warnings` would fail every
  run), `coverage-gate --base {base} --ignore /tests/` (100 percent on the
  files the run changed; a test file under `tests/` is left out, since
  `cargo llvm-cov` never reports one and the gate would read it as unmeasured), and the finding check over `findings/*.json` against the diff from
  `{base}` (the findings themselves left out) and the requirement.
  The coverage gate needs `cargo llvm-cov` with an LLVM of rustc's major version
  (`LLVM_COV` and `LLVM_PROFDATA` set), and the gates build with this host's
  linker flags, so the run is started inside the shell that provides them.

**v2 and the live run.** Workflow B v2's slices, test critics, finding checks, gates and repairs
bind the same pinned shims and Pi nodes as before (`findings-1`..`findings-3` now check the test critics'
findings; `-r3` names the final gates). The code review is the runner's `ticket-review` stage, whose gates
(`gate-critique-ticket-review-prepare` and the rest) and `accept-ticket-review-N` shims are pinned by the live
wiring of the stage templates, which no template has yet (the node goals name the command only). Until it
lands `gold-band-proof run --pins` refuses a live run of v2 (exit 5) and names exactly those ids, which
`the_checked_in_pins_bind_every_shim_of_workflow_b_but_the_review_stage_s` pins; the headless proof does not
depend on it.

What a live run still does not have: the run-start shim is not a node of
Workflow B (its family check resolves the family from the bound model since #783,
so a Pi node is no longer refused for sharing an id; a pinned shim has no family and
is refused unless the families file places its agent id). The report directory is
writable by the model agents of the run (the operating-system write limit of ticket #432 is not
applied to Gold Band's adapters yet), so a gate is only as trustworthy as the
agents beside it.

### Live runs of Workflow B (personal instance, Linux, 2026-11-14)

Both runs used `litellm-vibe/mistral-large-4-0` (Pi, the test author) and
`litellm-vibe-study/deepseek-v4.1-flash` (Pi, the writer); the pinned shims ran
the Rust gates. Pi does not report usage to Gold Band, so
`acp.prompt-usage.jsonl` holds null token totals and a token figure is not
recorded. The `litellm-vibe-study` route answers a one-token request in 0.6
seconds (HTTP 200), so the route and its key are not the cause of the first run.

* **Run `workflow-b-1794683412334736133`, ticket #695 (Rust JSON parser limits),
  killed by hand after about 30 minutes.** The test author finished in five
  minutes. The writer never hung: it made 401 tool calls over 1829 seconds
  (cargo test, scripted file rewrites, cargo test again) on a ticket whose
  requirement is open-ended, and never answered, so the node stayed `paused`
  with no outcome. Nothing bounded it: Gold Band has no per-node time limit and
  the run had no wall-clock limit. The run is now bounded by `--timeout` (above),
  and a ticket for a live run should be small and have one acceptance
  criterion per test.
* **Run `workflow-b-1794686077997506440`, ticket #699 (triage of a change that
  touches a git submodule, Rust port only), limit 2700 seconds, ended
  `completed` with `failure` after 1004 seconds.** The test author took 691
  seconds, the writer 237 seconds, the build gate passed and the test gate failed
  (`cargo test`, exit 101): 392 tests passed and the test author's own
  `a_change_that_updates_a_submodule_pointer_is_read_instead_of_failing` failed
  (`left: 1, right: 2` on the changed-file count of a gitlink). Workflow B runs
  with `max_attempts` 1, so a failed gate ends the run before the critics; the
  repair rounds and the critics have not yet run live. The ticket was public data
  only, so the `litellm-vibe` routes were permitted.

## Workflow B in the work instance

`installer/gold-band/workflows/workflow-b-work.json` (spec #392, stories 80 and
81; issues #462 and #797) is Workflow B v2 bound to the work instance's two agents only,
`claude-work` (Anthropic) and `codex` (OpenAI), beside the personal `workflow-b.json`. It has
the same shape (98 nodes: the personal template's 85, the `grant-refused` check and twelve grant checks): the plan, three unrolled
slices with a test critic, the gate repair edges, the self-review, the generated `ticket-review` block
and the final gates. The bindings differ. With two families a test critic cannot differ from both
the test author and the writer, so it differs from the test author (which the lint checks) and the
writer, its consumer, is of its family:

* **Claude** plans and writes each slice's test (`slice-plan`, `test-author-sN`, `test-revise-N`),
  reviews the code (the orchestrator is Codex, so the runner drops Codex from the stage's `work` roster
  and the work Claude agent covers the Spec and Standards axes alone: both lenses are on each `work`
  entry of the routing file) and judges the escalations (`gates-escalation` and
  `ticket-review-escalation`, `opus`, with their manual checks).
* **Codex** (`gpt-5.5`) is the writer of each slice, the two repairs, the self-review, the test critics
  (`test-critic-N`, the `pf-diff-reviewer` profile) and the review stage's orchestrator and revise node.
* **A grant-check node precedes every Codex node**, twelve in all. Each runs the pinned
  `gold-band-run-start grant-check` wrapper for the node it guards (`--node <name>`), and the guarded
  node has no other way in. A refused check (no grant, a lapsed one, or every `max_starts` start taken)
  takes the node's failure edge to the `grant-refused` manual check, so the run pauses (exit 2) before
  Codex starts. A run can reserve up to twelve Codex starts, so a grant needs `max_starts` of 12.

The gate, copier, verdict and grant-check nodes bind stub agent ids (`gate-*`,
`copy-*`, `verdict-*`, `grant-*`) as in the personal template; the live wiring
that gives them real agents is the live wiring ticket's. A real run on a `nedap`
repository still needs the run-start shim and a live grant for the run
(`workflow-b-work@<host/owner/repo>@<run id>`).

The proof is two layers. `gold-band-proof prove` runs the whole template
against the stub agents: the clean run, both repairs, the escalation pause, a
failing gate (build, tests, lint, coverage, the finding check and the final gates) and `grant-lapse-<node>` for
each of the eight grant checks on the clean path (the node fails and the run pauses at `grant-refused`). `tests/test_gold_band_workflow_b_work.py`
runs each Codex node's stretch of the real template with the real wrapper and
the real gateway bundle over a fake grants file in a throw-away configuration
outside any repository: with no grant the run pauses before Codex and no start is
reserved; with a live grant Codex runs after one reservation; with no start left
the next Codex node is refused. The workflow lint runs over a `*-work.json`
template as the work instance (`gold-band`), which may bind Codex.

**Reach levels.** Workflows bind agent ids by `provider`, and
`registry/reach-levels.yaml` holds no id yet (`ids: {}`), so this template needs
no reach cell. The support record has no `codex` agent either: Codex's reach
levels are unverified and are the Codex reach ticket's, so a work node that must
run at a held reach level (not just by the work instance's own limits) waits for it.

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
runner; `scripts/crates/critique` proves each through
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
   and the ticket review (`/review-tickets`), each stage wrapped in the generated critique
   block (accept, revise once with fresh critics, then escalate; see "The generated critique
   block") so a finding changes the artifact before the next stage starts. The review nodes are the
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

The spec and ticket drafts are written by Vibe GLM (Zhipu), the family of the
orchestrators that rule on their review (`spec-verdicts`, `tickets-verdicts`, Vibe-study GLM).
The runner leaves the orchestrator's family out of the critics' roster, so no
critic the runner picks is of the writer's family (#796; `workflow-lint --critique`
and the run-start `critic-family` check hold it). Before #796 the drafts were
Mistral and met `pi-vibe-mistral-large` in the roster. No node writes memory and
none binds Codex or Opus; the grill rounds use Vibe GLM. The work-instance variant is
`workflow-c-work.json` (next section); this file is not changed by it.

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

## Workflow C in the work instance

`installer/gold-band/workflows/workflow-c-work.json` (issue #463) is the same
graph bound to the work instance's two agents, `claude-work` and `codex`, and
nothing else. What differs from the personal file:

* The grill rounds, the needs-human checks and the publication check run on
  `claude-work`; the two drafts (`spec-draft`, `tickets-draft`) and the two review
  orchestrators (`spec-verdicts`, `tickets-verdicts`) run on `codex`. The runner
  leaves the orchestrator's family out of the roster, so the work critic is the
  work Claude agent and none is of the drafts' family (#796; before it the drafts
  were `claude-work` and so was a critic). The work instance has two families, so
  there is one critic family for a writer, not two. Shim nodes keep their `gate-`,
  `copy-` agents.
* Each Codex node is reached only through its grant-check node
  (`grant-check-spec-draft`, `grant-check-tickets-draft`, `grant-check-spec-verdicts`,
  `grant-check-tickets-verdicts`, the `gold-band-run-start
  grant-check` shim, see "The grant-check node"). A refused grant takes the
  failure edge to a manual check (`grant-refused`, shared by both)
  that has no way on: the owner asks for a new grant and starts the run again.
  The run-start gate also needs a live grant for the run before any node.
* The critique steps are the stage templates' nodes. Since the 2026-11-26
  amendment to ADR-0007 the critique runner refuses a `nedap` repository
  outright (reason `nedap`): nothing of it is sent to any critic, the work
  critics included, so these steps cannot pass on a `nedap` repository.
* Publication still pauses on `publication-check` for the owner, and only
  after it does the publish gate create GitHub issues, through the pinned
  `ticket-publish` wrapper, in the repository the work repository's own
  `docs/agents/setup.yaml` names.

Its tests (`tests/test_gold_band_workflow_c_work.py`) use a fixture setup file
(`tests/fixtures/gold-band/work-repo`) naming a placeholder repository and a
fake `gh`; no real repository is named and no issue is created. The headless
proof adds `grant-lapse-<node>` for each Codex node (the two drafts and the two
orchestrators), each pausing (exit 2) at its grant manual check.

## Workflow A in the personal instance

`installer/gold-band/workflows/research.json` is Workflow A, deep research
(spec #392, stories 82 to 85; issue #456), for the personal instance. Every
run works in a directory of its own, `~/.local/state/agent-kit/gold-band/research/<run id>/`
(below, `RUN`), and on a branch of its own of the private research repository.
One success chain, no failure edge and no cycle:

1. `gate-research-start`, the run-start shim. A refusal ends the run before any
   model node.
2. `gate-research-papers` and `copy-research-papers`: the gate shim around
   `git -C <research repository> worktree add RUN/papers-worktree -b
   research/<run id> origin/main`, so the merge writes onto the run's own branch,
   never into the owner's checkout.
3. `research-planning` (Claude, Opus): states the question, at most five
   sub-questions, at most six public sources (`id`, https `url`), the theme
   folder and the note's `min_tier`, as `RUN/plan.json`.
4. `gate-research-fetch` and `copy-research-fetch`: the fetch shim,
   `research-layout fetch --plan RUN/plan.json --out RUN/pages --max-sources 6`
   ([RUST.md](RUST.md#research-layout)). It refuses a plan over the width limit
   or naming a non-public url, and stores each page's text with its SHA-256 in
   `RUN/pages/fetch-log.json`.
5. `research-fetchers`, one `ai-dynamic` node: the fetchers, one per stored
   page, each reading only its own page and the plan and writing one note of
   verbatim quotes to `RUN/staging/<source id>.md`. Its limits are explicit:
   `maxFanout` 6 (the width, the same number the fetch shim enforces),
   `maxParallel` 3, `maxDepth` 2 and `maxGroupDepth` 1 (the depth: one fan-out
   group, its workers, no deeper), `allowNestedDynamic` false,
   `maxWorkflowInvocations` 1 and `maxDynamicNodes` 9 (the bootstrap, six
   fetchers, the merge and the acceptance). A fetcher fetches nothing and
   follows no link, so the depth of the web search is the plan's one level.
6. `gate-research-adopt` and `copy-research-adopt`: `research-layout adopt`
   copies every stored page byte for byte into the plan's theme folder on the
   run's branch, as `<theme>/<id>.txt`. A model never retypes a page: in the
   first live run (below) a merge worker without it kept two of six pages,
   because copying a page through a write tool is slow and error-prone.
7. `research-merge` (Claude, Sonnet): writes the result into
   `RUN/papers-worktree/papers` in the papers layout: one catalog entry per
   adopted page with its `url`, the theme's explainer when the folder is new,
   and one note whose every claim quotes a page (`> ` lines, then
   `-- <theme>/<id>.txt`). It never edits an adopted page.
8. `gate-research-index` and `copy-research-index`: `research-layout index` over
   the papers directory, so no model writes a generated index.
9. `gate-research-verify` and `copy-research-verify`: the read-only verifier,
   `research-layout verify --papers RUN/papers-worktree/papers --fetched RUN/pages
   --base origin/main`. It fails when the layout breaks a rule, when a quote is
   not on its page (`quote-not-found`), when a page differs from the page as
   fetched (`page-not-as-fetched`), or when a note the run added quotes a page
   the fetch did not store, quotes nothing, or quotes under 8 characters. So a
   quote that is not on its fetched page fails verification and ends the run.
10. `research-acceptance` (Claude, Opus): judges the plan against what the run
   added on its branch, and answers `ok`; `false` ends the run in failure. The
   only edge to `$end` leaves this node.

The branch is left for the owner to review and push; no node holds a GitHub
credential.

### Which nodes may be Pi, and why

The research repository is private, so a node that reads or writes it binds an
Anthropic model; only nodes that see nothing but public material may run Pi
over LiteLLM (agent-kit#737):

* **The fetchers are Pi** (`pi`, `litellm-vibe-study/deepseek-v4.1-flash`; the
  bootstrap and the node's own merge and acceptance `litellm-vibe/mistral-medium-3.5`).
  They read the plan and the public pages in the run directory, which holds
  nothing from the research repository, and write only `RUN/staging/`. Pi has
  no web tool (`registry/reach-levels.yaml`: `read-web` is unsupported), which
  is why a pinned wrapper fetches the pages and the fetchers only read them. The
  plan is derived from the owner's question, so the question reaches Pi too: it
  must be fit for a public model, and the research skill says so.
* **The verifier is not a model.** It reads the private branch, so Pi is ruled
  out, and a quote check is exact: a model asked whether a quote is on a page
  can say yes when it is not. The verify gate's own agent and the copier only
  run the wrapper and read its report.
* **The merge is Claude.** It reads the research repository's layout and writes
  into it.
* **Planning and acceptance are Claude Opus**, the two judgment nodes the spec
  gives Opus (their ids end in `-planning` and `-acceptance`, as the workflow
  lint requires). Neither binds a memory-writing agent.
* The gate and copier agent ids (`gate-research-*`, `copy-research-*`) are
  bound when the workflow is wired live; Pi cannot hold the
  `run-one-command` level they need, so they run on Claude.

### What Gold Band does with the fan-out (verified headless, v0.19.1)

* An ai-dynamic node may list an agent id once in `availableAgents`: a second
  `pi` entry (to give the fetchers two models) is refused at load with
  `available agent pi is duplicated`.
* A dynamic fan-out must create at least two nodes: a bootstrap proposal with
  one worker is rejected (`dynamic.fanout.nodes.too-few`) and the run pauses
  as blocked. The proof's stub therefore fans a single worker agent out twice.
* Whether Gold Band refuses a proposal over `maxFanout` was not proved here; the
  fetch shim refuses a plan over six sources before the fan-out, so the width
  holds either way.

### What is proved and what is not

The headless proof (`gold-band-proof prove`) passes on Linux against v0.19.1:
`clean` ends completed and in success (exit 0), and each of
`gate-fail-research-start`, `-papers`, `-fetch`, `-index` and `-verify` ends
completed and in failure (exit 1), so a refused verifier never reaches the
acceptance. Since the graph is one chain whose only edge to `$end` leaves the
acceptance, a clean success ran every node in order.
`tests/test_gold_band_research_workflow.py` checks the file itself (the order,
the limits, the bindings and the privacy split). The verifier's rules are the
`research-layout` unit tests, and `tests/verify_gate.rs` in that crate runs the
built verifier through `gold-band-gate run` and `verify`: a quote on its fetched
page passes both, a quote that is not fails the gate and the copier refuses its
report. Not proved: that the gate agents run the pinned wrappers with these
arguments (the live wiring), and that real fetchers quote well.

### The first live run (Linux, 2026-11-14)

The gate agents are not wired live yet, so the nodes were run in order by hand,
each with the agent and reach level the workflow binds, on the public question
"how should a client retry failed calls to a remote service (exponential
backoff, jitter, retry limits)?":

* planning: `claude -p --model opus` at the `read` level wrote a plan of five
  sub-questions and six sources (Google SRE book, two chapters; AWS Builders'
  Library; AWS Architecture Blog; RFC 9110; gRPC proposal A6);
* `research-layout fetch` through `gold-band-gate run` stored all six pages
  (12 to 453 KB of text each), and the copier accepted the report;
* six Pi fetchers (`pi -p --provider litellm-vibe-study --model
  deepseek-v4.1-flash`, read-only tools, three at a time) each wrote a note
  from its own page;
* the papers gate made `research/run-20261114` in a worktree of the research
  repository, `adopt` copied the six pages, and `claude -p --model sonnet` at
  the `write-scoped` level wrote six catalog entries, the theme explainer and a
  note with 34 quotes;
* `index`, then `verify` through the gate: `research-layout: ok`, copier exit 0.
  The same note with one word inserted into one quote failed with
  `quote-not-found` (exit 1);
* acceptance: `claude -p --model opus` answered `ok: true`, naming three cut
  quotes and the open points (no sourced base delay or multiplier, thin latency
  trade-offs).

A first merge without the adopt gate kept only two of the six pages, and the
acceptance node refused it (`ok: false`, sources fetched but not used); the
adopt gate was added for that reason. The result is committed on the local
`research/run-20261114` branch of the research repository, not pushed.

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

## Workflows choose models by tier (#914)

A workflow file names a LiteLLM tier where a tier fits, and a model where the model is the point. A tier
binding is the agent `pi` with the model `litellm-<route>/<tier>`, where `litellm-<route>` is the Pi provider of
the tier's profile in the model gateway's defaults (`tier-free` is `pi-free:tier-free`, so
`litellm-free/tier-free`; `tier-vibe` is `litellm-tier-vibe/tier-vibe`): the same profiles `model-gateway run
--tier` runs, rendered from `registry/litellm-tiers.json`. LiteLLM walks the tier's chain, so the template no
longer says which model answers.

* **Where a tier fits.** The to-spec and to-tickets question rounds, their checks and their escalation check,
  and the `needs-human` node of each critique template run on `tier-vibe`. The research fetchers (and the
  ai-dynamic node's bootstrap and acceptance) name `tier-free` first and `tier-vibe` second, since they read
  public pages only. Claude stays the planner, the merge and the judge (Opus, Sonnet), and Codex and
  Antigravity keep their pins.
* **Where a model stays.** A node the critique rules reason about by family stays concrete: the writers
  (`*-write`, `*-draft`), the stage orchestrators (`*-verdicts`, whose family leaves the critic roster) and the
  critics themselves. A tier hides the family, so binding one there would turn `critic-required` and
  `critic-family` off silently. Workflow B and C keep their pinned test author, writer and critic for the same
  reason, and `critique-spec-stage` keeps its Vibe and Antigravity roster.
* **The lint resolves every tier.** `workflow-lint` refuses a Pi binding whose model is named `tier-*` when the
  tier is not in the snapshot or the provider is not the tier's (`unknown-tier`).
* **The run-start shim resolves them too.** A tier is a chain of models of several families, so the family check
  takes the families of the chain's steps: each one must be allowed by the repository's entry, and a step with
  no family cannot be vouched for by any entry, so a repository with an entry that names
  fewer families than the chain holds is refused until its entry is extended with `allow`. A repository with no
  entry takes the personal default, and a `nedap` repository allows no tier (only `claude-work` and `codex`).
* **The ledger shows the tier.** Gold Band runs outside the gateway's ledger (ADR-0014), so a passing run-start
  check writes the run's own ledger, `ledger.jsonl` in the report directory: one line per model node with
  `run_id`, `attempt`, `workflow`, `node`, `agent`, `model` and `tier` (`null` for a model picked by hand). The
  line records the binding, not the answer: the model that served a tier call is on the gateway's ledger line
  of a `model-gateway run`, which Gold Band nodes do not make.

## What a bare Gold Band node does not guarantee

A Gold Band node that asks an agent to critique, with no runner behind it,
gives up the guarantees below. The critique stage templates keep them by making
the runner the executor of each stage (see
[The critique stage templates](#the-critique-stage-templates)); `critique-spec-stage.json`
sends the spec to critics directly and loses all of them, which is why it is
for personal repositories only. Each row names what the runner does, the
Rust source that does it and the tests beside it, and what a bare node does
instead.

| Guarantee the runner enforces | Where | A bare node |
| :- | :- | :- |
| **A critic cannot write.** Three layers: the critic's tool configuration, a private copy of the packet per critic, and a tripwire that compares the packet, the run's frozen packet and the repository's working tree before and after, restores the packet, and voids the critic (and any critic running beside it, when the cause cannot be told apart) on a change | `tripwire.rs`, `tests/tripwire.rs` | Has the agent's own permission prompt and the OS write limit of its reach level; nothing notices or voids a write, and nothing restores the files |
| **The packet is frozen and verified.** `prepare` records each packet file's sha256 in the run manifest; a packet that no longer matches is `invalid-run` | `manifest.rs`, `tests/manifest.rs` | Reads whatever is on disk when the node starts |
| **Search stays inside the packet.** The critic's search server answers from the packet only, with capped answers; a search outside it returns nothing | `search_server.rs`, `search_wiring.rs` | Uses the agent's own search and read tools over the whole checkout |
| **A critic is stopped at its time limit and price cap.** Each critic has its own limit, enforced by the CLI flag and an outer timeout; Antigravity's headless timeout exits 0 and reports success, so the runner counts an empty answer as a failure | `critic_run.rs`, `docs/critique-pipeline/integration.md` | Runs until the node's own limits (`max_attempts`, `max_rounds`) stop it; a timed-out agent can read as a pass |
| **A repository must have opted in.** `prepare` refuses a repository with no allowlist entry (`not-opted-in`), exit 4 under `--strict`; the run-start shim applies the personal default instead | `config.rs`, `manifest.rs` | The run-start shim only; with no entry the personal default applies |
| **Critics come from the allowed families, and the refuter is another family.** The roster is routed per stage with the family allowlist's code and history permissions | `roster.rs`, `config.rs` | Binds the agent id the workflow names, checked once at run start |
| **Every finding is checked against the packet.** A finding must quote evidence that is in the frozen packet, at the line it cites; findings that fail the mechanical check are rejected and counted, duplicates are merged, severities can be fixed by a declared check | `evidence.rs`, `findings.rs` | Takes the agent's text as it comes |
| **Findings are refuted before they are recorded.** `refute` asks another critic to object or downgrade; `record` writes the report only from what survived | `refute_round.rs`, `record_units.rs` | No second look; the node's answer is the result |
| **Depth follows the change.** Risk and size signals from the artifact decide how deep the stage goes | `depth.rs` | One fixed shape per workflow |
| **Daily quotas are counted.** Antigravity's launches per day are counted by the runner (it reports no quota headless) and past the reserve only the reserved stages may use it | `quota.rs`, `flow_budget.rs` | Not counted; the per-key EUR budgets in LiteLLM are the only cap, and Antigravity's quota is unwatched |
| **A refusal ends the run.** `--strict` turns `"status": "refused"` into exit 4, which the gate shim turns into a failed node | `cli.rs` | A refusal is text in an answer, which reads as a pass |
| **Spend is in the gateway ledger.** Critique runs through the model gateway, so its per-workflow and per-day caps count it | `run_ledger.rs` | Gold Band runs outside the gateway (ADR-0014); the ledger never sees it, and only the per-key budgets and, for Codex, a grant (counted in node starts) hold |

What a bare node keeps, because the run-start shim checks it on every run, and
the instance enforces it: the repository rule, the family allowlist and code
permission of every agent id the workflow binds, and in the work instance a
live grant for each Codex node (ADR-0011, ADR-0014).

Proof status: the runner's behaviours above are proved by the Rust tests
of the modules named in the table (`scripts/crates/critique` and
`scripts/crates/model-gateway`), which replay frozen goldens through the same
`--strict` command line a gate node runs. The headless proof (`gold-band-proof prove`) checks each
template's sequencing against stub agents only. That a live Gold Band run wires
its gate agents to the pinned wrappers is not yet proved (see
[What is proved and what is not](#what-is-proved-and-what-is-not)).

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

## Quiet settings and the updater

Setup replaces `settings.json` atomically (a staged 0600 file renamed over the
old one, one `.bak` kept, the data directory 0700) and keeps every key it does
not declare. It also turns the app's outbound components off, in the pinned
app's `SettingsConfig` (`src/config/mod.rs`):

* `desktopMetricsEnabled` (line 881) is set to false.
* `desktopMulticaEnabled`, the task-board integration (line 892), is set to false.
* Each `imIntegrations.channels[].enabled`, the chat connectors (line 888,
  `src/im/model.rs:485`), is set to false.

A key that is absent stays absent; the app defaults each to off.

The updater is held off by pointing it nowhere. The app has no updater
switch; its only setting is `desktopUpdaterUrlOverride` (`src/config/mod.rs:877`,
an https URL). Left alone, the updater polls GitHub's `latest.json` and installs
critical updates silently, replacing the binary the builder made from the pin
(it once replaced it with a v0.20.0
AppImage). The kit owns this key: setup sets it to
`GOLD_BAND_UPDATER_DISABLED_URL` (`https://127.0.0.1:9/gold-band-updates-disabled.json`
in `scripts/converge.py`), whatever it held, so updates arrive only by moving the pin.

One component cannot be switched off from settings:

* **Feedback.** The app has no feedback setting, so there is nothing to write.
