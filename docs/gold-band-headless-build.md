# Building the headless Gold Band binary (#347)

The Gold Band workflow tests run a headless `gold-band` binary that the
environment supplies. It is built from the audited commit
`987fc3e298b15d725a7f0e9b609b0f77dba63a74` (tag v0.19.1), through the crate
gate, after `crate-age` has checked Gold Band's own lockfile. Nothing here
changes Gold Band's Rust.

## Steps

```bash
git clone https://github.com/diodeme/Gold-Band gold-band-src
cd gold-band-src && git checkout 987fc3e

# 1. Gold Band ships a project cargo config that replaces crates.io with the
#    rsproxy.cn mirror. That would bypass the crate gate, so move it aside.
mv .cargo/config.toml .cargo/config.toml.upstream-rsproxy

# 2. The age check over Gold Band's lockfile (crate-age is built from the
#    standards repo, scripts/crates/crate-age).
crate-age --root .

# 3. The headless binary only: the root package, not the Tauri app.
cargo build --release -p gold-band --bin gold-band
./target/release/gold-band --help
```

## Result of the build on 2026-11-08

- **Age check:** `crate-age` proved 766 of the 768 locked registry versions at
  least 7 days old and exited 2 for the other two, which have no publish date
  at all: `gold-band-desktop 0.19.0` (Gold Band's own Tauri crate, a path
  dependency) and `tao 0.35.3` (a git patch pinned to tauri-apps/tao rev
  `c704261c`). No crate is younger than 7 days, so nothing went to the owner
  as too young. The git patch is the one unproven dependency of the build.
- **Lockfile:** at the audited commit the lockfile is stale for Gold Band's own
  two crates (it records `0.19.0`, their manifests say `0.19.1`), so the build
  runs without `--locked`. The resulting lockfile diff is exactly those two
  version lines; no third-party version changes.
- **Crate gate:** the build exposed a gate bug, fixed in the standards repo
  (branch `fix/crate-gate-dl-index`): every crate whose name starts with `dl`
  (here `dlopen2`) got a 404, because the gate routed every `/dl/` path as a
  download. A second gate limit remains: a download over 10 MiB is refused
  with 502 (seen for `windows 0.56.0` during `cargo metadata`); the macOS
  build does not need that crate.
- **Binary:** `target/release/gold-band`, 25,514,016 bytes, sha256
  `16696be5db938bd2efbbeeb22f078b2ad970b7fa86f4b9e574eaba35415af034`.
  `--help` lists `run` and `continue`.

Point the headless tests at the binary with `GOLD_BAND_BIN=<path>`; without it
they are skipped with that reason.

## Running the spec-stage workflow proof (#348)

The critique spec stage ships as a Gold Band workflow template,
`installer/gold-band/workflows/critique-spec-stage.json`, and its proof runs
the template headless against the stub ACP agent
`tests/fixtures/gold_band_stub_acp.py` — no model, no network: the test's
tmp home declares agents `vibe` and `agy` with the stub as their adapter
command, and the stub answers every `session/prompt` with a fenced JSON
findings document valid against the finding schema.

```bash
# Without a binary, only the template tests run:
uv run --no-project --with pytest --with pyyaml --with tomlkit --with jsonschema \
  python -m pytest -q tests/test_gold_band_spec_workflow.py

# The headless proof needs the binary:
GOLD_BAND_BIN=/path/to/gold-band \
  uv run --no-project --with pytest --with pyyaml --with tomlkit --with jsonschema \
  python -m pytest -q tests/test_gold_band_spec_workflow.py
```

What the proof asserts:

- A clean stub run (`STUB_ACP_VERDICT` unset or empty) walks both critics
  and the merge node, whose merged findings are empty, so the success
  condition routes to `$end` and `gold-band run` exits **0**.
- `STUB_ACP_VERDICT=needs-human` makes every findings document carry one
  needs-human case, the merge node's success condition fails, and the run
  routes to the manual-check node and pauses there: exit **2**, the verdict
  resolved out of band in the app or over IM.
- Every findings artifact the run wrote under the tmp `GOLD_BAND_HOME`
  validates against `skills/workstation/critique/schemas/finding.schema.json`
  (with `jsonschema` when the run provides it, else the schema's own
  required keys).
- The merge node's prompt carries both critics' artifacts: Gold Band hands a
  worker every earlier node of its round as predecessor context, not only
  the last one.
- Without `GOLD_BAND_BIN`, or without `cargo` on PATH (the proof renders the role
  profiles with the `gold-band-roles` crate), the binary tests skip with that reason; the
  template tests (the roster binding, no codex, the personal-repositories-only
  description, and workflow-lint where cargo is on PATH) run everywhere,
  including the required `python` CI job, which collects every test file.
  The optional `gold-band-headless` job runs this file alone.

Facts the binary taught, each one a refusal or a silent drop before the
proof passed (Gold Band v0.19.1, 987fc3e):

- A `settings.json` without `settingsSchemaVersion` (12 at this commit) goes
  through the version-0 migration, which rejects any agent id that is not a
  built-in; the headless runner then loads defaults with no agents at all and
  says only ``agent `agy` is not configured``.
- A worker `output.schema` must be Gold Band's simplified shape (field name
  to type name, one-element arrays), never JSON Schema: any of `type`,
  `properties`, `required`, `additionalProperties` or `items` at its top is
  refused. The template's shape is the finding schema's required fields; the
  full schema is checked on the artifacts.
- Gold Band appends `.json` to a json artifact's name, so the template names
  its artifacts without the extension.
- Every worker, the manual-check node included, needs a profile, and user
  profiles are read from `~/.gold-band/context/profiles`, not
  `presets/profiles`.
- A model binding is accepted only when the agent's `session/new` answer
  offers that model in a `configOptions` entry of category `model`.

If the binary shows a fact in this proof wrong — the DSL field the template
uses, the exit codes, where the artifacts land — follow the binary, fix the
template or the stub, and record what differed on the ticket.
