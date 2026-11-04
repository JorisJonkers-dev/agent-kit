---
name: model-gateway
description: Holds the model gateway's runner and its shipped defaults; it is not a workflow of its own, since the delegate skill, coming later, drives it.
---

# model-gateway

This folder is what the model gateway runs on. It does nothing on its own: an
agent calls the runner, and the `delegate` skill (a later ticket) is what
teaches an orchestrator to plan with it. The runner answers three questions:
`models`, which client models this machine can use right now; `run`, one
read-only delegation to one of them; and `workflow`, the life of the piece of
work a delegation belongs to.

## What is in here

- `model-gateway.mjs` — the runner. It is built from `council/ts` by
  `npm run build` and pinned in `manifest.yaml`'s `model_gateway.files`, the
  way the critique runner is pinned in `critique.files`. Run it with `node`.
- `defaults.json` — the shipped defaults: the clients on this machine, the
  client models each offers with their family, difficulty tier and capability
  notes, and the caps and thresholds a delegation is held to.
- `defaults.schema.json` — the JSON Schema `defaults.json` is validated
  against. The Python asset validators check the shipped file against it, so a
  broken default fails CI rather than a delegation.

The runner reads `defaults.json` from the folder it sits in, so the two always
travel together.

## The `models` command

`models` lists the client models on this machine that the data policy lets a
delegation from a repository reach. It prints exactly one JSON object on stdout
and exits 0; on an error it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
node model-gateway.mjs models --json '{"repo": "/path/to/repo"}'
node model-gateway.mjs models --input <file.json>
```

`repo` is the path the caller works in; the gateway asks git for that path's
repository root and its remote. The result carries one entry per permitted
client model with its client, model id, family, cost class, difficulty tier,
capability notes, source, `may_send` (`code` and `history`) and `inline_only`,
plus a `missing` list naming each client the gateway cannot use and why, a
`skipped` list naming each model it cannot offer and why, and a `policy` line
saying why the data policy listed what it did. A client counts as installed
when its binary is on `PATH`; one that is not is named in `missing` whatever
its model source. Models are sorted by client, then model id.

`source` says where the list came from:

- `live` — the client's own listing: Antigravity's `agy models` (a
  `Fetching available models...` line, then one `<id>\t<display name>` per
  model) and OpenCode's `opencode models` (one `<provider>/<model>` per line).
  Only the families a client may reach are offered, so Antigravity's Claude and
  GPT-OSS models are left out. An OpenCode model takes the cost class of its
  provider: the shipped defaults count OpenCode Go, the owner's subscription,
  as subsidised, and every other provider as metered-personal.
- `config` — the models the client's own configuration declares. Vibe's are
  listed by the alias `vibe --model` accepts, read from `$VIBE_HOME/config.toml`,
  else `~/.vibe/config.toml`. Hermes has no non-interactive listing — `hermes
  model` is an interactive picker — so the gateway reads `hermes config get
  model --json` and offers the one model that names as its default, with the
  provider it runs on. The other models Hermes could reach are not listed.
- `declared` — the shipped catalogue, for Codex and Claude Code.

A listed model takes its family, tier and notes from the catalogue entry that
matches its id. A model the catalogue does not know is still listed, with its
family worked out from its id, `"tier": null` and no notes: the `delegate`
skill must not pick a null-tier model, because nobody has judged what it
handles well. A model whose family the gateway cannot work out is not offered
at all: it is named in `skipped` with the reason, so it never vanishes
silently, and adding it to the catalogue in `defaults.json` or the machine
override brings it back.

A listing that fails, or a configuration that cannot be read, leaves that
client out and names it in `missing` with the reason; the other clients are
still listed. A Hermes configuration that names no model is such a failure.

Any other subcommand answers `{"status":"error","code":"unknown-command",...}`.

## The `run` command

`run` hands one read-only task to one client model and reports what came of
it. It prints one JSON object on stdout and exits 0, whether the delegation
succeeded or failed; only a refusal is an error, and it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
node model-gateway.mjs run --json '{"workflow":"review","client":"vibe","model":"mistral-large-4-0","mode":"read-only","prompt":"Review src/app.ts","repo":"/path/to/repo"}'
node model-gateway.mjs run --input <file.json>
```

The input is `workflow`, `client`, `model` (an alias `models` listed), `mode`,
`prompt`, and optionally `repo`, `carries` (`{"code":bool,"history":bool}`,
only for an inline run), `time_limit_s` (default 900), `max_price_usd` and
`provider`. `workflow` is required (`code: "invalid-input"`). The gateway runs
`client: "vibe"` and `client: "hermes"` in `mode: "read-only"` only; any other
client or mode is `code: "not-supported-yet"`.

Everything that can refuse the run happens before anything is launched:

- `nested-delegation` — the gateway sets `AGENT_KIT_DELEGATION=1` in every
  delegate's environment and refuses any call made under it, so a delegate
  cannot delegate on.
- `policy-refused` — the data policy (above) for `repo`, naming the rule. A run
  with a `repo` reads that repository, so it counts as carrying code and needs
  `may_send.code`; `carries.history` needs `may_send.history`. A run without a
  `repo` is inline-only: it works in a fresh empty temporary directory, never
  inside a repository, and `carries` may not claim code or history the policy
  does not allow.
- `provider-not-allowed` — a Hermes run must name a `provider`, and it must be
  one this machine lists in `clients.hermes.providers` (empty by default, so
  every Hermes run is refused until the owner names one). `openrouter` is
  refused whatever the configuration says, and so is a model id that starts
  with `openrouter/`: the owner forbade OpenRouter.
- `unproven-read-only` — a read-only run goes in place only on a client model
  with a passing read-only proof. For Vibe the proof is setup's recorded result
  for the critic client whose `model` matches, in critique's routing (the
  shipped `routing.json`, overridden by
  `~/.config/agent-kit/critique-routing.json`):
  `checks["read-only:<client>"].result == "pass"` in
  `~/.local/share/agent-kit/critique/machine-setup.json`. Every other client
  is proven by setup's `read-only:<client>` result for the client itself, which
  setup records for none of them yet, so a Hermes run is refused. A model with
  no passing proof is refused; the throwaway worktree for unproven clients
  comes in a later ticket.
- `unknown-model` — a model the catalogue cannot place, or one this machine
  disabled.

A launched Vibe run is Vibe, non-interactively:

```sh
vibe --prompt-file <run dir>/prompt.md --model <alias> --agent delegate-readonly \
     --trust --workdir <dir> --output json --time-limit <s> [--max-price <usd>] \
     --output-dir <run dir>/vibe
```

`delegate-readonly` is a Vibe agent setup installs beside the critic agents:
`read_file` and `grep` only (Vibe 3.0.4 has no listing tool), nothing outside
the working directory, no subagents, no skills, no MCP, no project context. The
gateway also sets `VIBE_ENABLE_MCP`, `VIBE_ENABLE_SUBAGENTS`,
`VIBE_INCLUDE_PROJECT_CONTEXT` and `VIBE_OUTSIDE_WORKDIR` for the launch, so
the isolation holds even if the agent file is missing.

A launched Hermes run is Hermes, one-shot:

```sh
hermes -z <wrapped prompt> --usage-file <run dir>/usage.json -m <model> --provider <provider>
```

Hermes takes the prompt as its argument, so the run directory keeps `prompt.md`
for the record rather than for the launch. It writes its own usage to
`usage.json`: `estimated_cost_usd`, the token counts, `model`, `provider`,
`api_calls` and whether it `completed` or `failed`. The gateway reads that file
for the run's tokens and dollars, so a Hermes run is metered from what Hermes
itself priced; a run whose usage file is missing or unreadable counts as zero
tokens and zero dollars, and one Hermes reports as failed is a failed run with
`reason: "failed"`. Hermes loads the working directory's own rules and memory,
so the run carries `AGENT_KIT_DELEGATION=1` and nothing else.

The gateway appends a short instruction to the caller's prompt: end the answer
with a line `SUMMARY:` followed by at most `summary_max_chars` characters
summarising the result. The result's `summary` is that text, cut at the limit;
when the delegate wrote none, it is the last `summary_max_chars` characters of
its final answer.

The result is `{"status":"ok"|"failed","run_id","workflow","client","model",
"family","mode","tokens":{"input","output","cached"},"cost_usd","output_path",
"summary"}`, and a failed run adds a `reason`: `timeout`, `parse-error`,
`exit-<n>` or `launch-failed` (Vibe could not be started at all). The raw
output is kept either way.

A run with a `repo` keeps its directory in that repository's git common
directory, `<git common dir>/model-gateway/runs/<run_id>/`, so a worktree's
runs sit with the repository's; an inline run keeps it in
`$XDG_STATE_HOME/agent-kit/model-gateway/runs/<run_id>/`, else
`~/.local/state/agent-kit/model-gateway/runs/<run_id>/`. It holds `prompt.md`,
`input.json`, `output.json` (Vibe's full output) and `result.json`. `run_id` is
a UTC timestamp plus a short random suffix.

Every run also appends one JSON line to the machine-wide ledger,
`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl` (else
`~/.local/state/...`): workflow, client, model, family, mode, repository key,
tokens, dollars, outcome, duration and run id. Writes take an exclusive lock
file beside it, so two gateway processes never interleave a line. Dollars come
from the client's own report when it prices the run — Vibe's `export.json`, or
Hermes' `usage.json` — else from the tokens times the catalogue's `prices` for
the model.

## The `workflow` commands

`workflow start`, `workflow end` and `workflow list` keep a workflow's life.
`start` and `end` take `{"name": "<id>"}`; `list` takes `{}`. Each prints one
JSON object: `{"status":"ok","workflow":{"name","started_at","ended_at","state"}}`
for `start` and `end`, and `{"status":"ok","workflows":[...]}` for `list`.

```sh
node model-gateway.mjs workflow start --json '{"name": "spec-293"}'
node model-gateway.mjs workflow end   --json '{"name": "spec-293"}'
node model-gateway.mjs workflow list  --json '{}'
```

A workflow records when it started and, once someone ends it, when it ended. A
workflow nobody ends within 24 hours counts as ended, worked out on read from
the clock, so a forgotten workflow cannot stay open for ever. `run` joins a
workflow: one named for the first time starts at that moment, and one that
ended is refused (`code: "workflow-ended"`), because its caps and grants
lapsed with it — a new piece of work needs a new name. `workflow end` on a
workflow this machine never started is `code: "unknown-workflow"`.

The workflows live in `$XDG_STATE_HOME/agent-kit/model-gateway/workflows.json`,
else `~/.local/state/agent-kit/model-gateway/workflows.json`, beside the
ledger and under the same lock: a change reads the file, applies itself and
writes the result through a temporary file that is renamed into place, so two
gateway processes never lose each other's change.

## The data policy

The family allowlist critique writes decides what a repository's content may
go to. It lives at `$XDG_CONFIG_HOME/agent-kit/critique-allowlist.json`, else
`~/.config/agent-kit/critique-allowlist.json`, and keys each repository by
`<host>/<owner>/<repo>` with `families`, `code` and `history`.

- A repository with an entry lists only the client models whose family the
  entry names, and each reports the entry's `code` and `history` in `may_send`.
- A repository with a remote but no entry, or with no remote, lists nothing and
  `policy` says why.
- A repository owned by the `nedap` GitHub organisation lists only Codex and
  Claude Code, whatever its entry says, with code and history allowed.
- No `repo`, or a path outside any repository, lists only subsidised and
  metered-personal client models, each `inline_only` and allowed to send
  neither code nor history.

`anthropic` and `openai` are families like any other: a repository's entry must
list them for a delegation to reach Claude Code or Codex. In a `nedap`
repository it is the client that counts, not the family: only the Codex and
Claude Code clients may receive anything.

A path git cannot read — one that does not exist, or a repository git refuses
for its ownership or permissions — is an error (`code:
"repository-unreadable"`) naming the path and git's message. It never falls
back to inline content.

## The machine override

The owner's own settings live outside this repository, in
`$XDG_CONFIG_HOME/agent-kit/model-gateway.json`, else
`~/.config/agent-kit/model-gateway.json`. A missing file means no overrides.

The file takes the same top-level keys as `defaults.json`, every one optional,
and merges over the shipped defaults:

- `clients.<id>.<field>` overrides that field, including
  `provider_cost_classes`: the cost class of each provider a client's models
  run on, by provider name (OpenCode's, whose models are listed as
  `<provider>/<model>`). A provider it does not name takes the client's own
  cost class.
- `clients.hermes.providers`: the providers a Hermes run may name, by the name
  `hermes --provider` takes. It is empty by default, so no Hermes run is
  allowed until the owner lists one; `openrouter` stays refused whatever it
  says.

A client field the gateway does not know is an error naming it, rather than a
setting quietly ignored: a client has `binary`, `cost_class`, `model_source`,
`families`, `provider_cost_classes` and `providers`, and nothing else.
- `models`: an entry matching `client` and `model` overrides its `tier`,
  `notes` or `family`; a new entry adds a declared model.
- `settings.<key>` overrides that setting, deep-merging
  `metered_personal_caps`.
- `disabled`: a list of `"<client>:<model>"` keys left out of `models`.

An unreadable or invalid override is an error (`code: "invalid-config"`) that
names the file. Never edit the shipped `defaults.json` to change this machine:
the next setup run replaces it.
