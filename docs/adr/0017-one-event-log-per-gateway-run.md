# One event log per gateway run, written by the gateway

The gateway ran each client to completion and buffered its pipes. Nothing could
be seen while a run went, and a run that ended badly could leave nothing to read:
two OpenCode runs on 2026-11-26 failed with no exit code, no error and no
`stderr.txt` in their results. The client had written its error as a JSON line on
stdout and exited 1 with an empty stderr. The gateway ranked the exit code above
that error, so the reason became the bare `exit-1`. It skipped `stderr.txt`
because stderr was empty, and `result.json` had no member for how the process
ended. A client killed by a signal read as exit code 124, with nothing to say a
signal did it.

We now stream each launch and write one event log per run. The gateway writes it,
not the client, so it reads the same for every harness.

- **Runner.** The client starts as the leader of a process group of its own. One
  thread per pipe hands each chunk to the run as it is read. `output.json` and
  `stderr.txt` grow live and are rewritten whole at the end, and `stderr.txt` is
  written even when it is empty. The wait status is decoded into an exit code or
  a signal. The time limit sends SIGTERM to the group and SIGKILL after the
  grace. What the client leaves running in its group when it exits is killed, so
  its pipes close. A gateway stopped with SIGINT, SIGTERM or SIGHUP passes the
  stop on to the group. It records `signalled` and starts no further launch.
  Since the group no longer shares the terminal's, a Ctrl-C would otherwise
  reach only the gateway.
- **Result.** `result.json` gains `exit_code`, `signal`, `stderr_tail` (4 KB)
  and, for a failed run, `failure`. That member is `kind` plus one line of
  `reason`, and `output_tail` when stderr was empty. The kinds are
  `spawn-failed`, `signalled`, `timed-out`, `sandbox-denied`, `harness-error`
  and `gateway-error`. The `reason` slug is unchanged, since the retry and the
  climb read it.
- **Schema.** `events.jsonl`, `v: 1`. The envelope is `v`, `seq`, `ts`, `type`,
  `run_id`, `workflow` and the OpenTelemetry GenAI attributes
  (`gen_ai.operation.name`, `gen_ai.system`, `gen_ai.agent.name`,
  `gen_ai.request.model`, and `gen_ai.usage.*` in the finish), so an OTLP
  exporter later is a mapping. JSONL stays the format on disk: it can be read
  with grep and needs no collector. The types are `run.started`, `diagnostic`,
  `heartbeat` (every 15 s) and `run.finished`. The adapters' types (message,
  tool, usage, plan) and stall detection come in later slices. Each line is one
  append.
- **Exactly one finish.** The log's guard writes a failed finish when the
  gateway returns an error after the launch. A gateway killed outright cannot
  write anything. Its `run.started` names its process id, and the next run
  started in the same runs directory finishes such a log as `lost` once that
  process is gone. It first claims the log with an exclusively created
  `events.lost`, so two gateways never both write the finish.

## Sandbox denials are not captured

On macOS 27 a denial inside an `agent-launcher confine` profile does not show in
`log show` or `log stream` for the confined process (tried with a confined
`cat` of a file outside its grants). Capturing it would need a reporting rule in
the profile. Until then `sandbox-denied` is read from the client's stderr
("Operation not permitted", `deny(`), and the reason says the denial log is not
captured.

## Consequences

- Goldens of every `run` scenario carry `events.jsonl`, an always-written
  `stderr.txt` and the new result members; the process ids in a log are masked.
- `Process` gained `run_watched` and `is_alive`, and `Files` gained `list_dir`,
  each with a default. A fake that does not stream reports the whole output once
  the program is done.
- The log is never a reason to fail a run. A line that cannot be written is
  dropped.
