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
| `scripts/crates/model-gateway/` | The Rust gateway. The `model-gateway` command on PATH (`installer/model-gateway`) runs the binary setup builds. |
| `skills/workstation/model-gateway/defaults.json` | The shipped defaults. |
| `skills/workstation/model-gateway/defaults.schema.json` | The schema `defaults.json` is validated against by `scripts/validate_manifest.py`. |
| `skills/workstation/model-gateway/SKILL.md` | The skill's text. Setup installs the whole folder in every agent home. |

`model_gateway.files` pins both JSON files and leaves `SKILL.md` out, exactly as
`build_tracker.files` does.

## The command

`model-gateway <subcommand> --json '<object>'` or `model-gateway <subcommand>
--input <file.json>`; `models` and `catalog` also run with no arguments. Every
subcommand prints exactly one JSON object on stdout and exits 0; on an error it
prints `{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.
`catalog` is the one exception to the JSON rule: it prints a plain table by
default and takes flags instead of an object (`--json` prints the object).

The subcommands, as `skills/workstation/model-gateway/SKILL.md` describes each:

| Subcommand | What it does |
| --- | --- |
| `models` | The client models this machine can use now for a repository (`repo`), with the data policy, the profiles and a `skipped` list; `--free` or `{"free": true}` keeps only free models. |
| `catalog` | One row per canonical model the proxy serves: modality, family, tier, context length, free flag, cost, every deployment with its route, key and probe status, and the clients and profiles that can run it. Filters `--route`, `--free`, `--status`, `--modality`. |
| `usage` | What each client can do now and what it has spent. |
| `run` | One delegation to one client model, read-only or writing. |
| `batch` | A whole plan of delegations in one call. |
| `workflow start`, `end`, `list` | The life of the piece of work a delegation belongs to. |
| `grant write`, `reserve`, `list` | The owner's permission to spend their employer's money. |
| `worktrees clean` | Removes the kept worktrees of writing runs. |

Any other subcommand is `unknown-command`.

`models` takes `repo` and `free`, both optional (`models` alone is `{}`), and
returns one entry per permitted client model with `client`, `model`, `family`,
`cost_class`, `tier`, `notes`, `source`, `may_send`, `inline_only`, `cost` (the
class and the input and output price per million tokens, `0` for a free model and
`"unknown"` where nobody states one) and `providers` (each provider serving it
in fallback order, with the route that reaches it), plus `missing` (each client
the gateway cannot use and why), `skipped` (each model it cannot offer and why),
`policy` and `profiles`. Models are sorted by client, then model id.

## Choices later tickets rely on

- **`source` is `live`, `config` or `declared`.** `live` is the client's own
  listing: Antigravity's `agy models`, OpenCode's `opencode models`, Vibe's
  answer to a probe (ADR-0010) and, for a Pi client, the models of its
  `litellm-<route>` provider in Pi's `models.json`. `config` is Hermes, which
  offers the one model its configuration names as its default. `declared` is
  the shipped catalogue, for Codex and Claude Code. A client whose listing
  fails is named in `missing` with the reason, so a `missing` entry never means
  "not built yet".
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
- **OpenRouter runs through one client.** `pi-openrouter` (and `pi-free`, for
  the `free` route) have `data_scope: public` and run models priced at zero only:
  `models` skips a billed one with the reason, `run` refuses it with
  `openrouter-billed` (`free-only-refused` on `pi-free`), and every other client
  still refuses `openrouter` as a provider or an `openrouter/` model id.
- **The enums are closed.** `cost_class` is `subsidised`,
  `metered-personal` or `metered-employer` in files, and `models` also prints
  `free` for a model the live LiteLLM catalogue marks free (for a Pi model, a group whose every deployment is free; Pi's zero `cost` stands in only where the catalogue cannot say); `tier` is `trivial`, `routine`,
  `hard` or `frontier`; `model_source` is `live`, `config` or `declared`. A
  new value is a schema change, not a data change.
- **`vibe` and `agy` model entries are catalogue entries.** Their tier and
  notes are joined onto what those clients list; a listed model the catalogue
  does not know takes its family from its id, or from the live LiteLLM
  catalogue (see SKILL.md, "The live LiteLLM catalogue"), and an entry the
  client no longer lists is named in `skipped`.
- **The defaults carry the caps and thresholds** a delegation is held to:
  `plan_warning_ratio`, `summary_max_chars`, `max_parallel_per_client`,
  `availability_window_minutes` and `metered_personal_caps`
  (`per_workflow_usd` 2, `per_day_usd` 5).

## Family binding and stated escalation (#494, #535)

- **A delegation is bound to the family of the model it names.** `run` always
  names a client and a model, so it always names a family (the `family` of
  that catalogue entry). The retry, the climb and the fall-through stay inside
  that family: only a model of it ever answers. When none can, the delegation
  ends `failed` with the cause, or `unavailable` when no client model of the
  cost class can start, and that message says the family is binding. It is
  never answered by another family without being asked to.
- **`family_binding` is the input that lifts it.** It defaults to `true`;
  `false` lets the gateway move to a model of another family (still never into
  a costlier cost class by itself). A value that is not a boolean is refused.
- **An escalation across families is stated.** The summary of the answering
  run opens with `escalated from <family> (<client>: <cause>) to <family>`,
  naming both families and the cause, before the summary the client wrote. A
  batch report carries the same line in the summary of the delegation that
  escalated. A failed cross-family delegation therefore cannot be read as a
  second opinion from the family that was asked for.
- **Where it is implemented.** The Rust gateway
  (`scripts/crates/model-gateway/src/escalation.rs`, `bind_family` and
  `escalation_notice`, and `escalation_run.rs` for the `unavailable` message)
  and the node runner print the same words.

## Delegation profiles (#773, #774)

- **One concept owns the models and the tool scope.** `profiles` in
  `defaults.json` maps a name to `description`, `models` (`<client>:<model>`
  keys of the catalogue), `tools` (`mcp`, `plugins`, `skills`, each a list or
  `any`; a kind left out is deny-all) and the optional `pin`, `ladder`,
  `distinct_families`, `exclude_writer_family`, `max_spend_usd` and
  `unenforced_clients`. The source
  is `registry/delegation-profiles.yaml` with its schema beside it;
  `python scripts/render_registry.py --write` renders it into `defaults.json`
  and `--check` fails when the section is stale. The node runner and the Rust
  gateway read the same section and print the same bytes.
- **`models` lists the profiles** after `skipped`, each with the models it
  leaves for the caller's repository (the data policy intersected with what is
  usable now, cheapest first) or an `error` saying why it leaves none.
- **`run` takes `profile` and `narrow`.** A request may narrow a profile (fewer
  models, fewer tools, a lower spend cap, the writer's family left out), never
  widen it. The refusals are `unknown-profile` and `profile-refused`, each with
  its reason: no allowed model for the repository (never a fallback), a model
  outside the profile, a widening `narrow`, and a tool scope the client cannot
  enforce (see "Tool scope on Pi" below); every other client is refused a
  profile that restricts tools. The data policy runs first, so a model the
  repository does not permit is still `policy-refused`.
- **Roles are profiles, and `--harness` chooses where one runs (#855).**
  `research`, `implement`, `critique`, `red-team`, `refute` and `triage` are
  profiles in the same registry, each deny-all on tools, capped per run and cut
  by the data policy (a repository never reaches `pi-public`, `pi-openrouter` or
  `pi-free`). `run --profile ROLE --harness H` (or `profile` and `harness` in the
  input) replaces `client` and `model`: the gateway keeps the profile's models
  for the repository that are on the harness (`pi` is every Pi route, `pi-<route>`
  one, `claude`, `codex`, `agy` and `vibe` themselves) and runs the cheapest. A
  harness with none (hermes has no model configured) is `profile-refused` with
  the models that are left. Only Pi enforces a scope: a profile lists in
  `unenforced_clients` the other clients it accepts on the strength of their own
  read-only launch, and a client it does not list is refused as before.
  `scripts/crates/model-gateway/src/harness.rs` holds the choice; the
  `run-roles` scenario holds the goldens.
- **Tiers are profiles too, rendered from litellm (#905).** A LiteLLM tier
  (`tier-free`, `tier-vibe`, `tier-go-cheap`, `tier-strong-open`, `tier-review`) is
  a model name the proxy serves, with a cost ceiling and an ordered chain that
  litellm owns (`catalog/routing.json`, key `tier_groups`). agent-kit holds a
  snapshot, `registry/litellm-tiers.json` (`python3 scripts/litellm_tiers.py
  --write --ref REF` refreshes it, `--check --upstream REF` finds drift), and
  `render_registry.py` renders one profile per tier into `defaults.json`, plus
  the key list and Pi provider of each tier's route, refusing a route whose models
  differ from the tier's `key_models`. `run --tier T` (or `tier` in the input)
  is `--profile T --harness pi`: Pi sends the tier's name as the model, with the
  key of the route that holds it (`tier-free` uses the `free` key). An unknown
  tier is `unknown-tier`, never a default, and a tier is not combined with a
  profile, harness, client or model. `tier-strong-open` is an escalation tier
  with no workstation key, so a run on it is refused until a key is provisioned.
  The `run-tier` scenario holds the goldens.
- **`matrix` proves them (#856).** `model-gateway matrix` runs each role on each
  harness once and judges the cells against `docs/model-gateway/harness-matrix.json`;
  the stub proof is the `matrix-stub` scenario in CI, `{"live": true}` is the owner's
  machine, and `docs/model-gateway/harness-matrix.md` is the table.
- **A failing delegation stays inside its profile**: the retry, the climb and
  the fall-through are cut to the profile's models.
- **Not yet:** Gold Band agent ids per profile (#784), and spend enforcement
  from LiteLLM key spend (#788).

### Tool scope on Pi (#773)

Implemented in the Rust gateway, which is the only gateway now that the node
runner is retired.

A profile's `tools` is the scope a run holds, narrowed by the request. Each of
`mcp`, `plugins` and `skills` is a list of names, or left out for deny-all:

```yaml
tools:
  skills: [lint, review-checklist]   # folders with a SKILL.md
  plugins: [hello]                   # Pi extensions
  # mcp is left out: none
```

For a Pi run the gateway builds a throwaway agent directory (the launch's own
`PI_CODING_AGENT_DIR`, removed when the run ends, since it holds the route's
key). It holds the one-provider `models.json`, an empty `settings.json`, and a
copy of exactly the skills and plugins the effective scope lists, and nothing
else of the operator's. Pi is started through `agent-launcher confine` with
`--no-extensions --no-skills --no-mcp --no-prompt-templates --no-themes
--no-context-files --no-approve` as before, and with `--skill <copy>` and
`-e <copy>` for each listed item, which are the only ones that load (checked
against Pi 1.5.0: with these flags the session has no command or skill other
than the listed ones, although `~/.pi/agent/skills`, `~/.agents/skills` and the
home's `mcp.json` exist).

- **Skills** are found as `<home agent dir>/skills/<name>/SKILL.md`, else
  `~/.agents/skills/<name>/SKILL.md`; the whole folder is copied.
- **Plugins** are Pi extensions, found as `<home agent dir>/extensions/<name>`,
  `<name>.ts` or `<name>.js`, and loaded with `-e`.
- **Refused, with the reason, as `profile-refused`** (before anything runs):
  - an `mcp` entry. Pi has a built-in MCP client (it reads `mcp.json`), but a
    confined launch reaches only the route's proxy port and starts no other
    program, so a listed server could never connect. `mcp` stays deny-all on Pi
    and `--no-mcp` is always passed;
  - `any` for any kind: the home is built only from named items;
  - an entry that is not a plain name (letters, digits, `.`, `_`, `-`), so a
    path cannot reach out of the home;
  - a skill or plugin that is not in the operator's home (the message names
    the places it looked in).
- Every other client cannot enforce a scope and is refused a profile that
  restricts any kind.
- **Ledger.** A run held to a profile writes `profile` and `tool_scope` (the
  scope it ran with, `{"mcp":[],"plugins":[],"skills":[...]}`, `"any"` for an
  open kind) on its ledger line. `usage` reports totals per client, day and
  workflow, so it does not carry a per-run scope; read it from the ledger.
