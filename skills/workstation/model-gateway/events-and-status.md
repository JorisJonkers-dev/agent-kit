### The event log

`events.jsonl` is one JSON object per line, appended while the run goes, the
same for every client. Each line has `v` (the schema version, `1`), `seq`
(from 1, per run), `ts` (RFC 3339, milliseconds), `type`, `run_id`,
`workflow`, the OpenTelemetry GenAI attributes `gen_ai.operation.name`
(`invoke_agent`), `gen_ai.system` (the model family), `gen_ai.agent.name`
(the client) and `gen_ai.request.model`, and `data`:

| `type` | `data` |
| :- | :- |
| `run.started` | `gateway_pid`, `mode`, `time_limit_s` |
| `process.started` | `pid`: the client's process, the leader of its own process group |
| `sandbox.started` | `name`: the run's Docker Sandbox, which `cancel` removes |
| `diagnostic` | `level`, `source` (`stderr`: one per stderr line, the first 200; `gateway`: what the runner did, such as the time limit's SIGTERM), `text` |
| `heartbeat` | every 15 s while the client runs: `pid`, `alive_s`, `stdout_bytes`, `stderr_bytes`, `last_output_age_s` |
| `fallback` | the free fallback this run is: `fallback` (`from`, `to`, `reason`, `from_run_id`), `available_again_in_s`, `available_at`, `recommendation` |
| `retry` | this run runs again after the wait the proxy named: the `retry` object above |
| `routing` | when the gateway saw the proxy's answers: `capture`, `requested`, `litellm_model_id`, `api_base`, `served_model`, `attempted_fallbacks`, `fallback_fired` (LiteLLM's own fallback) and `gateway_fallback` (this run is the free fallback) |
| `message` | what the model said or thought, read from the client's own output as it arrives: `role`, `kind` (`text` or `thinking`), `text` (cut at 8,000 characters) |
| `plan` | a plan or to-do list the client reports: `entries` of `text` and `status` |
| `tool.started`, `tool.finished` | a tool call: `gen_ai.tool.call.id`, `gen_ai.tool.name`, `input` (started), `status` (`ok` or `error`) and `output` (finished), each cut at 2,000 characters |
| `usage` | tokens and dollars the client reported: `gen_ai.usage.input_tokens` (the cached prompt included, as in the ledger), `gen_ai.usage.output_tokens`, `cached_input_tokens`, `cost_usd` (`null` when the client prices nothing) |
| `stall` | the client wrote nothing for the threshold: `silent_s`, `threshold_s`, `cancel_at_s` |
| `cancel` | the run is being cancelled: `by` (`owner` or `stall`) and `reason` |
| `run.finished` | exactly once: `status` (`ok`, `failed`, `cancelled`, `lost`), `exit_code`, `signal`, `reason`, `failure`, `stderr_tail`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `cached_input_tokens`, `cost_usd`, `pid`, `duration_s` |

`message`, `plan`, `tool.*` and `usage` come from the client's stdout, one
adapter per client (ADR-0020): Claude Code (`--output-format stream-json
--verbose`), Codex (`exec --json`), OpenCode (`run --format json`), pi (`--mode
json`), Vibe (`--output streaming`) and Antigravity (`--output-format
stream-json`) each stream native JSON, and Hermes is driven over ACP (`hermes
acp`), since it has no streaming JSON of its own. A line an adapter does not
know, or that is not JSON, becomes a `diagnostic` with `source` `adapter` and
never fails the run. A client whose stream carries no usage (Vibe, which
writes it to an export) has one `usage` event written from the run's result
just before `run.finished`. Codex prices nothing, so its `usage` has no
`cost_usd`; the priced figure is on `run.finished`.

`tail -f events.jsonl` shows a live run. A gateway killed before it wrote
`run.finished` leaves the log open; the next run started beside it (same runs
directory) finishes it as `lost` (reason `gateway-lost`) once that gateway's
process is gone. The log never fails a run, and `result.json` stays the
record.

### What the runs are doing: `ps`, `status`, `logs`, `tail`, `cancel`, `gc`

Every run is listed in a global index, `run-index.jsonl` beside the ledger in
the state directory: one line when it starts, one when it ends, one when `gc`
removes it. Each line is one append, so any number of gateways write it at
once; at 1 MiB it is renamed to `run-index.<ms>-<id>.jsonl` and a new one
starts, and only the newest eight rotated files stay. These commands answer
from the index and the runs' event logs and never read a client's raw output.

```
model-gateway ps [--all] [--workflow NAME] [--state STATE] [--limit N] [--json]
model-gateway status RUN [--json]
model-gateway logs RUN [--types TYPE,TYPE] [--json]
model-gateway tail [-f] [--types TYPE,TYPE] [--run RUN] [--workflow NAME] [-n LINES] [--since SECONDS] [--exit-when-done] [--json]
model-gateway cancel RUN [--json]
model-gateway gc [--days N] [--dry-run] [--json]
```

- `ps` lists the running runs and those started in the last 24 hours, newest
  first: run id, state, client, model, mode, age and last event. A run is
  `running`, `stalled` (quiet past the stall threshold), `ok`, `failed`,
  `cancelled` or `lost` (its gateway died without writing a finish).
  `--all` lists everything the index holds.
- `status RUN` is one run: state, current step, last event and its age,
  heartbeat age, seconds since the client last wrote, tokens, cost, the
  client's process id, its sandbox, and for a failed run its reason and
  failure. `RUN` is a run id or the start of one.
- `logs RUN` prints the run's events and diagnostics, a line each; `--json`
  prints them as they are on disk.
- `tail` prints `<ts> <run id> <type> <summary>` lines. `-f` follows runs
  that are going or ended in the last `--since` seconds (600), at most one
  line per run every 10 seconds (the line says how many events it stands
  for), except `stall`, `cancel` and `run.finished`, which come at once.
  `--types stall,run.finished` is the watch for a batch: it prints only
  those. It ends when `--run`'s run finishes, or with `--exit-when-done` when
  every run it follows has, and otherwise never, which is what Monitor wants.
- `cancel RUN` sends SIGTERM to the client's whole process group and SIGKILL
  after five seconds, removes the run's sandbox with `sbx rm --force`, and
  the run ends as `cancelled` with exactly one `run.finished`. Its gateway
  writes the finish; only when that gateway is gone does `cancel` write it.
  A cancelled delegation is never retried, and a batch skips what depends on
  it.
- `gc` removes the directories of runs that ended more than `--days` (30)
  days ago, notes it in the index, and drops rotated index files that old. It
  never touches a run that is still going.

[`watching-a-batch.md`](watching-a-batch.md) is the guide for an orchestrating
session: the batch in the background, a filtered `tail -f` under Monitor, and
what to read when a line needs a look.

`mcp` serves `ps`, `status`, `logs` and `tail` as a read-only MCP server (newline-delimited
JSON-RPC on stdin and stdout, no socket). It answers from the same run index and event logs, so
its `ps` is the table `model-gateway ps` prints; `logs` and `tail` return at most the newest 200
lines, and it has no tool that starts, changes or cancels a run. It is off unless setup registers
it: `AGENT_KIT_RUN_STATE_MCP=1 installer/setup-workstation.sh` adds `run-state` to Claude Code and
Codex, and a setup run without the variable removes it again.

`export [--endpoint URL] [--file PATH] [--run RUN] [--workflow NAME]` sends the event logs as
OpenTelemetry traces: one trace per run (the trace id is the run id's SHA-256, so exporting twice
updates, not duplicates), one span, `invoke_agent <client>`, with every event as a span event. The
`gen_ai.*` attributes go out under the names and values the log holds (`gen_ai.operation.name`,
`gen_ai.system`, `gen_ai.agent.name`, `gen_ai.request.model`, `gen_ai.usage.*`, `gen_ai.tool.*`); the
rest of an event's data goes under `gateway.*`, and message and tool text (`text`, `input`,
`output`) never leaves the machine. The endpoint is `--endpoint`, else
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, else `OTEL_EXPORTER_OTLP_ENDPOINT` with `/v1/traces`
(OTLP/HTTP, JSON body, sent with `curl`; `OTEL_EXPORTER_OTLP_HEADERS` become request headers
without appearing in the process list). `--file` appends the same request as one line. With
neither, `export` does nothing and says so; nothing in the gateway exports by itself.

**Stall policy.** A run whose client writes nothing to stdout or stderr for
`settings.stall.threshold_s` seconds (180 by default, set in the machine
override) gets a `stall` event, and one still silent at three times that is
cancelled the way `cancel` does it, with the reason in `cancel.requested` in
the run directory and in the result's `failure`. A client that writes again
before the cancel starts the count over.

`batch` prints one progress line per delegation state change on stderr
(`batch <batch id> <delegation>: queued|running|done|failed|skipped
<client>:<model>`), so its JSON on stdout stays clean.

Every run also appends one JSON line to the machine-wide ledger,
`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl` (else
`~/.local/state/...`): workflow, client, model, family, mode, harness,
repository key, tokens, dollars, outcome, duration and run id. Writes take an
exclusive lock file beside it, so two gateway processes never interleave a
line. Dollars come from the client's own report when it prices the run — Vibe's
`export.json`, or Hermes' `usage.json` — else from the tokens times the
catalogue's `prices` for the model.

The line also carries the run's routing: `tier` (the LiteLLM tier its profile
is, else `null`), `litellm_model_id` (the deployment that served it, from
`x-litellm-model-id`), `served_model` (the model the response body names),
`attempted_fallbacks` (from `x-litellm-attempted-fallbacks`), `fallback_fired`
(`attempted_fallbacks >= 1`) and `routing_capture`, which says how much the
gateway saw: `recorded`, `no-litellm-headers`, `no-request`, `hidden` or
`not-litellm`. A field the gateway could not see is `null`, never a guess. A
run on Pi that carries no tools of its own goes through a recording proxy, so
its routing is `recorded`; every other run is `hidden` or `not-litellm`. A
recorded run's `result.json` carries the same as a `routing` object (and a
`routing` event), with `api_base` from `x-litellm-model-api-base` when the proxy
sends it, so `status` can name the serving route and whether a fallback fired. A `--tier` run also carries
`tier_decision` (`requested`, `chosen`, `reason`, `signals`), and a run on a
LiteLLM route carries `allowance_usd` and `litellm_cost_usd` when its responses
reported them.

