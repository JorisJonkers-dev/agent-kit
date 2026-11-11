# Model gateway: integration

What agent-kit#294 settled, for the tickets that build on it. The words in
bold in [CONTEXT.md](../../CONTEXT.md) under "Model gateway" mean what they
say there; [ADR-0007](../adr/0007-one-model-gateway-for-every-delegation.md)
records why there is one gateway,
[ADR-0008](../adr/0008-agents-write-grants-after-asking.md) why an agent writes
its own grants, and
[ADR-0010](../adr/0010-vibe-lists-vibe-critics-stay-pinned.md) why the
gateway probes Vibe for its aliases instead of reading `config.toml`, and why
the vibe critic agents keep their pinned models.

## Where the files are

| Path | What it is |
| --- | --- |
| `council/ts/src/contexts/model-gateway/` | The bounded context: `domain/`, `application/`, `adapters/`, layered as `contexts/build-tracker/` is. |
| `council/ts/src/model-gateway-cli/index.ts` | The CLI entry, exporting `runModelGatewayCli(argv, runtime)` and `defaultModelGatewayRuntime()`, as `src/build-tracker-cli/index.ts` does. |
| `skills/workstation/model-gateway/model-gateway.mjs` | The bundle `council/ts/scripts/build.mjs` writes (mode 0755). Pinned in `manifest.yaml`'s `model_gateway.files`. |
| `skills/workstation/model-gateway/defaults.json` | The shipped defaults. |
| `skills/workstation/model-gateway/defaults.schema.json` | The schema `defaults.json` is validated against by `scripts/validate_manifest.py`. |
| `skills/workstation/model-gateway/SKILL.md` | The skill's text. Setup installs the whole folder in every agent home. |

`model_gateway.files` pins the bundle and both JSON files, and leaves
`SKILL.md` out, exactly as `build_tracker.files` does. `pnpm run build` in
`council/ts` rebuilds the bundle: update its sha256 in the same commit.

## The command

`model-gateway <subcommand> --json '<object>'` or `model-gateway <subcommand>
--input <file.json>`. It prints exactly one JSON object on stdout and exits 0;
on an error it prints `{"status":"error","code":"<slug>","message":"<one
line>"}` and exits 1. #294 implements only `models`; any other subcommand is
`unknown-command`.

`models` takes `{}` today (later tickets add `repo`) and returns one entry per
client model with `client`, `model`, `family`, `cost_class`, `tier`, `notes`
and `source`, plus a `missing` list naming each client that is not installed
and why. Models are sorted by client, then model id.

## Choices later tickets rely on

- **`source` is `live`, `config` or `declared`.** #294 produces only
  `declared`, for Codex and Claude Code. Vibe and Antigravity listing is #310
  and Hermes is #297; until then those clients list no models and are not
  reported missing, so a `missing` entry never means "not built yet".
- **A client is installed when its `binary` resolves on `PATH`**, through the
  process port, so tests pass a fake and nothing launches a real client.
- **The shipped defaults are read from the folder the running bundle sits
  in** (`dirname(import.meta.url)`), and the runtime takes that path as an
  injectable value so a test passes a temp file.
- **The machine override is `$XDG_CONFIG_HOME/agent-kit/model-gateway.json`,
  else `~/.config/agent-kit/model-gateway.json`.** A missing file means no
  overrides. It takes the same top-level keys as `defaults.json`, every one
  optional, and merges over it: `clients.<id>.<field>`, `models` entries
  matched on `client` and `model` (or added), `settings.<key>` with a deep
  merge for `metered_personal_caps`, and `disabled`, a list of
  `"<client>:<model>"` keys left out of `models`. An unreadable or invalid
  override is `invalid-config` and names the file.
- **The enums are closed.** `cost_class` is `subsidised`,
  `metered-personal` or `metered-employer`; `tier` is `trivial`, `routine`,
  `hard` or `frontier`; `model_source` is `live`, `config` or `declared`. A
  new value is a schema change, not a data change.
- **`vibe` and `agy` model entries are catalogue entries.** Their tier and
  notes are what #310 joins onto what those clients list; #294 lists only the
  entries whose client's `model_source` is `declared`.
- **The defaults carry the caps and thresholds** a delegation is held to:
  `plan_warning_ratio`, `summary_max_chars`, `max_parallel_per_client`,
  `availability_window_minutes` and `metered_personal_caps`
  (`per_workflow_usd` 2, `per_day_usd` 5).
