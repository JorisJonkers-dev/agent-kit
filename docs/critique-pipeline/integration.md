# Critique pipeline: integration

This document settles what the critique runner can rely on from the two critic
CLIs, maps every deliverable of the critique pipeline onto agent-kit's layout,
and lists where the design departs from its brief and why. The words in bold
in [CONTEXT.md](../../CONTEXT.md) under "Critique pipeline" mean what they say
there, and [ADR-0001](../adr/0001-critics-run-as-direct-cli-calls.md) records
why critics are direct CLI calls.

The source brief ("Cross-Provider Critique Pipeline — Implementation Brief",
30 Oct 2026) is not kept in this repository. Its deliverables below are
reconstructed from the spec, agent-kit#167, which is the source of truth for
every design decision. Where this document and #167 disagree, #167 wins until
it is amended.

The checks were run on 2026-10-30 for agent-kit#168, on macOS, against
Vibe 3.0.3 (`~/.local/bin/vibe`, the `mistral-vibe` uv tool) and Antigravity
CLI 1.4.11 (`/opt/homebrew/bin/agy`).

## Summary

Most of what #167 assumes holds. Five things do not, and the tickets that
build on this one need to know:

- **Vibe's `--max-price` does not cap a single turn.** It is checked between
  turns. A one-turn run capped at $0.012 finished its turn and spent $0.034.
  Only `--time-limit` interrupts a turn in progress.
- **Vibe's cost is only as good as the prices in its config.** Vibe computes
  `cost_usd` from the `input_price` and `output_price` of the model entry.
  The models setup syncs into the personal Vibe home have none, so Mistral
  Large and DeepSeek Flash report $0.00 and the price cap never fires. A
  critic agent file has to carry its own priced model entry.
- **Vibe 3.0.3 offers no `grep` tool in headless mode.** Listing `grep` in the
  tool allowlist is harmless but does nothing; a critic can read files and
  nothing else. The prompt has to name the packet files.
- **agy ignores `--mode plan` when `--disable-slash-commands` is set**, and
  says so on stderr. Plan mode cannot be combined with disabling skills.
  What keeps an agy critic read-only is that headless mode auto-denies every
  action that needs permission, and a denial ends the run with an empty
  response.
- **agy enforces the finding schema by omission, not by retry.** A response
  that fits the schema arrives as `structured_output`; one that does not is
  returned as prose with no `structured_output` field and exit code 0.

Everything else, including pinning the model in a Vibe agent file, is
confirmed below with the command and the output that shows it.

## How the checks were run

Every run used a throwaway working directory under the session scratchpad,
never a repository checkout, and a trivial prompt with no repository content
(a one-line `note.txt` containing "The answer is 7.", and an `outside.txt`
next to the working directory containing a canary string). Each Vibe run had
`--max-price` of $0.05 or less, a `--time-limit`, and an outer `timeout`. Each
agy run had `--print-timeout` and an outer `timeout`.

The critic agent files were placed in the throwaway directory's own
`.vibe/agents/`, which Vibe reads from a trusted working directory, so
nothing was written to the owner's Vibe home. Vibe and agy still record their
own session history under `~/.vibe/logs/session` and
`~/.gemini/antigravity-cli/conversations`; those entries hold only the
trivial prompts.

A SHA-1 listing of every file in the working directory was taken before and
after each run. "Working directory unchanged" below means the two listings
were identical.

Total live spend: Vibe recorded $0.115 in its `export.json` files. Two runs
used models with no configured price, one used placeholder prices, and one
was interrupted before its usage was counted (see the findings on cost), so
the true figure is somewhat higher, roughly $0.15. agy ran 11 times in print
mode against the Antigravity quota, which has no dollar cost.

## Vibe

All Vibe runs used the personal account. `MISTRAL_API_KEY` and `VIBE_HOME`
were unset, so Vibe used its default home `~/.vibe` and the machine-wide
login, and every run that reached the API printed:

```text
Reading API keys from the macOS Keychain (set them as environment variables to skip this)
```

That keychain item is the personal login (see the Vibe launchers in
[SETUP.md](../SETUP.md#vibe-launchers-and-private-memory)). `vibe-study` hands
the Study account key over as `MISTRAL_API_KEY`, which overrides the
keychain, so **the runner must unset `MISTRAL_API_KEY` for every Vibe critic
launch**, or a critique started from a `vibe-study` session spends on the
Study account. It should unset `VIBE_HOME` too, so a critique started from
`vibe-private` does not run in the private home.

### An agent file can pin the model

It can. A Vibe agent file is a config overlay: everything in it other than
`display_name`, `description`, `safety`, `agent_type` and `instructions` is
applied on top of `config.toml` for that run, `active_model` included (the
built-in `lean` agent pins its model the same way). This agent file pins
GLM 5.3, which is not the personal home's default (DeepSeek Flash):

```toml
display_name = "Critic GLM"
description = "Read-only critic pinned to GLM 5.3"
safety = "safe"
active_model = "glm-5-3"
enabled_tools = ["read_file", "grep"]
outside_workdir = "deny"
enabled_skills = ["re:^$"]
enable_mcp = false
enable_subagents = false
```

```sh
timeout 90 vibe -p "Reply with exactly the single word: pong" --output json \
  --max-price 0.02 --time-limit 60 --trust --agent critic-glm --output-dir out
```

Exit code 0. `out/export.json` reports `"active_model": "glm-5-3"`, and the
session journal's `meta.json` records the model actually called:

```json
"config": { "active_model": "glm-5-3", "model_name": "zai-glm-5-3" }
```

The same works for a model the agent file defines itself. This file adds its
own entry for DeepSeek Flash under a new alias, with prices:

```toml
active_model = "critic-deepseek-flash"

[[models]]
name = "deepseek-v4-1-flash-1017"
provider = "mistral"
alias = "critic-deepseek-flash"
input_price = 0.3
output_price = 1.0
thinking = "high"
```

The run reported `"active_model": "critic-deepseek-flash"`, called
`deepseek-v4-1-flash-1017`, and computed `cost_usd` from those prices. (The
prices above were placeholders for the test; the real ones belong in the
shipped agent files.) This is the form the `critic-<model>` agents should
take: the agent owns its model entry and prices, and does not depend on what
`vibe-sync-models.py` happened to write into the home.

So none of the alternatives is needed. For the record: Vibe 3.0.3 has no
per-run model flag (`vibe --help` lists none), Vibe reads any config key from
a `VIBE_` environment variable (its bundled documentation gives
`VIBE_ACTIVE_MODEL` as the example; not exercised here), and `VIBE_HOME`
selects a separate home.

The agent file can also turn off session logging for critic runs, so packet
text does not accumulate in the personal home's history. With
`[session_logging]` `enabled = false` in the agent file, the effective config
in `export.json` showed `"enabled": false`. Whether Vibe then writes nothing
at all under `~/.vibe/logs/session` was not checked: this session cannot read
the Vibe home (setup denies Claude `Read(~/.vibe/**)`).

### A read-only agent loads headless, and the CLI enforces it

The agent above (`read_file` and `grep` only, `outside_workdir = "deny"`,
skills filtered out, MCP and subagents off) loads in headless mode with no
warning, also under `VIBE_STRICT_CONFIG=1`, which refuses a config with any
setting Vibe does not use. `export.json` shows the overlay took effect:

```text
enabled_tools: ['read_file', 'grep']   outside_workdir: 'deny'
enable_mcp: False   enable_subagents: False   tools: {}
```

The prompt told the critic to create a file, run `touch created2.txt`, and
read `../outside.txt`:

```sh
timeout 150 vibe -p "This is a permissions test. Do these three steps in order, ... 1) Create a file named created.txt ... 2) Run the shell command: touch created2.txt 3) Read the file ../outside.txt. ..." \
  --output json --max-price 0.04 --time-limit 120 --trust --agent critic-glm --output-dir out
```

Exit code 0, working directory unchanged, and the canary string from
`outside.txt` appears nowhere in the output. The model did try all three; the
tool results in the journal are:

```text
file_system.write_file  tool_denied  "Tool execution denied by approval policy"
file_system.bash        tool_denied  "Tool execution denied by approval policy"
file_system.read_file   tool_denied  "... outside the working directory and `outside_workdir = "deny"` ..."
```

A second run (DeepSeek Flash agent) read `note.txt` successfully, and was
refused `bash` and the `skill` tool the same way:

```text
file_system.read_file  completed   "The answer is 7.\n"
file_system.bash       tool_denied
skill                  tool_denied
```

Four things the runner and the agent files need to take from this:

- **There is no `grep` tool.** The model's tool catalog in 3.0.3 is
  `search_tool_functions`, `run_typescript`, `wait`, `read_file`,
  `write_file`, `edit`, `bash` and `skill`. Asked to grep, the model looked
  for a search function, found none, and tried `bash`, which was denied.
  A critic can only `read_file`, and cannot list a directory, so the prompt
  must list the packet's files by path.
- **The catalog still describes the denied tools.** The model sees
  `write_file`, `bash` and `skill` and is refused when it calls them, so it
  spends turns on attempts. The role prompt should say plainly that the
  critic can only read the listed files.
- **Skills are stopped by the tool allowlist, not by `enabled_skills`.**
  The system prompt listed the same four built-in plugin skills
  (`vibe:create-plugin`, `vibe:skill-creator`, `vibe:vibe`, `vibe:worktree`)
  with and without `enabled_skills = ["re:^$"]`. What prevents loading one is
  that `skill` is not in `enabled_tools`. Keeping `enabled_skills` in the
  agent file costs nothing; it is not the guard. None of the kit's own skills
  appeared in the headless system prompt in any run.
- **A trusted working directory is also a config source.** That is how the
  test agents above were found. According to Vibe's source
  (`vibe/core/config/harness_files/_harness_manager.py`), with `--trust` Vibe
  also reads tools, hooks and skills from `.vibe/` and `.agents/skills/` in
  the working directory, takes a `.vibe/config.toml` there as its config file
  in place of the user's, and collects `AGENTS.md` files up to the trust
  root. The packet builder must never put a `.vibe/`, `.agents/` or
  `AGENTS.md` at the packet root.

### JSON output shape

With `--output json`, stdout is one JSON array of the turn's journal entries,
in order. A one-step run gives two entries; a run with tool calls adds
`reasoning` and `effect` entries and several interim assistant messages:

```json
[
  { "type": "message", "role": "user", "content": [{"type": "text", "text": "Reply with exactly the single word: pong"}], ... },
  { "type": "message", "role": "assistant", "generationStatus": "completed",
    "content": [{"type": "text", "text": "pong"}], "outcome": {"type": "committed"}, ... }
]
```

- **Final message:** the last entry with `"type": "message"` and
  `"role": "assistant"`, text in `content[].text`. A message cut off by
  `--time-limit` is still there, marked
  `"outcome": {"type": "discarded", "cause": "interrupt"}`, so the runner must
  go by the exit code, not by finding text.
- **Tool calls:** `"type": "effect"` entries, with `detail.toolName` and
  `state.status` (`completed` or `failed`, with `state.error.code`, for
  example `tool_denied`).
- **Cost:** not on stdout. `--output-dir DIR` writes `DIR/export.json` on
  every exit, including errors, with:

```text
outcome: "finished"   stop_reason: null   error: null   exit_code: 0
cost_usd: 0.00689     steps: 1
usage: {input_tokens: 7049, output_tokens: 3, cached_input_tokens: 2624, total_tokens: 7052}
limits: {max_turns: null, max_price: 0.02, max_tokens: null, time_limit_s: 60.0}
config: { ...the effective config, active_model and models included... }
```

On a failure stdout is empty, the reason is on stderr as `Error: ...`, and
`export.json` has `outcome` and `error.message`.

The system prompt alone is about 7,000 input tokens, so even a trivial GLM
run costs about $0.003 to $0.007.

### Cost depends on configured prices

`cost_usd` is computed from the model entry's prices, not reported by the
API. The personal home's effective config gives prices only for Mistral
Medium 3.5 (1.5 / 7.5 per million tokens) and GLM 5.3 (1.4 / 4.4). Every
model `vibe-sync-models.py` adds has 0 / 0, including `mistral-large-4-0-3`,
`deepseek-v4-1-flash-1017`, `zai-glm-5-3-flash` and `mistral-small-latest`. A
pong run on Mistral Large reported `cost_usd: 0.0` for 7,081 input tokens.
With a zero price `--max-price` can never trigger and the per-flow Vibe cap
cannot be tracked. The critic agent files must define their model entries
with real prices (see above).

### Exit codes and limits

| Exit | Observed with | `export.json` `outcome` |
|---|---|---|
| 0 | every completed run | `finished` |
| 1 | `--output bogus`, `--max-price abc` (argument errors, rejected before a session starts); an agent file with an unknown key under `VIBE_STRICT_CONFIG=1` | `config_error` (strict config case) |
| 2 | a model id the API rejects (`Status 400 ... Invalid model`); **also `--agent` naming an agent that does not exist** | `infrastructure_failure` |
| 3 | `--time-limit` reached; `--max-price` reached | `deadline`, `price_limit` |

The unknown-agent case is worth noting: Vibe's own documentation describes
exit 1 as the usage or config error, but a missing agent exits 2 as an
infrastructure failure (`Error: Agent 'no-such-agent-xyz' not found.`). A
runner that retries exit 2 on a fallback client would retry a missing
`critic-<model>` agent too. It should check `error.message` or verify the
agent file exists before launching.

**`--time-limit` stops a turn in progress.** A 4,000-word essay prompt on the
GLM critic agent with `--time-limit 10`:

```text
exit=3 elapsed=11s
Stopped: deadline
outcome: 'deadline'  stop_reason: 'interrupted'  cost_usd: 0.0  usage: {input_tokens: 0, output_tokens: 0, ...}
```

The interrupted turn's tokens are not counted: `cost_usd` is 0 although about
ten seconds of output had streamed. The runner should not treat a deadline
stop as free; it can charge the critic's price cap as a conservative
estimate.

**`--max-price` stops the run, but only between turns.** The same prompt with
`--max-price 0.012 --time-limit 120`:

```text
exit=3 elapsed=79s
Stopped: price_limit
outcome: 'price_limit'  cost_usd: 0.03351  usage: {input_tokens: 7068, output_tokens: 6843, ...}
```

The single turn ran to the end (a complete 4,861-word essay, committed) and
cost 2.8 times the cap before the cap was applied. Setting
`max_output_tokens = 600` on the agent's model entry did not help: a repeat
run produced 6,515 output tokens and cost $0.0386. So a critic's real worst
case is its price cap plus one full turn, and the only hard bound inside a
turn is `--time-limit` (with the outer `timeout` behind it). The per-critic
caps in routing should be read with that in mind, and the spend check before
each Vibe launch should use the recorded `cost_usd`, which can exceed the cap.

### GLM and DeepSeek Flash are reachable headless on the personal account

Both answered through Vibe in headless mode on the personal login, as did
Mistral Large:

| Model | Alias in the personal home | Model id called (`meta.json`) | Result |
|---|---|---|---|
| GLM 5.3 | `glm-5-3` | `zai-glm-5-3` | exit 0, "pong", $0.0069 |
| DeepSeek Flash | `deepseek-v4-1-flash-1017` | `deepseek-v4-1-flash-1017` | exit 0, read and reported `note.txt` |
| Mistral Large 4 | `mistral-large-4-0-3` | `mistral-large-4-0` | exit 0, "pong", cost unpriced |

The personal home also lists `zai-glm-5-3-flash`, `zai-glm-latest`,
`glm-5-2`, `mistral-large-latest` and `mistral-small-latest`; the routing
configuration chooses among them.

## Antigravity (agy)

agy was signed in with the owner's Google account (the `cli.log` shows
`authMethod=consumer`). `agy models` lists six models; the only Gemini ones
are Gemini 3.8 Flash at three efforts:

```text
gemini-3.8-flash-high    Gemini 3.8 Flash (High)
gemini-3.8-flash-medium  Gemini 3.8 Flash (Medium)
gemini-3.8-flash-low     Gemini 3.8 Flash (Low)
claude-sonnet-4-6        claude-opus-4-6-thinking        gpt-oss-120b-medium
```

The Claude and GPT-OSS entries must never be used as critics (#167).

### JSON output shape

```sh
timeout 120 agy -p "Reply with exactly the single word: pong" --output-format json \
  --model gemini-3.8-flash-high --print-timeout 90s --disable-slash-commands --sandbox < /dev/null
```

stdout is a single JSON object:

```json
{"conversation_id":"70d9b4a0-...","status":"SUCCESS","response":"pong\n",
 "duration_seconds":2.495951,"num_turns":1,
 "usage":{"input_tokens":7661,"output_tokens":29,"thinking_tokens":28,"cache_read_tokens":0,"total_tokens":7690}}
```

- **Final message:** `response`.
- **Cost:** there is none. agy reports token usage only; Antigravity is
  quota, not money.
- **Structured output:** with `--json-schema`, a conforming result also
  appears as `structured_output` (see below), and the schema is echoed back
  as `json_schema`.
- **Denials:** `denied_actions`, a list of `{action, display_name}`, appears
  when headless mode refused something.
- **Stops:** `finished_reason: "print_timeout"` appears when
  `--print-timeout` fired.

`status` was `"SUCCESS"` and the exit code 0 in every run, including the ones
that produced nothing. The runner has to read the fields: an empty
`response`, a non-empty `denied_actions`, a `finished_reason` of
`print_timeout`, or a missing `structured_output` all mean the critic failed.

`--output-format stream-json` adds an `init` event (model, cwd,
`permission_mode`, and the full tool list), `step_update` events with text
deltas and per-step usage, and a final `result` event with the same object as
above.

### Plan mode, sandbox and read-only

`--mode plan` cannot be combined with `--disable-slash-commands`. The first
run passed both and agy printed on stderr:

```text
warning: --mode plan has no effect while slash command expansion is disabled.
```

Since #167 needs skills and slash commands disabled for a critic, plan mode
is not available to it. What actually makes headless agy read-only is the
permission model: an action that needs permission cannot be prompted for in
print mode, so it is auto-denied, and the run ends there with an empty
response.

With `--mode plan --sandbox` (slash commands left on), told to create a file,
run `touch` and read `../outside.txt`:

```text
exit=0  working directory unchanged
jetski: no output produced — a tool required the "write_file" permission that headless mode cannot prompt for, so it was auto-denied. ...
{"status":"SUCCESS","response":"", ... "denied_actions":[{"action":"write_file","display_name":"WriteFile"}]}
```

With `--disable-slash-commands --sandbox` and no plan mode, the same three
steps with the shell command first:

```text
exit=0  working directory unchanged
jetski: no output produced — a tool required the "command" permission ...
{"status":"SUCCESS","response":"", ... "denied_actions":[{"action":"command","display_name":"RunCommand"}]}
```

Reading a file inside the working directory needs no permission: told to use
only its file reader, the critic returned `note.txt` exactly. Told to read
`note.txt` and then `../outside.txt`, the run ended with no output and
`denied_actions: read_file`; since the inside read needs no permission, the
refused read was the one outside.

So agy refuses to create a file, run a command or read outside its working
directory in headless mode, with or without plan mode. Two cautions follow:

- **A denial kills the critique.** Gemini Flash readily reaches for a shell
  command (one review run, asked only to read `note.txt` and report defects,
  tried `RunCommand` and produced nothing). The role prompt must say the
  critic can only read the listed files, and the runner must count a
  non-empty `denied_actions` as a failed critic.
- **The guarantee rests on agy's defaults.** An allow rule under
  `permissions.allow` in agy's `settings.json` would let that action through.
  The owner's settings hold only `trustedWorkspaces`, which lists the
  agent-kit checkout; a packet under `.git/critique/` of that checkout lies
  inside a trusted workspace, and these checks ran outside one. The live
  smoke check (#178) must run the create-a-file test with the packet in its
  real location, and the tripwire stays the backstop.

agy also offers the model web and browser tools (`search_web`,
`read_url_content`, `open_browser_url`) and `call_mcp_tool`. None of them was
exercised. Whether they need permission headless, and so whether a hostile
packet could make a critic fetch a URL, is open and belongs in #171 and #178.

### `--json-schema`

The finding schema from #167 (findings with `id`, `severity`, `category`,
`location`, `claim`, `evidence`, optional `suggestion`, and
`no_findings_reason`) was passed as a file.

A cooperative review of a one-line spec ("The system must respond fast and
must never respond slower than it does today.") returned prose in `response`
followed by the JSON, took two turns (`num_turns: 2`), and carried a
`structured_output` object with five findings, every enum value valid:

```json
"structured_output": {"findings": [{"category": "untestable", "claim": "The term 'fast' is subjective, ...",
  "evidence": {"kind": "quote", "value": "fast"}, "id": "untestable-speed-metric",
  "location": "'The system must respond fast'", "severity": "major", "suggestion": "..."}, ...]}
```

Told to use the severity `blocker` and the category `style`, which the
schema does not allow, the critic complied in prose and agy returned no
`structured_output` at all, with exit 0 and `status: "SUCCESS"`. A third
run, told to ignore the output format and answer in prose, stalled until the
print timeout and returned an empty response. So agy validates: it never
hands back a `structured_output` that breaks the schema. It does not make the
model comply. The runner should read findings from `structured_output` only
and treat its absence as a failed critic, and should still validate it
against the schema itself.

### Request counts

agy reports no request count, quota or remaining allowance in print mode:
not in the JSON result and not in the `stream-json` events, and `agy --help`
lists no usage subcommand (`/usage` is interactive only). `num_turns`
counts model turns within the run, not quota requests; a structured-output
run reports 2. `cli.log` records quota refreshes (`doRefreshQuota`) without
numbers. The runner's own per-day invocation counter, which #167 already
specifies, is the only count available.

### Print timeout

`--print-timeout 3s` on a long prompt:

```text
exit=0
[agy] print timeout after 3s with turn in progress; returning partial output
{"status":"SUCCESS","response":"","duration_seconds":0,"num_turns":1,"usage":{"input_tokens":0,...},"finished_reason":"print_timeout"}
```

The print timeout stops the run, but exits 0 and reports `SUCCESS`; only
`finished_reason` shows it.

## Phase 0 facts

These were established before this ticket and are recorded in #167. The
CLI facts were re-confirmed here.

- PAL is v9.8.2. There has been no push since 2025-12-15, and clink rejects
  CLI names outside gemini, codex and claude (ADR-0001).
- Vibe 3.0.3 has the headless flags (`-p`, `--prompt-file`, `--output`), a
  tool allowlist (`--enabled-tools`, which in headless mode disables every
  unlisted tool), `--trust`, `--time-limit`, `--max-price` and
  `--output-dir`. It has no per-run model flag.
- agy 1.4.11 has print mode (`-p`), `--output-format json`, `--json-schema`,
  `--mode plan`, `--sandbox`, `--model`, `--print-timeout` and
  `--disable-slash-commands`. Its Gemini models are 3.8 Flash only.
- Pocock's skills are at 1.3.1.

## Deliverable map

The brief's deliverables, reconstructed from #167, and where each lands in
agent-kit. Tickets are the sub-issues of #167.

| Deliverable | Where it lives in agent-kit | Ticket |
|---|---|---|
| Critique bounded context (packet, fan-out, parsing, dedupe, evidence checks, tripwire, allowlist, depth, budget, ledger, report) | `council/ts/src/contexts/critique/`, beside the existing contexts; reuses the ports in `council/ts/src/ports/` and the shared kernel | #169 to #175 |
| Boundary: no import of council's cross-critique, design-vote or consolidation flow | rules in `council/ts/.dependency-cruiser.cjs` | #169 |
| Runner commands `prepare`, `fan-out`, `record`, `allow` | a CLI entry beside `council/ts/src/cli/`, bundled by `council/ts/scripts/build.mjs` into its own runner, pinned in `manifest.yaml` the way `council/council.mjs` is | #169 |
| Critic clients (binary, fixed arguments, model, family, cost reading) | the shipped routing configuration and the Vibe agent files; model names appear nowhere else | #169, #171 |
| Read-only Vibe critic agents `critic-<model>` | rendered from `registry/estate-tooling.yaml` into `registry/generated/vibe/`, installed by a new setup step into the personal Vibe home's `agents/` (`~/.vibe/agents/`), not `vibe-private`'s | #176 |
| Routing (stages, depths, lenses, caps, fallbacks, Antigravity quota, per-flow Vibe cap) | default file in the `critique` skill directory; optional per-machine override under `~/.config/agent-kit/` | #173, #174 |
| Prompts: shared preamble, `spec-redteam` role, eight lens files, finding schema | the `critique` skill directory (`skills/critique/`), hashed into the ledger | #169, #177 |
| Shared `critique` skill and user-invocable `critique-spec` skill | `skills/critique/` and `skills/critique-spec/`, as `local_skills` entries in `registry/estate-tooling.yaml` for claude, codex, vibe and antigravity | #176, #177 |
| Family allowlist | per machine, outside any repository, keyed by remote, under `~/.config/agent-kit/` beside the keep file | #169 |
| Run directories (packet, raw output, merged findings, report) | `critique/<run-id>/` inside the repository's git common directory | #169 |
| Ledger | one JSON-lines file per user in agent-kit's data directory (`~/.local/share/agent-kit/`) | #169 |
| Tracker and domain-doc resolution | inside the critique context, through the `gh` port | #175 |
| TypeScript tests (port fakes, 100% coverage, eslint, tsc, dependency-cruiser) | colocated `*.test.ts` in the critique context, run by `council/ts`'s `npm test` | every TS ticket |
| Python tests for the registry and setup render | `tests/test_registry_render.py` and a test beside `tests/test_vibe.py` for the critic-agent step | #176 |
| Live smoke check and seeded sample run | run by hand, outside CI; evidence added to this document | #178 |
| Integration document | this file | #168 |
| Glossary and ADR | `CONTEXT.md` "Critique pipeline", `docs/adr/0001-critics-run-as-direct-cli-calls.md` | #168 (done) |

The locations of the routing override, the allowlist and the ledger are
proposals that follow where agent-kit already keeps per-machine files (the
keep file and `env.sh` in `~/.config/agent-kit/`, marketplaces in
`~/.local/share/agent-kit/`). The ticket that builds each one settles the
exact path.

## Departures from the brief

Each of these is a decision in #167. The reasons are the ones recorded there
and in the grilling session behind it.

- **No PAL.** Critics are started as direct subprocesses (`vibe -p`,
  `agy -p`), not through PAL MCP's clink. clink rejects CLIs outside its
  three defaults, upstream is inactive, and it folds conversation history
  into the prompt, which works against blind critics. See ADR-0001.
- **TypeScript instead of Python.** The runner is a bounded context in
  council's TypeScript package rather than a separate Python program. It
  reuses council's process, git, gh, clock and env ports, their test fakes,
  the shared kernel, the bundling and pinning of a single runner file, and
  the 100% coverage and dependency-cruiser gates that already apply there.
- **No hooks in v1.** No Claude Code hooks ship (no commit gate, PR gate or
  session-start suggestion). The estate ships no agent hooks, and changing
  that needs its own ADR first.
- **`/flow` becomes a stepper merged with `/flow-guide`.** There is one
  entry point that steps through the flow and owns setup, explanation and
  budget, instead of two overlapping ones. It is out of scope for this
  slice; the only part delivered now is the opt-in, offered by
  `/critique-spec`.
- **Storage in the git directory.** Run directories live under the
  repository's git common directory, so they are never committed, need no
  `.gitignore` change, and are shared by every worktree of the repository.
- **Per-machine allowlist.** Which model families a repository may send to is
  kept on the owner's machine, outside the repository, so nothing about the
  pipeline is committed to client or shared repositories. A repository owned
  by the `nedap` GitHub organisation is always denied, and one with no remote
  counts as not opted in.
- **Depth replaces `--fast` and `--deep`.** A stage runs at base or deep
  depth, chosen from deterministic signals in the artifact (size, risk terms,
  a `risk:high` label), with an explicit argument always winning. The owner
  no longer has to judge it each time, and the choice and its signals are
  logged so the automatic rule can be checked.
- **Pocock 1.3.1 rather than v1.1.** The installed skills are 1.3.1. There
  `/to-spec` publishes the spec to the issue tracker rather than writing a
  file, so `/critique-spec` reads the spec and its tickets from the tracker
  (GitHub issues or local markdown tickets). `/to-spec`, `/to-tickets` and
  `/implement` are not model-invocable in 1.3.1, so the critique cannot call
  them; the owner runs `/to-spec`, then `/critique-spec`. Pocock's skills are
  not edited or forked.

## What this changes for the later tickets

- **#169 (runner tracer).** Unset `MISTRAL_API_KEY` and `VIBE_HOME` for Vibe
  critics. Pass `--output-dir` and read cost, outcome and errors from
  `export.json`; read the final message from the last assistant entry on
  stdout. List the packet files by path in the prompt. Never put `.vibe/`,
  `.agents/` or `AGENTS.md` at the packet root. Do not treat exit 2 as
  retryable without checking that the agent exists.
- **#171 (Antigravity critic).** Use `--disable-slash-commands --sandbox`
  without `--mode plan`. Read findings from `structured_output` only. Treat
  an empty `response`, any `denied_actions`, a missing `structured_output`
  or `finished_reason: print_timeout` as failed, whatever the exit code.
- **#173 (budget).** `--max-price` overshoots by up to one turn, a deadline
  stop reports no cost, and an unpriced model reports none. Budget from
  `export.json` with those gaps in mind, and count agy invocations yourself.
- **#176 (setup).** Each `critic-<model>` agent file carries its own priced
  `[[models]]` entry, `enabled_tools = ["read_file", "grep"]`,
  `outside_workdir = "deny"`, `enable_mcp = false`,
  `enable_subagents = false` and `[session_logging] enabled = false`, and
  must pass `VIBE_STRICT_CONFIG=1`.
- **#178 (live smoke check).** Repeat the create-a-file test for agy with the
  packet inside a trusted workspace's git directory, and check whether agy's
  web and MCP tools are refused headless.
