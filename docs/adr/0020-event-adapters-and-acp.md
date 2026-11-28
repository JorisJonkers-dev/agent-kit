# Event adapters read each harness's stream; Hermes is driven over ACP

Decided 2026-11-28 (#938, #939, spec #934). ADR-0017 gave every run an event log that holds only the gateway's own
events (`run.started`, `diagnostic`, `heartbeat`, `run.finished`). Nothing of what the harness was doing showed
until it ended: a batch could only be told by its heartbeats that a run was alive.

## Decision

Each harness's stdout is read, line by line, as it arrives, into the same events (`message`, `plan`, `tool.started`,
`tool.finished`, `usage`), written by the run's log beside the gateway's own. One adapter per client lives in
`event_adapters/`; the log picks it from the client id and feeds it every complete stdout line. Event data uses
OpenTelemetry's GenAI names (`gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.usage.input_tokens`,
`gen_ai.usage.output_tokens`). Text is cut (messages 8,000 characters, tool input and output 2,000), since the log
is read with `tail`.

- **An unknown line never fails a run.** A line that is not JSON, or an event an adapter has no meaning for, is a
  `diagnostic` (`source: adapter`, `level: info`), and the run goes on. The adapters never touch the result: the
  existing parsers in `outputs.rs` still decide the answer, the tokens, the dollars and the failure.
- **Cost in the events is the spend record's.** An adapter counts a client's tokens and dollars with the same
  arithmetic as that client's parser (the cached prompt inside the input; Claude's cache writes inside the input;
  OpenCode's reasoning inside the output), and the golden tests assert that the usage events of every fixture add
  up to what the parser records. Claude and Antigravity report totals in their last event, so the usage event is
  written there; pi and OpenCode report per message and step, which are summed. Codex and Antigravity price
  nothing (`cost_usd` is `null` on their `usage`; the figure the ledger records, from the catalogue's prices, is on
  `run.finished`). A client whose stream carries no usage at all (Vibe: its tokens and dollars are in
  `export.json`) has one `usage` event written from the result just before `run.finished`.

## When a harness uses ACP and when native JSON

**Native JSON when the harness streams events on stdout by itself.** The flag the gateway passes to start it:

| Harness | Flag | Notes |
| :- | :- | :- |
| Claude Code | `--output-format stream-json --verbose` | permanent: no protocol adapter between the gateway and the most used harness (#934 story 49); the final `result` event is what `claude_output` reads |
| Codex | `exec --json` | permanent, as above |
| OpenCode | `run --format json` | tool parts arrive once finished, so a tool is started and finished together |
| pi | `--mode json` | the `message_update` deltas are not events; the assistant message and the tool executions are |
| Vibe | `--output streaming` | one journal entry per line, the same entries `--output json` printed as one array, so the answer is read from either; its `effect` entries carry the tool's end state, so a tool is started and finished together. The stream has no usage; ACP (`vibe-acp`) is not needed, and stays unused until Vibe's stream is not enough |
| Antigravity | `--output-format stream-json` | `step_update` events, one start and one end per tool step |

**ACP when the harness has no streaming JSON.** Hermes prints the answer text alone (`hermes -z`), and a
one-shot run therefore reports nothing until it ends. `hermes acp` speaks the Agent Client Protocol: JSON-RPC 2.0,
one message per line, on stdin and stdout. The gateway is the ACP client. It sends `initialize`, then `session/new`
(the run's working directory, no MCP server), then `session/prompt` with the prompt, each when the answer to the one
before has been read, and closes stdin when the prompt's answer comes. `session/update` notifications are the events
(`agent_message_chunk` and `agent_thought_chunk` joined into one `message` each, `tool_call` and `tool_call_update`,
`plan`); the prompt's answer holds the stop reason and the usage (`inputTokens`, `cachedReadTokens`, `outputTokens`).
The gateway advertises no client capability, so Hermes asks it for no file or terminal; a permission request it does
make is refused, and any other request is answered `method not found` so the agent never waits on it. The answer the
ledger records is the text after the last tool call; a stop reason other than `end_turn`, or an error answer, is a
failed run with the agent's own words.

The rule for a harness added later: it uses native JSON if it has a mode that writes one event per line and each
event is complete (a tool call's input and its result both appear); it uses ACP if it has none and an `acp` mode.
A harness with neither is read as text, one `message` per run, and gets no tool events.

## Consequences

- `LaunchWatch` gained `interactive`, `opening` and `reply`, each with a default, so only a watch that drives its
  client says anything: the runner writes `opening` to the client's stdin, keeps the pipe open, writes what `reply`
  returns after every stdout chunk and closes the pipe when it says so. Every other launch is unchanged.
- Hermes no longer gets its prompt as an argument, and writes no `usage.json` (ACP has no `--usage-file`): its tokens
  are the prompt's usage, and its dollars come from the catalogue's prices as for Codex.
- A fake Hermes that does not speak ACP never gets its stdin closed; the time limit ends it.
- The goldens of every `run` scenario that launches Claude Code, Vibe, Antigravity or Hermes carry the new flags.
- Fixtures in `src/tests/event_fixtures` are streams recorded from real runs (secrets and paths removed, long text
  cut), except Codex's: no Codex run was made on the machine they were recorded on (Codex is metered by an
  employer and needs a grant), so its adapter was built from the documented `exec --json` format and its fixture is
  written by hand from that format. It is to be replaced by a recording when a grant allows one.
