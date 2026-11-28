## The `catalog` command

`models` lists what a client can run; `catalog` answers "what can I run, where,
and for how much" from the other side: one row per canonical model the proxy
serves, whether or not any client runs it now.

```sh
model-gateway catalog [--repo PATH] [--route ROUTE] [--status STATUS] [--modality MODALITY] [--free] [--json]
```

It prints a plain table by default: a line per model with its modality, family,
tier, context length, free flag, cost (the class, then the input and output price
per million tokens, `unknown` where nobody states one, `0` for a free model) and
status, then indented lines for what serves it and what runs it. `--json` prints
one JSON object (`status`, `policy`, `updated_at`, `warnings`, `missing`,
`pools`, `models`) whose entries carry `model`, `modality`, `family`, `tier`,
`context_length`, `free`, `cost` (as `models` gives it), `status`, `reason`,
`deployments`, `clients` and `profiles`. Each deployment names the `pool` (the
route's LiteLLM key) it draws on, or `null`. Each `pools` entry gives `key`,
`route`, `monthly_cap_eur` (`"none"` for an uncapped key), `budget_duration` and
`spend_eur`, which `measure-litellm-budget --agent ROUTE` reports for a capped
key and which is `"unknown"` when that program is missing or the proxy does not
answer. The table prints the same as a `budget pool` line per key.

A deployment is one provider that serves the model: its `order` (fallback order,
lowest first), `provider` (Mistral, Google, Anthropic, OpenRouter, OpenCode Go or
local), the LiteLLM `route` and the `key` (the environment variable of the
provider key) that reach it, the `upstream_id` it serves the model under, and its
probe `status` with the provider's own `reason`. `clients` and `profiles` are the
clients that list the model now and the delegation profiles that allow one of
them, under the data policy of `--repo`. A row's `status` is that of the usable
deployment, else of the first the catalogue probed (`unprobed` when none).

A model that cannot run is never dropped: an embedding or OCR model, a model every
deployment of which is blocked, and a model a client skipped are rows with the
reason in `reason`. A client model no catalogue group covers is a row of its own.
`--route` keeps the models a deployment or a client of that route serves,
`--status` the models whose row or any deployment has that status, `--modality`
the models of that modality, and `--free` the free ones. A client the gateway
cannot use is named at the end of the table, and in `missing`.

## The `usage` command

`usage` reports every client the gateway knows: whether it can run now, what
it has spent, what its own plan has left, and how close it is to the plan
limit the owner set for it. It prints one JSON object on stdout and exits 0.

```sh
model-gateway usage --json '{"days": 7, "workflow": "spec-293"}'
model-gateway usage --input <file.json>
```

The input is optional: `days` (7 by default) is how many UTC days the usage
covers, ending today, and `workflow` narrows the usage to one workflow. A
`days` that is not a whole number above 0, or a `workflow` that is not a name,
is `code: "invalid-input"`.

The result is `{"status":"ok","days","clients":[...]}`, plus `workflow` when
the caller named one. Each client carries:

- `availability` — `{"status","installed","signed_in","reason"}`, where
  `status` is `available`, `unavailable` or `unable_to_start`, and `reason` is
  one line saying what the reading rests on. It is found passively, never by
  sending a prompt:
  - `installed` is whether the client's binary is on `PATH`, as `models` asks.
  - `signed_in` comes from the client's own status command, run read-only
    through the process port: `codex login status`, `hermes auth list` and
    `opencode auth list`. Vibe, Antigravity and Claude Code have no status
    command the gateway can read, so they report `"unknown"` — which does not
    stop a run, because silence is not a refusal. A status command that cannot
    be run is `"unknown"` too, never `"no"`.
  - A client whose last run in the availability window
    (`availability_window_minutes`, 60 by default) was `rate-limited` or
    `auth` is `unavailable`, unless a later run of it succeeded. The window is
    a shipped default the machine file may override.
  - A client at 100% of any plan window is `unable_to_start`.
- `quota` — `{"status":"unknown","reason"}`. No client's status command
  reports what is left of its plan, so the gateway says `unknown` rather than
  a zero that would read as an exhausted plan.
- `usage` — `{"runs","tokens":{"input","output","cached"},"cost_usd"}` over
  the days the report covers, read from the ledger.
- `by_day` — the same totals per UTC day, oldest first, for the days that have
  runs.
- `by_workflow` — the same totals per workflow, in workflow order.
- `plan_limits` — one entry per window of the client's plan limit, with what
  the ledger says is spent in it: `{"window","limit_usd","spent_usd","ratio",
  "state"}` or `{"window","limit_runs","runs","ratio","state"}`, where `state`
  is `ok`, `warning` (at or above `plan_warning_ratio`, 0.8 by default) or
  `exhausted` (at 100%).
- `warnings` — one line per window at or above the warning ratio, for example
  `opencode has used 93% of its week plan limit ($28.00 of $30.00)`.

