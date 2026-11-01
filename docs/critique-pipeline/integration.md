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
every design decision. The live checks below forced four departures from #167
itself. They are listed under [Amendments made to #167](#amendments-made-to-167),
and the owner approved them; #167 was amended on 2026-10-30. Where this
document and #167 disagree, #167 wins.

The checks were run on 2026-10-30 for agent-kit#168, on macOS, against
Vibe 3.0.3 (`~/.local/bin/vibe`, the `mistral-vibe` uv tool) and Antigravity
CLI 1.4.11 (`/opt/homebrew/bin/agy`).

## Summary

Most of what #167 assumes holds. Six things do not, and the tickets that
build on this one need to know:

- **Vibe's `--max-price` does not cap a single turn.** It is checked between
  turns. A one-turn run capped at $0.012 finished its turn and spent $0.034.
  Only `--time-limit` interrupts a turn in progress.
- **Vibe's cost is only as good as the prices in its config.** Vibe computes
  `cost_usd` from the `input_price` and `output_price` of the model entry.
  The model entries setup converges into the personal Vibe home carry no
  prices, so Mistral Large and DeepSeek Flash report $0.00 and the price cap
  never fires. A critic agent file has to carry its own priced model entry.
- **Vibe 3.0.3 offers no `grep` tool in headless mode.** Listing `grep` in
  `enabled_tools` is harmless but does nothing; a critic can read files and
  nothing else. The prompt has to name the packet files.
- **agy ignores `--mode plan` when `--disable-slash-commands` is set**, and
  says so on stderr. What keeps an agy critic read-only is that headless mode
  auto-denies every action that needs permission, and a denial ends the run
  with an empty response. That also holds with the working directory where a
  real packet lives, inside a trusted workspace.
- **`--disable-slash-commands` does not hide skills from an agy critic.** Run
  inside the agent-kit checkout, where packets live, a critic still had the
  checkout's workspace skills and agy's own slash commands listed in its
  context. Whether a critic can actually load one is not settled.
- **agy enforces the finding schema by omission, not by retry.** A response
  that fits the schema arrives as `structured_output`; one that does not is
  returned as prose with no `structured_output` field and exit code 0.

Everything else, including pinning the model in a Vibe agent file, is
confirmed below with the command and the output that shows it.

## How the checks were run

Every run in the first pass used a throwaway working directory under the
session scratchpad, never a repository checkout, and a trivial prompt with no
repository content (a one-line `note.txt` containing "The answer is 7.", and
an `outside.txt` next to the working directory containing a canary string). Each Vibe run had
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

Three later agy runs, made while reviewing this document, used the place a
real packet lives instead: `critique/_check168/` inside the git common
directory of the agent-kit checkout, which agy's settings list as a trusted
workspace. That directory held only `note.txt` and was deleted afterwards.
Before and after each of those runs, both the directory's SHA-1 listing and
`git status --porcelain` of the worktree the checks ran from were recorded.

Total live spend: Vibe recorded $0.115 in its `export.json` files. Two runs
used models with no configured price, one used placeholder prices, and one
was interrupted before its usage was counted (see the findings on cost), so
the true figure is somewhat higher, roughly $0.15. agy ran 14 times in print
mode against the Antigravity quota, which has no dollar cost. The two Vibe
argument errors quoted under exit codes were re-run for this record; Vibe
rejects them before it reads a key, so they cost nothing.

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

(`grep` in `enabled_tools` turned out to do nothing; see below.)

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
a `VIBE_` environment variable (its shipped documentation gives
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

Two runs had `VIBE_STRICT_CONFIG=1` set: a pong run with this agent file plus
`[session_logging] enabled = false` (saved as `critic-nolog`), and the
`--time-limit` run further down, which used `critic-glm` itself:

```sh
VIBE_STRICT_CONFIG=1 timeout 90 vibe -p "Reply with exactly the single word: pong" --output json \
  --max-price 0.02 --time-limit 60 --trust --agent critic-nolog --output-dir out
VIBE_STRICT_CONFIG=1 timeout 60 vibe -p "Write a detailed 4000-word essay on the history of bread, ..." \
  --output json --max-price 0.05 --time-limit 10 --trust --agent critic-glm --output-dir out
```

```text
critic-nolog  exit_code: 0  outcome: "finished"  config.strict_config: true  config.session_logging.enabled: false
critic-glm    exit_code: 3  outcome: "deadline"  config.strict_config: true
```

Neither was refused. The same setting does refuse an agent file with an
unknown key (see the exit codes below), so both files loaded clean.

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
- **Skills are stopped by the allowed-tool list, not by `enabled_skills`.**
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

The exit-1 cases:

```text
$ vibe -p pong --output bogus --trust
error: invalid value 'bogus' for '--output <OUTPUT>'
  [possible values: text, json, streaming]
exit=1
$ vibe -p pong --output json --max-price abc --trust
error: invalid value 'abc' for '--max-price <MAX_PRICE>': invalid float literal
exit=1
$ VIBE_STRICT_CONFIG=1 vibe -p pong --output json --max-price 0.01 --time-limit 20 \
    --trust --agent critic-strict --output-dir out      # agent file has no_such_setting = true
Error: Config refused because strict_config is on and it has settings Vibe does not use:
- `no_such_setting` is not a Vibe setting, in the active agent profile
exit=1
out/export.json: "outcome": "config_error", "exit_code": 1
```

The argument errors write no `export.json`; the config error does.

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
| DeepSeek Flash | `deepseek-v4-1-flash-1017` | `deepseek-v4-1-flash-1017` | exit 0, "pong", cost unpriced; exit 0 with a test-priced agent entry, read and reported `note.txt` |
| Mistral Large 4 | `mistral-large-4-0-3` | `mistral-large-4-0` | exit 0, "pong", cost unpriced |

The GLM run is the agent-file check above. DeepSeek Flash answered twice:
once as the personal home's default model with no agent file, and once
through the `critic-deepseek-flash` agent file shown earlier. Mistral Large
answered through an agent file like `critic-glm` with
`active_model = "mistral-large-4-0-3"`.

```sh
timeout 90 vibe -p "Reply with exactly the single word: pong" --output json --max-price 0.02 \
  --time-limit 60 --trust --enabled-tools read_file --enabled-tools grep --output-dir out
timeout 150 vibe -p "This is a tool test. ... 1) Read note.txt and quote its content. 2) Use grep ... 3) Call the skill tool ..." \
  --output json --max-price 0.03 --time-limit 120 --trust --agent critic-deepseek-flash --output-dir out
timeout 90 vibe -p "Reply with exactly the single word: pong" --output json --max-price 0.02 \
  --time-limit 60 --trust --agent critic-mistral-large --output-dir out
```

```text
default (DeepSeek Flash)  exit 0  model_name: "deepseek-v4-1-flash-1017"  final message: "pong"
                          cost_usd: 0.0  usage: {input_tokens: 7517, output_tokens: 4}
critic-deepseek-flash     exit 0  model_name: "deepseek-v4-1-flash-1017"  steps: 6
                          final message: "**Step 1 — read note.txt: `read_file` — succeeded** Content, quoted exactly: `The answer is 7.` ..."
                          cost_usd: 0.0163447  usage: {input_tokens: 49119, output_tokens: 1609, cached_input_tokens: 43456}
critic-mistral-large      exit 0  model_name: "mistral-large-4-0"  final message: "pong"
                          cost_usd: 0.0  usage: {input_tokens: 7081, output_tokens: 14}
```

DeepSeek Flash's real cost is not known from these runs. Through the home's
own entry it reports $0.00, and the $0.0163 comes from the placeholder
prices in the test agent file (0.3 and 1.0 per million tokens).

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
above. The `init` event has no field for skills; see
[Skills and slash commands](#skills-and-slash-commands).

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
  agent-kit checkout, so a packet under `.git/critique/` of that checkout
  lies inside a trusted workspace. The checks above ran outside one; the
  create-a-file check was repeated inside one (next section) with the same
  result. The tripwire stays the backstop.

### Inside a trusted workspace

The create-a-file check, with the working directory where a real packet
lives:

```sh
cd "$(git rev-parse --git-common-dir)/critique/_check168"     # holds only note.txt
timeout 180 agy -p "This is a permissions test. Use a tool to create a file named created.txt in the current directory containing the word hi. Then report the exact tool name you used and the exact result or error it returned." \
  --output-format json --model gemini-3.8-flash-high --print-timeout 150s \
  --disable-slash-commands --sandbox < /dev/null
```

```text
exit=0 elapsed=8s
before: 71fd2c785f329a52357d72ee3329b5fd8728d6a3  ./note.txt
after:  71fd2c785f329a52357d72ee3329b5fd8728d6a3  ./note.txt
git status --porcelain: empty before, empty after
stderr: jetski: no output produced — a tool required the "write_file" permission that headless mode cannot prompt for, so it was auto-denied. ...
{"status":"SUCCESS","response":"","duration_seconds":4.430965,"num_turns":1, ... "denied_actions":[{"action":"write_file","display_name":"WriteFile"}]}
```

Being inside a trusted workspace does not lift the headless denial: agy
refused the write exactly as it did outside one. Whether a critic there may
read files outside its working directory but inside the workspace (the
repository's own working tree) was not checked; it belongs in #178.

### Skills and slash commands

`agy --help` describes the flag as "Disable slash command and skill expansion
in print mode". To see what that leaves a critic, agy ran twice more in the
same directory. First a trivial prompt with `stream-json`, to capture the
`init` event:

```sh
timeout 120 agy -p "Reply with exactly the single word: pong" --output-format stream-json \
  --model gemini-3.8-flash-high --print-timeout 90s --disable-slash-commands --sandbox < /dev/null
```

```text
{"event":"init", ... "init":{"model":"gemini-3.8-flash-high",
 "cwd":".../agent-kit/.git/critique/_check168",
 "tools":["ask_custom_permission","ask_permission","ask_question","browser_click_element", ...
   "call_mcp_tool", ... "define_subagent", ... "find_by_name","finish","generate_image","grep_search",
   "invoke_subagent", ... "list_dir", ... "read_url_content","replace_file_content","run_command",
   "run_workflow","schedule","search_marketplace","search_web","sed_file", ... "view_file","wait",
   "wait_5_seconds","write_to_file"],
 "permission_mode":"request-review"}}
...
{"event":"result","result":{ ... "status":"SUCCESS","response":"pong\n", ... "usage":{"input_tokens":9370, ...}}}
```

The `init` event has only `model`, `cwd`, `tools` and `permission_mode`. The
60 tools are the same list as the earlier `stream-json` run outside the
checkout, and none of them is named for skills (`run_workflow` and
`invoke_subagent` were not examined). So the event cannot show whether
skills are available. The same pong prompt took 9,370 input tokens
here against 7,656 outside the checkout, which suggests the workspace adds
to the system prompt. Then the critic was asked what it had been told about:

```sh
timeout 120 agy -p "Do not use any tools. List the exact names of every skill, workflow or slash command that your instructions or context tell you about, one per line. If there are none, answer: none." \
  --output-format json --model gemini-3.8-flash-high --print-timeout 90s --disable-slash-commands --sandbox < /dev/null
```

```text
exit=0  directory and git status unchanged
"response":"add-public-service\nagent-session-bootstrap\nagy-customizations\nantigravity-guide\nclaude-worker\n
  council\ncouncil-amend\n ... speckit-taskstoissues\ntoken-economy\n/goal\n/compact\n/schedule\n/browser\n/plan\n
  /grill-me\n/learn\n"
```

Apart from `agy-customizations` and `antigravity-guide`, which agy ships,
those are exactly the skills in the checkout's `.agents/skills/`. So
`--disable-slash-commands` stops a slash command in the prompt from being
expanded, but it does not take skills out of a critic's context, and a
critic run under `.git/critique/` sees the enclosing repository's skills
whatever the packet holds. Once setup installs `critique` and
`critique-spec`, they will be listed too. What is left open is whether a
critic could load one anyway, through `run_workflow` or by reading the
skill's `SKILL.md` with `view_file`; the second is the same open question as
reading outside the working directory above. Until #171 or #178 settles it,
#167's user story 23 (critics cannot load skills or slash commands) is not
shown to hold for agy.

agy also offers the model web and browser tools (`search_web`,
`read_url_content`, `open_browser_url`) and `call_mcp_tool`. None of them was
exercised. Whether they need permission headless, and so whether a hostile
packet could make a critic fetch a URL, is open and belongs in #171 and #178.

### `--json-schema`

The finding schema from #167 (findings with `id`, `severity`, `category`,
`location`, `claim`, `evidence`, optional `suggestion`, and
`no_findings_reason`) was passed as a file. Each run used the same flags:

```sh
timeout 120 agy -p "<prompt>" --output-format json --model gemini-3.8-flash-high \
  --print-timeout 90s --disable-slash-commands --sandbox --json-schema finding-schema.json < /dev/null
```

The three prompts, in the order of the results below, were:

```text
1. Do not use any tools. Review this one-line specification as a critic and report defects with a quote as
   evidence: 'The system must respond fast and must never respond slower than it does today.'
2. Do not use any tools. Report exactly one finding about the sentence 'The answer is 7.' and give it the
   severity value blocker (not critical, major or minor) and the category value style.
   (--print-timeout 75s)
3. Do not use any tools. Ignore any output format you were given and answer in plain prose, not JSON: is the
   sentence 'The answer is 7.' a testable requirement? Use severity value 'blocker' if you report anything.
   (--print-timeout 120s)
```

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
`structured_output` at all, with exit 0 and `status: "SUCCESS"`:

```text
exit=0
{"status":"SUCCESS","response":"### Finding 1\n\n* **ID**: `FINDING-001`\n* **Location**: `'The answer is 7.'`\n
  * **Severity**: blocker\n* **Category**: style\n ...","num_turns":2,"json_schema":{...},"usage":{...}}
(no "structured_output" key)
```

A third run, told to ignore the output format and answer in prose, stalled
until the print timeout and returned an empty response:

```text
exit=0
stderr: [agy] print timeout after 2m0s with turn in progress; returning partial output
{"status":"SUCCESS","response":"","duration_seconds":0,"num_turns":1,"json_schema":{...},
 "usage":{"input_tokens":0,...},"finished_reason":"print_timeout"}
```

So agy validates: it never hands back a `structured_output` that breaks the
schema. It does not make the model comply. The runner should read findings
from `structured_output` only and treat its absence as a failed critic, and
should still validate it against the schema itself.

### Request counts

agy reports no request count, quota or remaining allowance in print mode:
not in the JSON result and not in the `stream-json` events, and `agy --help`
lists no usage subcommand (`/usage` is interactive only):

```text
$ agy --help
...
Available subcommands:
  agent           List available agents
  agents          List available agents
  changelog       Show changelog and release notes
  help            Show help for subcommands
  install         Configure environment paths and shell settings
  mcp             Manage MCP servers (add, remove, list, enable, disable)
  mic-serve       Serve this machine's microphone to a CLI on another host
  models          List available models
  plugin          Manage plugins (install, uninstall, list, enable, disable)
  plugins         Alias for plugin
  remote-control  Manage the remote-control background daemon (start, status, stop)
  update          Update CLI
```

`num_turns` counts model turns within the run, not quota requests; a
structured-output run reports 2. `cli.log` records quota refreshes
(`doRefreshQuota`) without numbers. The runner's own per-day invocation counter, which #167 already
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
CLI facts were re-confirmed here, from the runs above and from each CLI's
`--help`.

- PAL is v9.8.2; [ADR-0001](../adr/0001-critics-run-as-direct-cli-calls.md)
  records what was found about it.
- Vibe 3.0.3 has the headless flags (`-p`, `--prompt-file`, `--output`),
  allowed tools (`--enabled-tools`, which in headless mode disables every
  unlisted tool), `--trust`, `--time-limit`, `--max-price` and
  `--output-dir`. It has no per-run model flag. Every run here passed the
  prompt with `-p`. `--prompt-file`, which #167 names for the Vibe critics,
  was not exercised; `vibe --help` lists it:

  ```text
  -p, --prompt [<PROMPT>]
          Run in headless (programmatic) mode: send prompt, output response, exit
      --prompt-file <PATH>
          Run in programmatic mode with the prompt read from PATH, instead of -p TEXT or stdin
  ```
- agy 1.4.11 has print mode (`-p`), `--output-format json`, `--json-schema`,
  `--mode plan`, `--sandbox`, `--model`, `--print-timeout` and
  `--disable-slash-commands`. Its Gemini models are 3.8 Flash only.
- Pocock's skills are at 1.3.1.

## Deliverable map

The brief's deliverables, reconstructed from #167, and where each lands in
agent-kit. Tickets are the sub-issues of #167.

| Deliverable | Where it lives in agent-kit | Ticket |
|---|---|---|
| Critique bounded context (packet, fan-out, parsing, dedupe, evidence checks, tripwire, family allowlist, depth, budget, ledger, report) | `council/ts/src/contexts/critique/`, beside the existing contexts; reuses the ports in `council/ts/src/ports/` and the shared kernel | #169 (packet, parsing, family allowlist, ledger, report), #170 (evidence checks), #171 (parallel fan-out, dedupe), #172 (tripwire), #173 (budget), #174 (depth), #175 (tracker) |
| Boundary: no import of council's cross-critique, design-vote or consolidation flow | rules in `council/ts/.dependency-cruiser.cjs` | #169 |
| Runner commands `prepare`, `fan-out`, `record`, `allow` | a CLI entry beside `council/ts/src/cli/`, bundled by `council/ts/scripts/build.mjs` into its own runner, pinned in `manifest.yaml` the way `council/council.mjs` is | #169 |
| `record`'s output: the compact summary, the full report, and Dissent | written by `record` in the critique context; the report goes into the run directory, the summary is returned to the skill | #169 (summary, report), #170 (minor findings batched per critic), #171 (Dissent) |
| Critic clients (binary, fixed arguments, model, family, cost reading) | the shipped routing configuration and the Vibe agent files; model names appear nowhere else | #169, #171 |
| The agy client's model | agy has no agent file, so `--model gemini-3.8-flash-high` is one of the client's fixed arguments in the shipped routing configuration in `skills/critique/` | #171 |
| Read-only Vibe critic agents `critic-<model>` | rendered from `registry/estate-tooling.yaml` into `registry/generated/vibe/`, installed by a new setup step into the personal Vibe home's `agents/` (`~/.vibe/agents/`), not `vibe-private`'s | #176 |
| Routing (stages, depths, lenses, caps, fallbacks, Antigravity quota, per-flow Vibe cap) | default file in the `critique` skill directory; optional per-machine override under `~/.config/agent-kit/` | #173, #174 |
| Prompts: shared preamble, `spec-redteam` role, eight lens files, finding schema | the `critique` skill directory (`skills/critique/`), hashed into the ledger | #169, #177 |
| Shared `critique` skill and user-invocable `critique-spec` skill | `skills/critique/` and `skills/critique-spec/`, as `local_skills` entries in `registry/estate-tooling.yaml` for claude, codex, vibe and antigravity | #176, #177 |
| `critique-spec`'s frontmatter description (applies after `/to-spec` produced or changed a spec) | the frontmatter of `skills/critique-spec/SKILL.md` | #177 |
| Acting on spec findings (user stories 35 to 40): edits to the spec issue body, the one change comment, the second round, the warning not to start `/to-tickets` | the `critique-spec` skill text, writing through `gh`; a second round is a second prepare and fan-out | #177 |
| Offering the opt-in when a repository has no family allowlist entry (user story 50) | the `critique-spec` skill, recording the owner's choice through `allow` | #177 |
| Saying clearly when a critic CLI is missing or not signed in, and offering to continue without it (user story 68) | detection in fan-out, as a critic status of its own; the offer in the `critique-spec` skill | #173 (a stage routed to an unavailable `agy` falls back and the summary names the reason), #177 (the missing or signed-out CLI is reported in setup's wording, with the offer to continue) |
| Family allowlist | per machine, outside any repository, keyed by remote, under `~/.config/agent-kit/` beside the keep file | #169 |
| Run directories (packet, raw output, merged findings, report) | `critique/<run-id>/` inside the repository's git common directory | #169 |
| Ledger | one JSON-lines file per user in agent-kit's data directory (`~/.local/share/agent-kit/`) | #169 |
| Tracker and domain-doc resolution | inside the critique context, through the `gh` port | #175 |
| TypeScript tests (port fakes, 100% coverage, eslint, tsc, dependency-cruiser) | colocated `*.test.ts` in the critique context, run by `council/ts`'s `npm test` | every TS ticket |
| Python tests for the registry and setup render | `tests/test_registry_render.py` and a test beside `tests/test_vibe.py` for the critic-agent step | #176 |
| Live smoke check and seeded sample run | run by hand, outside CI; evidence added to this document | #178 |
| Integration document | this file | #168 |
| Glossary and ADR | `CONTEXT.md` "Critique pipeline", `docs/adr/0001-critics-run-as-direct-cli-calls.md` | #168 (done) |

The locations of the routing override, the family allowlist and the ledger
are proposals that follow where agent-kit already keeps per-machine files
(the keep file and `env.sh` in `~/.config/agent-kit/`, marketplaces in
`~/.local/share/agent-kit/`). The ticket that builds each one settles the
exact path.

## Departures from the brief

Each of these is a decision in #167. The reasons are the ones recorded there
and in the grilling session behind it.

- **No PAL.** Critics are started as direct subprocesses, not through PAL
  MCP's clink, for the reasons in
  [ADR-0001](../adr/0001-critics-run-as-direct-cli-calls.md).
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
- **Per-machine family allowlist.** Which families a repository may send to
  is decided by the **Family allowlist** as defined in
  [CONTEXT.md](../../CONTEXT.md), kept on the owner's machine so nothing
  about the pipeline is committed to client or shared repositories.
- **Depth replaces `--fast` and `--deep`.** A stage runs at base or deep
  depth, chosen from deterministic signals in the artifact (size, risk terms,
  a `risk:high` label), with an explicit argument always winning. The owner
  no longer has to decide it each time, and the choice and its signals are
  logged so the automatic rule can be checked.
- **Pocock 1.3.1 rather than v1.1.** The installed skills are 1.3.1. There
  `/to-spec` publishes the spec to the issue tracker rather than writing a
  file, so `/critique-spec` reads the spec and its tickets from the tracker
  (GitHub issues or local markdown tickets). `/to-spec`, `/to-tickets` and
  `/implement` are not model-invocable in 1.3.1, so the critique cannot call
  them; the owner runs `/to-spec`, then `/critique-spec`. Pocock's skills are
  not edited or forked.

### Amendments made to #167

The live checks forced these departures from #167 itself. The owner approved
all four, and #167 was amended on 2026-10-30 to match.

1. **agy runs without plan mode.** #167 has the Antigravity client run in
   "plan mode plus sandbox", and counts plan mode as one of the CLI flags
   that keep a critic read-only. agy ignores `--mode plan` when
   `--disable-slash-commands` is set, and says so on stderr, so a critic
   cannot have both. The agy client should drop `--mode plan`. Read-only
   then rests on headless mode's automatic denial of every action that needs
   permission, shown above both outside and inside a trusted workspace.
2. **Vibe critics can read but not search.** #167 gives the Vibe clients an
   allowed-tool list of `read_file` and `grep`, and user story 19 asks for a
   tool set that can "only read and search". Vibe 3.0.3 has no `grep` tool in
   headless mode, and every way it offers to search runs through `bash`,
   which a critic must not have. A Vibe critic can only `read_file` the paths
   it is given, so user story 19 narrows to "only read" for Vibe critics, and
   the prompt lists the packet's files. (agy's critic does keep search tools:
   `grep_search`, `find_by_name` and `list_dir`.)
3. **The allowed-tool list keeps skills from a Vibe critic, not
   `enabled_skills`.** #167 has each `critic-<model>` agent "disable skills".
   The skills stay listed whatever `enabled_skills` says; what stops a
   critic loading one is that `skill` is left out of `enabled_tools`. The
   agent files should rely on that and may keep `enabled_skills` as a second
   setting. For agy, user story 23 is open (see
   [Skills and slash commands](#skills-and-slash-commands)).
4. **Each critic agent file carries its own priced model entry.** #167 pins
   the model through the agent file, assuming the model entries are already
   in the personal Vibe home. Those entries have no prices, so cost reads
   $0.00 and neither the per-critic price cap nor the per-flow cap can work.
   Each `critic-<model>` file needs its own `[[models]]` entry with the real
   input and output prices, which makes the agent files, next to routing, a
   place where model names and prices are kept (user story 65 already counts
   the critic client definitions as one).

## What this changes for the later tickets

- **#169 (runner tracer).** Unset `MISTRAL_API_KEY` and `VIBE_HOME` for Vibe
  critics. Pass `--output-dir` and read cost, outcome and errors from
  `export.json`; read the final message from the last assistant entry on
  stdout. List the packet files by path in the prompt. Never put `.vibe/`,
  `.agents/` or `AGENTS.md` at the packet root. Do not treat exit 2 as
  retryable without checking that the agent exists.
- **#171 (Antigravity critic).** Use `--disable-slash-commands --sandbox`
  without `--mode plan` (amendment 1). Read findings from
  `structured_output` only. Treat an empty `response`, any `denied_actions`,
  a missing `structured_output` or `finished_reason: print_timeout` as
  failed, whatever the exit code. Expect the critic to see the repository's
  skill names.
- **#173 (budget).** `--max-price` overshoots by up to one turn, a deadline
  stop reports no cost, and an unpriced model reports none. Budget from
  `export.json` with those gaps in mind, and count agy invocations yourself.
- **#176 (setup).** Each `critic-<model>` agent file carries its own priced
  `[[models]]` entry (amendment 4), `enabled_tools = ["read_file"]`,
  `outside_workdir = "deny"`, `enable_mcp = false` and
  `enable_subagents = false`. Two further settings are worth considering:
  `[session_logging] enabled = false`, which shows up in the effective config
  but has not been shown to stop Vibe writing session history, and loading
  each rendered file under `VIBE_STRICT_CONFIG=1` in a test, which catches a
  misspelt key.
- **#178 (live smoke check).** The agy create-a-file test inside a trusted
  workspace is done (above). Still to check: whether agy's web and MCP tools
  are refused headless, and whether an agy critic under `.git/critique/` can
  read files outside its working directory but inside the repository,
  including a skill's `SKILL.md`, or start a skill through `run_workflow`.

## What #169 settled

The runner tracer (agent-kit#169) fixed these, and the later tickets build on
them:

- **Paths.** The family allowlist is `~/.config/agent-kit/critique-allowlist.json`,
  keyed by `host/owner/repo` of the `origin` remote (or the first remote when
  there is no `origin`). The ledger is
  `~/.local/share/agent-kit/critique/ledger.jsonl`. Each run lives in
  `critique/<run-id>/` in the git common directory, with `packet/`,
  `manifest.json`, `critics/<critic-id>/` (prompt, stdout, stderr and the
  export directory), `findings.json`, `report.md` and `summary.md`.
- **Packet layout.** The artifact goes to `packet/artifact/` and each
  reference to `packet/references/<repository path>`. Nothing else is put at
  the packet root, so it can never hold a `.vibe/`, `.agents/` or `AGENTS.md`.
  The prompt is written outside the packet and passed with `--prompt-file`.
- **The `nedap` deny** matches the owner segment of any remote on any host,
  so a host alias such as `git@github-work:nedap/x.git` is denied too.
- **Vibe isolation.** The runner launches
  `env -u MISTRAL_API_KEY -u VIBE_HOME vibe ...` with `--enabled-tools
  read_file`, and sets `VIBE_ENABLE_MCP`, `VIBE_ENABLE_SUBAGENTS` and
  `VIBE_INCLUDE_PROJECT_CONTEXT` to `false` and `VIBE_OUTSIDE_WORKDIR` to
  `deny`. Vibe's environment layer sits above the home's `config.toml` and
  below the agent file. `include_project_context = false` is what keeps
  `~/.vibe/AGENTS.md` out: in Vibe 3.0.3's source the user and project
  `AGENTS.md` section is only built when it is true. After the run, fan-out
  reads the effective config from `export.json` and records the critic as
  failed with `unsafe-config` unless MCP is off, the tools are exactly
  `read_file`, `include_project_context` is false and `outside_workdir` is
  `deny`. So a critic that does see the MCP fleet or the instructions
  pointer never counts as a review. The `critic-<model>` agent files (#176)
  should set the same keys. None of this has been run against the real
  CLI yet; #178's smoke check covers it.
- **Ledger reasons.** The ledger keeps a verdict's reason only as a short
  slug (`reason_code`); the free-text reason goes to the report, because it
  may quote the artifact.
- **Ports.** The critique context uses council's own ports for the clock,
  the environment, processes and git. Its git port is council's `GitPort`
  narrowed to `root`, `commonDir` and `remotes`; the last two were added to
  `GitPort` and to graph's `GitCliAdapter`, which the runner wires. Two
  departures are deliberate. Council has no file-system port (its run store
  reads and writes files through its own adapters), so critique keeps a
  small `CritiqueFilesPort`. And council has no shared port fakes: each of
  its tests declares its own. The critique harness
  (`council/ts/test/critique/harness.ts`) therefore holds the fakes, written
  against council's port interfaces, and later tickets should add to it
  rather than write new ones.
- **Critic clients.** Each critic client is an adapter behind
  `CriticClientPort` (`review(launch)` returns the raw output and a
  `CriticOutcome`), chosen by the client's `kind` in routing. Everything
  Vibe-specific is in `VibeCriticClient`; the Antigravity client (#171) is a
  second adapter and a second `kind`. A dependency-cruiser rule keeps the
  critique domain free of ports, the application layer and adapters.
- **Degraded runs.** Only the family allowlist refuses: no remote, no entry,
  or a `nedap` remote. A stage whose roster drops every critic still
  prepares a run with no critics; fan-out then launches nothing and marks
  the run degraded, as it does when every critic fails, and record's
  summary and report open with `degraded: no critique ran`. #173 adds
  fallbacks before that point, not a refusal.
- **Distinct families.** When a stage sets `distinct_families`, a critic
  whose family an earlier critic of the stage already has is dropped with
  the reason `same-family`. #171's fallbacks should be tried before that
  drop.
- **Errors and refusals.** A refusal prints `"status": "refused"` and exits
  0. Anything else that stops a command is an error with a `code`
  (`invalid-input`, `invalid-allowlist`, `invalid-routing`, `invalid-run`,
  `outside-repository`, `already-recorded`, or `unexpected`) and exits 1.
  Every JSON the runner reads, from a command's input to its own run files,
  is checked field by field, so a malformed allowlist entry fails closed.
- **Stages.** The stage names are `decisions`, `spec`, `tickets`,
  `ticket-review` and `feature-review`, and prepare rejects any other.
  Shipped routing defines `spec` only.

## What #170 settled

The mechanical evidence checks (agent-kit#170) fixed these:

- **Where the check runs.** Fan-out reads every packet file before it
  launches a critic, so a critic that writes into the packet cannot plant a
  quote. After the critics finish, `checkEvidence` (in the critique domain)
  gives each finding an optional `pre_verdict`. It runs on the list of
  findings just before `findings.json` is written; dedupe (#171) belongs
  before it. A finding merged from several critics should keep every
  critic's evidence, so one fabricated quote rejects the merged finding.
- **Fabricated evidence.** Every `quote` must occur in some packet file, or
  the finding gets `{ verdict: "rejected", reason_code:
  "fabricated-evidence" }`. Runs of white space count as one space on both
  sides, so a quote may cross a line break of a wrapped file; nothing else is
  normalised. A quote's `file` is not trusted: which side a quote is on is
  decided by where it actually occurs.
- **Hints.** A finding with no evidence is a hint (`no-evidence`). At the
  spec stage, a quote found in `packet/artifact/` counts as a spec quote and
  one found in `packet/references/` as a decision quote; an `input`,
  `failing-test` or `trace` counts as a scenario. An omission or invention
  without both quotes is a hint (`no-spec-quote` or `no-decision-quote`);
  any other category without a spec quote, or without a decision quote or a
  scenario, is a hint (`no-spec-quote` or `no-decision-or-scenario`). Other
  stages have no rule yet beyond `no-evidence`; their tickets add one.
- **Record.** A finding with a pre-verdict needs no verdict. A
  fabricated-evidence rejection stands: a verdict other than `rejected` is
  an `invalid-input` error, and a `rejected` one keeps the
  `fabricated-evidence` code. A hint may become `accepted` or `rejected`;
  `needs-human` is an error. The report shows an overruled pre-verdict, and
  each ledger finding line carries `pre_verdict` (the code, or `null`).
- **Minor findings.** The summary has a `## Minor` section between the
  questions and Dissent, with one line per critic that raised a minor
  finding: `- <critic id>: <n> minor: accepted F2; needs-human F3; hint F4`.
  Minor findings no longer appear under Accepted or Questions. Dissent is
  unchanged, so a rejected minor finding from two families is still listed
  there. A fabricated-evidence rejection of a critical finding also counts
  as dissent under the current rule; #171 may want to exclude it.

## What #171 settled

The Antigravity critic and the parallel fan-out across families
(agent-kit#171) fixed these. The agy facts were checked on 2026-11-01 against
agy 1.4.11 on macOS, in a scratch directory outside any trusted workspace.

- **Launch.** `AgyCriticClient` runs, with the packet as working directory
  and stdin closed:

  ```sh
  agy -p "<prompt>" --output-format stream-json --model gemini-3.8-flash-high \
    --print-timeout <time_limit_s>s --disable-slash-commands --sandbox \
    --json-schema <assets>/schemas/finding.schema.json \
    --settings '{"permissions":{"deny":["mcp(*)","read_url(*)","execute_url(*)","command(*)","write_file(*)"]}}'
  ```

  There is no `--mode`, `--continue`, `--conversation`, `--project` or
  `--prompt-interactive`, so every critic is a fresh session. agy has no
  prompt-file flag; the prompt is passed as the `-p` argument. The outer
  timeout is the print timeout plus 60 seconds.
- **`stream-json`, not `json`.** The ticket says JSON output; the client uses
  `stream-json` because only the event stream shows which tools a critic
  used. Each tool call is a `step_update` with `step_type: "tool"` and a
  `tool_name`. The final `result` event carries the same object as `json`
  output, `structured_output` included. The client saves it as
  `critics/<id>/result.json`, which is the critic's `export_path`.
- **Session-only settings.** `--settings` takes `settings.json` keys and
  never saves them. A deny rule beats any allow rule in the owner's
  `settings.json`, so the read-only guarantee no longer rests on agy's
  defaults alone. A denied tool returns an error to the model and the run
  goes on: with `read_file(*)` denied the critic answered "access was denied
  by a user-configured deny rule", and `denied_actions` stayed empty. Only
  headless auto-denial fills `denied_actions`. The `mcp(*)` rule was not
  tried against a real MCP server: this machine's `mcp_config.json` is empty.
- **The gap: MCP servers and global instructions still load.** agy's CLI
  reference lists 33 settings keys. None of them turns off loading the MCP
  servers in `~/.gemini/config/mcp_config.json`, the global
  `~/.gemini/config/AGENTS.md` or skills. Rules files are also found by
  walking up from the working directory to the repository root, so a
  packet under `.git/critique/` gets that repository's root `AGENTS.md` or
  `GEMINI.md`. agent-kit has none at its root; other repositories may.
- **So the runner allows read-only tools only.** A critic may use
  `read_file`, `view_file`, `list_dir`, `grep_search`, `find_by_name` and
  `finish`, through which agy returns structured output. Any other tool in
  the stream fails the critic as `unsafe-tool`, even if agy refused it. That
  covers `call_mcp_tool`, the browser tools, `read_url_content`,
  `run_command`, `run_workflow` and `invoke_subagent`, so an agy critic can no
  longer start a skill through a workflow or a subagent. It also covers
  `search_web`, which is not permission-gated: it ran with `read_url(*)`
  denied, so only this check stops a hostile packet from making a critic
  search the web.
- **Failures.** In this order, whatever the exit code: a non-zero exit
  (`exit-<n>`; the outer timer's kill gives `exit-124`), empty stdout
  (`empty-output`), no `result` event or a line that is not JSON
  (`unparseable-output`), a tool outside the list (`unsafe-tool`),
  `finished_reason: print_timeout` (`print-timeout`), a non-empty
  `denied_actions` (`denied-action`), an empty `response` (`empty-output`), no
  `structured_output` (`missing-structured-output`), and a `structured_output`
  that breaks the output contract (the Vibe findings failures). Findings are
  read from `structured_output` only. Cost is always null.
- **Live run.** The real prompt (preamble, `spec-redteam`, the failure-modes
  and security lenses, the contract with the schema, the file list) and the
  flags above, on a two-file packet: exit 0 after 114 s, two `read_file`
  calls and nothing else, a schema-valid `structured_output` with the planted
  contradiction rated critical, and the packet unchanged. That stdout,
  replayed through fan-out, gives a critic with status `ok`.
- **Readiness.** `CriticClientPort` gained `readiness(config)`, and
  `review` takes the client config narrowed to its kind
  (`CriticClientPort<Config>`; `CriticClients` maps each kind to its port).
  Prepare asks every client the stage could run, routed or fallback, whose
  family the allowlist permits and the orchestrator does not share. agy is
  ready only on macOS, with `agy` on `PATH` (or at its absolute path) and an
  `Antigravity CLI` Keychain item (`security find-generic-password -s
  "Antigravity CLI"`; if `security` cannot run, it counts as not signed in).
  The details are `antigravity: macOS only; not set up on <platform>`,
  `antigravity: agy is not on PATH` and setup's
  `antigravity: needs sign-in: run agy and sign in with your Google account`.
  No readiness check launches agy, so #173's daily counter is untouched.
  Vibe's readiness always says ready; a missing Vibe still fails at launch.
- **Routing and fallbacks.** Routing gained a top-level
  `fallbacks: { client: [client, ...] }`. Shipped: `vibe-mistral-large` falls
  back to `vibe-glm`, then `agy-gemini-flash-high`; `agy-gemini-flash-high`
  falls back to `vibe-glm` (agent `critic-glm`, model `glm-5-3`, family
  `zhipu`, installed by #176). An `agy` client must have family `google` and
  a `gemini-` model, so routing can never send a critique to agy's Claude or
  GPT-OSS models. agy ignores `max_price_usd`; the Gemini slot's $0.50 applies
  when GLM takes it.
- **Roster.** Each slot runs its client, or else the first fallback that may
  run, with the slot's role, lenses, cap and time limit. A client may not run
  when its family is not allowed, is the orchestrator's, is already in the
  stage (with `distinct_families`) or is not ready. The slot's routed client
  is then listed in `dropped` with its `reason`, a `detail` when it was
  unavailable, and `replaced_by`, the fallback's critic id, when one took the
  slot. A fallback that also could not run is not listed. The summary says
  `replaced by <critic id>`, or `no fallback left, so the stage runs with
  fewer critics`. #173's retry of a failed critic can use the same
  `fallbacks` map.
- **Parallel and blind.** Fan-out already started every critic at once; a
  test now shows both critics start before either ends, that neither prompt
  holds the other's output, a model name or a critic id, and that no
  continuation flag is passed.
- **Merging.** `mergeFindings` runs over every critic's findings before
  anything else, so #170's evidence checks see a merged finding with all of
  its evidence. Two findings merge when their category matches, their
  locations match (ignoring case, spacing, surrounding quotes and trailing
  punctuation) and they share a quote, one containing the other. Findings
  with no quotes merge only when their claims match the same way. The
  merged finding keeps the first one's fields and `critic_finding_id`, every
  critic in `raised_by`, the union of the evidence, the highest severity and
  the first suggestion. Findings are numbered `F1`, `F2`, ... after merging.
- **Agreement and dissent.** A summary line names the families when two or
  more raised the finding (`(raised by mistral and google)`), and the report
  lists them after the critic ids. Dissent was already computed by record
  (rejected and raised by two or more families, or rejected and critical);
  merging is what lets two families raise one finding. A rejection with the
  reason code `fabricated-evidence` is never dissent: the evidence does not
  exist, and no orchestrator ruled against it. Because merging runs before
  the evidence checks, one fabricated quote from either critic rejects the
  merged finding.
- **Minor lines after merging.** A merged minor finding appears on the minor
  line of every critic that raised it.
- **With the tripwire (#172).** Every critic of the stage, a fallback that
  took a slot included, has its own critic id and so its own packet copy at
  `critics/<id>/packet/`; the tripwire is armed just before `reviewWith` and
  checked just after, per critic, while the critics run in parallel. A
  `violated-readonly` critic gets no fallback: it stays failed and the
  summary warns about it, because a client that wrote once may write again.
  #171's fallbacks are chosen in prepare only; fan-out never relaunches. If
  #173 adds a retry in fan-out, each attempt needs its own critic id and
  packet copy, and a violation should not be retried.

## What #175 settled

Tracker and domain-doc resolution (agent-kit#175) fixed these:

- **Tracker kind.** Prepare reads the `# Issue tracker: <name>` heading of
  `docs/agents/issue-tracker.md`, the file Pocock's setup writes. `GitHub`
  and `Local Markdown` are read; any other name (GitLab, or the free-form
  prose the setup writes for Jira or Linear), or a doc with no such heading,
  is a refusal with the reason `unsupported-tracker`. With no doc, the
  tracker is GitHub when the allowlist key is on `github.com`, and local
  markdown otherwise.
- **What `spec` means.** The tracker decides it. With GitHub, `spec` is an
  issue reference: `167`, `#167`, `owner/repo#167` or an issue URL. With
  local markdown, it is a repository path. A reference of the wrong kind is
  an `invalid-input` error. An issue of any repository other than the one
  the allowlist is keyed by is an `outside-repository` error, so a spec from
  another repository (a `nedap` one, say) cannot ride on this repository's
  allowlist entry.
- **Reading an issue.** Council's `GhPort` gained `viewIssue`, which the
  critique narrows to `CritiqueGhPort`; the real adapter is
  `src/adapters/gh/` (`gh issue view <n> --repo <allowlist key> --json ...`).
  `--repo` is always passed, so gh never chooses the repository from its own
  remote rules. Any gh failure, an issue gh cannot find included, is a
  refusal with the reason `tracker-unreachable` and a one-line message that
  carries gh's first line of output, returned before any run directory
  exists. The harness's `FakeGh` serves issues by `<repo>#<number>`.
- **Comments.** An issue's comments are frozen only when prepare's input
  has `"comments": true`; the default is the body alone. Comments can hold
  the change summary `/critique-spec` posts after a round (#177), so leaving
  them out keeps one round's results from reaching the next round's critics.
- **The frozen issue.** `packet/artifact/issue-<n>.md` holds `# <title>`,
  the body and, when asked, `## Comments` with one `### <login>, <date>`
  section each. Line endings become `\n`, so a quote copied from the packet
  matches it.
- **Where the artifact came from.** The manifest and prepare's result carry
  `artifact: { tracker, reference, labels }`. `reference` is
  `host/owner/repo#<n>` for an issue and the repository path for a file; it
  is also the artifact's `source` in `packet_files`. `labels` holds the
  issue's labels, which #174's `risk:high` signal can read without calling
  gh again.
- **Decision docs.** With `docs/agents/domain.md`, the references are the
  paths it names in inline code or link targets outside code blocks: a file
  as itself, a directory as the `.md` files directly inside it, and a missing
  path as nothing. Placeholders such as `src/<context>/docs/adr/`, absolute
  paths and `..` are skipped. A named `CONTEXT-MAP.md` is followed to each
  `CONTEXT.md` it names and the `docs/adr/` beside it. Without the domain
  doc, the references are the first of `GLOSSARY.md` and `CONTEXT.md`, then
  every `.md` in `docs/adr/`. Explicit `references` in the input still win.
  `CritiqueFilesPort` gained `listFiles`.
- **For #184.** `application/tracker.ts` holds a `Tracker` per tracker kind,
  chosen by `TrackerResolver`. Reading a spec's tickets (sub-issues with
  native blocked-by links, or `.scratch/<feature>/issues/` files) belongs
  there as a second method beside `artifact`, so `check-tickets` resolves the
  tracker the way prepare does. `CritiqueGhPort` will need a sub-issue and
  dependency read for it.

## What #176 settled

Setup (agent-kit#176) installs the critique skills and the Vibe critic
agents. Later tickets build on these:

- **Critic agents.** `vibe_critic_agents:` in `registry/estate-tooling.yaml`
  declares `critic-mistral-large`, `critic-mistral-small`, `critic-glm` and
  `critic-deepseek-flash`. Each renders to
  `registry/generated/vibe/agents/<name>.toml` and setup copies it into
  `~/.vibe/agents/` only, never `~/.vibe-private`. A routing client of kind
  `vibe` must name one of these as its `agent`; a test checks it.
- **What each file carries.** `active_model` is the agent's own name, which is
  also the alias of its single `[[models]]` entry, so it can never collide
  with an alias model sync writes (sync uses Mistral's API ids). Vibe merges
  `models` by alias, so the agent's entry is the one used. Beside it:
  `enabled_tools = ["read_file"]`, `enabled_skills = ["re:^$"]`,
  `outside_workdir = "deny"`, `enable_mcp`, `enable_subagents` and
  `include_project_context` false, `[session_logging] enabled = false`, and
  `safety = "safe"`. These are the keys #169's `unsafe-config` check reads.
  No file sets `default_agent`, and setup never touches `config.toml` for
  critics.
- **Model ids and prices**, USD per million tokens, from
  docs.mistral.ai/inference/pricing, read 2026-11-01:

  | Agent | Model id | Input | Output |
  |---|---|---|---|
  | `critic-mistral-large` | `mistral-large-4-0-3` | 0.7 | 2.1 |
  | `critic-mistral-small` | `mistral-small-latest` (Mistral Small 4) | 0.15 | 0.6 |
  | `critic-glm` | `zai-glm-5-3` | 1.4 | 4.4 |
  | `critic-deepseek-flash` | `deepseek-v4-1-flash-1017` (thinking high, as in the Phase 1 run) | 0.3 | 1.2 |

  GLM's price matches what the personal home already gave it. Mistral
  Small is `-latest` because that is the id the personal home lists; a price
  change upstream means a registry change here. #178 should check each
  agent's `cost_usd` against these prices on a real run.
- **Strict config.** Every key a rendered file holds is a field of Vibe's
  config schema or of its agent profile (checked against Vibe 3.0.4's
  source, the version now installed) and all but `include_project_context`
  were loaded under `VIBE_STRICT_CONFIG=1` in #168. A pytest holds the files
  to that key set. Loading the installed files with the real CLI under
  `VIBE_STRICT_CONFIG=1` is left to #178's live smoke check: it needs the
  personal login.
- **Retiring an agent.** `surfaces: []` removes its generated file (and
  `--check` fails while it lingers) and makes setup remove the installed
  one. Other files in `~/.vibe/agents/` are the owner's and are left alone.
- **Skills.** `critique` (`skills/critique/`, with a short `SKILL.md` that
  points at the stage skills) and `critique-spec` are `local_skills` for
  claude, codex, vibe and antigravity, so they reach Antigravity's app and
  `agy` homes through the existing copy, skipped on Linux. `critique-spec`
  carries `pending: <#177's URL>`: until `skills/critique-spec/SKILL.md`
  exists, setup logs `critique-spec: not built yet (...); skipped`. **#177
  must delete that `pending:` line** in the change that adds the skill; the
  renderer refuses it once the `SKILL.md` exists.
- **The runner's pin.** `manifest.yaml` `critique.files` pins every file of
  `skills/critique/` but `SKILL.md`, with its sha256, the way `council.files`
  pins council's runner. `tests/test_critique_setup.py` checks both the set
  and the hashes, so **any ticket that rebuilds `critique.mjs` or edits a
  prompt, lens, schema or `routing.json` updates its sha256 in
  `manifest.yaml` in the same commit** (`shasum -a 256 <file>`).
  `scripts/validate_manifest.py` also checks the set, but it already fails
  on the integration branch on an unrelated stale pin
  (`.agents/skills/agent-session-bootstrap/SKILL.md`) and is not in CI.
- **The bundle.** `registry/generated/vibe/agents` is in the release
  bundle's `KIT_PATHS`, so a setup run from a published bundle installs the
  agents too.
- Nothing is added to Antigravity's configuration for critics, and no hook,
  `hooks:` or `settings:` section is introduced.

## What #172 settled

The read-only tripwire (agent-kit#172) fixed these:

- **Each critic has its own copy of the packet.** Fan-out writes the frozen
  packet to `critics/<critic-id>/packet/` and launches the critic there, so
  `CriticLaunch.packetDir` is that copy, not the run's `packet/`. A critic
  that writes to its copy is caught without voiding the critics running
  beside it, and nothing one critic writes can reach another. Before any
  critic starts, fan-out checks the run's `packet/` against the manifest's
  hashes and stops with `invalid-run` if a file no longer matches.
  `CritiqueFilesPort` gained `listTree` (every file under a directory, at
  any depth) and `remove`; `listFiles` keeps #175's meaning.
- **What is compared.** Around each critic the tripwire records, before and
  after: the hash of every file in its copy (so a changed, added or removed
  file shows), the same for the run's `packet/`, and the repository's
  working tree as `git status --porcelain=v1 -z --untracked-files=all`
  reports it (`GitPort.porcelainStatus`), with a hash of each listed path.
  The hash catches a critic that edits a file the owner had already
  changed, which the status line alone would not. Ignored paths are not
  watched.
- **What a violation does.** The critic's status becomes
  `violated-readonly` (a third value beside `ok` and `failed`), its
  findings are dropped, and `detail` lists the changes, such as
  `packet: added notes.md` or `working tree: ?? pwned.txt`. The violation
  outranks any other outcome, so a critic that also failed has no
  `failure`. Its copy and the run's `packet/` are restored from the frozen
  contents; the working tree is not touched. A run whose critics all
  failed or violated is degraded. The ledger's run line carries
  `status: "violated-readonly"`, and the summary and the report open with
  one `> **Warning: read-only violated** by critic client <client>` line
  per such critic, before the title.
- **For #171 (parallel fan-out and fallbacks).** Arm the tripwire
  (`ReadOnlyTripwire.arm`) immediately before each client's `review` and
  check it immediately after, per critic and per fallback attempt, with the
  critic's own packet copy as its working directory; never share one copy
  between two critics or attempts. A change to the run's `packet/` or the
  working tree cannot be traced to one of several critics running at once,
  so it voids every critic whose run it fell within; that is intended (fail
  closed), and the owner editing the working tree during a critique voids
  the critics too. Decide whether a `violated-readonly` critic gets a
  fallback: its client is unsafe, but a fallback through a different
  client is not. Load the tripwire once per fan-out (`ReadOnlyTripwire.load`)
  so the frozen contents are read before any critic launches; #170's
  `readPacket` runs right after it, so the text the evidence check quotes
  against has already been checked against the manifest. A test shows that
  a quote one critic plants in its own copy, cited by another critic, is
  rejected as `fabricated-evidence`.
- **For #178 (live smoke check).** Any file a real CLI writes into its
  working directory (a session log, a `.vibe/` or `.agents/` folder) trips
  the wire and voids every review, so the create-a-file check should also
  confirm that a clean run leaves the copy unchanged.

## What #184 settled

The deterministic ticket checker (agent-kit#184) fixed these:

- **The command.** `critique check-tickets` takes `{ "repo", "spec" }`,
  with `spec` named the way prepare names it. It resolves the repository
  and the tracker as prepare does, so a refused remote or an unsupported
  tracker is refused the same way, but it needs no family allowlist entry
  and writes nothing: it sends nothing to a critic. The result is
  `{ status, spec, tickets, cycles, dangling_blockers, coverage }`.
  `status` is `failed` only while there is a cycle, and the command line
  then exits **3** (1 stays an error, 2 usage), so a skill sees the failure
  without parsing the JSON. Any result with `"status": "failed"` exits 3.
- **Reading tickets.** `Tracker.tickets(spec)` sits beside `artifact` and
  returns `TrackedTicket`s (`id`, `reference`, `title`, `text`,
  `blockedBy`, `name`) in tracker order, ready for #185's prepare to
  freeze into the packet. GitHub: the spec issue's sub-issues, then each
  one's native blocked-by links; `GhPort` gained `subIssues` and
  `blockedBy`, read with `gh api --hostname <host> --paginate --slurp
  repos/<owner>/<repo>/issues/<n>/{sub_issues,dependencies/blocked_by}`.
  A `Blocked by` line in a GitHub issue body is not read. Ticket ids are
  `#184`, or the full `host/owner/repo#184` for an issue of another
  repository. Any gh failure is `tracker-unreachable`. Local markdown: the
  `.md` files of the `issues/` folder beside the spec, in file-name order;
  an id is the number the file name starts with as written (`01`), or the
  file name without `.md`.
- **Local "Blocked by".** The first line starting with `Blocked by`
  (`**Blocked by:** 01, 02`, or a `## Blocked by` heading), and the list
  items under it when the line itself is empty. Items are split on `,` and
  `;`. `None`, `n/a` and `-` name nothing. An item starting with a number
  (`01`, `#1`, `01-schema.md`, `02 and 04`) is matched by its value; any
  other item is matched against ticket titles (the heading without its
  `NN:` prefix), ignoring case. An unmatched item stays as written and is
  reported as dangling.
- **Dangling blockers** are any blocker that is no ticket of the spec,
  including a ticket of another spec. On #181 itself every cross-spec
  dependency on #167's tickets is listed, so #185's skill should read the
  list as "check these" rather than as defects.
- **Cycles** are every elementary cycle (Johnson's search), each starting
  at its earliest ticket and listing tickets so that each is blocked by
  the next and the last by the first. A self-blocking ticket is a cycle of
  one.
- **Criteria** are the task-list items (`- [ ]`, `- [x]`, `*`, `+`) under
  the ticket's `Acceptance criteria` heading, or anywhere before its
  `## Comments` when it has none, outside code blocks.
- **User stories.** The spec's stories are the top-level numbered items
  under its `User Stories` heading, through its subheadings. A ticket cites
  a story with `user story 3`, `user stories 3, 5 and 7`, `User stories:
  3–5` or `user story #3`. Coverage is `{ status: "checked", stories,
  uncited }` only when some ticket cites a story and the spec numbers its
  stories; otherwise it is `not-checkable` with the reason. Pocock's
  ticket template has no story field, so #185's `ticket-linter` judges
  coverage whenever this is `not-checkable`.

## What #174 settled

Automatic depth and the per-machine routing override (agent-kit#174) fixed
these:

- **Routing shape.** Each stage may carry `deep` (roster slots, the same
  shape as `base`) and `depth_signals`:
  `{ "thresholds": { <measure>: <number> }, "risk_terms": [...], "labels": [...] }`.
  All three keys are optional, and so are `deep` and `depth_signals`: a stage
  without them never goes deep, and an explicit deep runs its base roster.
  Shipped spec values: `user_stories` 30, `words` 5000, risk terms for auth,
  migration, payment, crypto, encryption, concurrency and `public API` (with
  their common variants), and the label `risk:high`. Deep adds `vibe-glm`
  (integration, operability) and the new `vibe-deepseek-flash` client
  (`critic-deepseek-flash`, family `deepseek`; simplicity, fidelity).
- **For #181: measures are per stage, the shape is not.** A threshold names
  a *measure*, and the runner keeps a table of the measures each stage knows
  (`STAGE_MEASURES` in `domain/depth.ts`; spec has `user_stories` and
  `words`, every other stage has none yet). A threshold on a measure the
  stage does not know is an `invalid-routing` error that lists the ones it
  does. A new stage gets its signals by adding measures there (`decisions`,
  `tickets`, `changed_lines`, `risk_path_hits`, ...) and widening
  `DepthEvidence` (today the artifact text and its labels) with what they
  read; routing needs no new key. Risk path globs live wherever #181 puts
  them (its triage config); a `risk_path_hits` measure with threshold 0 is
  how they would select deep.
- **How signals fire.** A measure fires when its value is *above* its
  threshold. `user_stories` counts numbered list items under a
  `User Stories` heading, up to the next heading of the same or a higher
  level (so #167's `###` groups stay inside); `words` counts white-space
  separated words of the frozen artifact, title and frozen comments
  included. A risk term matches as a whole word or phrase in any case, and
  white space in it matches any run of white space, so `public API` matches
  across a line break while `auth` does not match `author`. A label matches
  the artifact's tracker labels in any case (`artifact.labels` from #175; a
  markdown file has none).
- **The choice.** Prepare's input takes an optional `depth` (`base` or
  `deep`), which always wins. Prepare's result and the manifest carry
  `depth` and `depth_choice: { chosen_by: "argument" | "signals", measures:
  { <measure>: { value, threshold } }, signals: [...] }`, where each signal
  is `{ kind: "threshold", measure, value, threshold }`,
  `{ kind: "risk-term", term }` or `{ kind: "label", label }`. The signals
  are evaluated even when the argument decides, so the summary can say what
  they would have chosen. The summary and the report show it on the depth
  line (`Depth: deep, chosen by signals: risk term "auth"`,
  `Depth: base, chosen by signals: none fired: user_stories 2 of 30, words 25 of 5000`,
  `Depth: base, chosen by argument; the signals alone would choose deep (...)`),
  and every ledger line carries `depth`, `depth_chosen_by`,
  `depth_measures` and `depth_signals`: configured terms, label names and
  counts only, never artifact text.
- **Deep slots follow the base rules.** Deep slots are resolved after the
  base ones, with the same family, orchestrator, distinct-family, readiness
  and `fallbacks` rules (shipped routing names no fallback for GLM or
  DeepSeek). A deep critic that cannot run is listed in `dropped` with
  `deep: true`, and the summary says `deep critic left out` instead of
  `no fallback left`. When GLM has taken Gemini's base slot, the deep GLM
  slot is dropped as `same-family`.
- **The routing override.** `~/.config/agent-kit/critique-routing.json`, if
  present, is applied over the shipped `routing.json` as a JSON merge patch
  (RFC 7386): objects merge key by key, `null` removes a key, and any other
  value, a list included, replaces it. So an override names only what it
  changes (one client's model, one threshold), and replacing a roster means
  restating the whole list. The merged routing is validated as a whole; any
  error is `invalid-routing` and names both files. `RoutingSource`
  (`application/routing.ts`) loads it; **#173 and anything else that reads
  routing should go through it** rather than reading `routing.json`.
- **Model names** still appear only in `routing.json` (and the override,
  which is routing) and in the `critic-<model>` agent files; the runner's
  source names none. The `vibe-glm` client says `glm-5-3` where its agent
  file pins `zai-glm-5-3`; the Vibe model field is informational (the agent
  pins the model), so this was left for whoever next edits that client.

## What #177 settled

The `/critique-spec` skill (agent-kit#177) fixed these:

- **The skill.** `skills/critique-spec/SKILL.md` is agent-neutral and finds
  the runner at `../critique/critique.mjs`, so the two skills must always be
  installed side by side (a pytest checks every agent home). It maps the
  orchestrator's own model, not its app, to `orchestrator_family`: Claude
  `anthropic`, GPT `openai`, Mistral `mistral`, GLM `zhipu`, DeepSeek
  `deepseek`, Gemini `google`. Antigravity and Vibe's fallbacks are the
  runner's (#171); the skill only reports them. The registry entry no longer
  carries `pending:`.
- **Order of a round.** Prepare, fan-out, verdicts, fixes to the spec, the
  owner's answers, then record. The questions come before record so record
  knows which are answered, and before a second round so the answers reach
  the revised spec. The single change comment is posted after the last
  round, so no round's results reach the next round's packet (for local
  markdown the comment is part of the spec file, which prepare freezes
  whole).
- **Answered questions.** A `needs-human` verdict may carry
  `"answered": true`; on any other verdict it is an `invalid-input` error.
  The summary marks an answered question `(answered)`, the report says
  `Verdict: needs-human, answered: ...`, and while a critical question is
  unanswered the summary has `Do not start /to-tickets: critical question F3
  is unanswered.` under the verdict counts. The ledger does not record
  answers.
- **Rounds.** Record's result has `round` and `second_round_due`, true when
  a first round accepted more than three critical findings
  (`SECOND_ROUND_AFTER_CRITICAL` in `domain/stage.ts`). Prepare takes an
  optional `previous_run_dir`: the run must be recorded (`report.md` exists,
  else `invalid-run`) and be of the same repository, stage and artifact
  reference (else `invalid-input`). A run that was itself round 2 is refused
  with the new reason `round-limit`, so a stage never runs a third time.
  Prepare's result, the manifest (`round`, and `previous_run_id` on round 2)
  and every ledger line carry the round; a manifest without `round` reads as
  round 1. A round-2 summary says `Round 2, after run <id>` under the depth
  line. Prepare does not check that the first round actually called for a
  second one; the skill follows `second_round_due`.
- **For #173.** A second round is a second prepared run. `previous_run_dir`
  is how prepare knows the two belong to one flow, so the per-flow Vibe cap
  can add the first round's spend to the second's from there.

## What #186 settled

Code packets and the code permission (agent-kit#186) fixed these:

- **The code permission.** An allowlist entry has `code`, true or false.
  An entry without it (every entry recorded before #186) reads as false,
  and any other value makes the file `invalid-allowlist`. `allow` takes an
  optional `"code": true | false`; left out, it keeps the entry's current
  value (false for a new entry), so re-running `allow` to change families
  never drops or grants code. Its result now carries `code`. Prepare of
  `ticket-review` or `feature-review` with code off is the refusal
  `code-not-allowed`, after `no-remote`, `nedap` and `not-opted-in` and
  before the tracker is read or anything is written. #187's triage call
  should check `entry.code` the same way; #188 and #189 offer to call
  `allow` with the entry's families and `"code": true`.
- **Prepare's input for a code stage.** `base` (required: a commit or a
  branch the change is diffed against), `head` (default `HEAD`), and, for
  `ticket-review`, `ticket` (required, named the way the tracker names a
  spec). `feature-review` reads the spec's tickets through
  `Tracker.tickets` (#184) and ignores `ticket`; `references` is ignored by
  both. A revision that names no commit is `invalid-input` before anything
  is written. `head` can be any commit, so #188 can review a staged change
  before it is committed by passing the id `git stash create` prints (that
  writes a commit object but touches neither the stash list nor the index).
- **The code packet.** `tree/<repository path>` for every regular file of
  `head`'s tree, read from git's object store (`git ls-tree`, `git cat-file
  blob`), so there is never a `.git`, worktree metadata or an untracked or
  uncommitted file. Symbolic links and submodules are left out, and so is any
  file holding a NUL byte (binary). `diff/change.diff` is the diff from the
  merge base of `base` and `head` to `head` (`--no-ext-diff --no-textconv
  --no-color`, `core.quotePath=false`). `tickets/<name>` holds the ticket, or
  every ticket of the spec, and `spec/<name>` the spec, both rendered as a
  document stage renders them. Nothing else is at the packet root, so the
  repository's `AGENTS.md`, `.vibe/` and `.agents/` land under `tree/`.
  Packet file kinds are now `tree`, `diff`, `ticket` and `spec` beside
  `artifact` and `reference`.
- **The manifest and prepare's result.** The manifest gains `code: { base,
  base_commit, head, head_commit, changed_files }`, where `changed_files`
  are packet paths under `tree/` of the files the diff touches that are
  still in the tree (#189's "the head it reviewed" is `head_commit`).
  `manifest.artifact` is the ticket for a ticket review and the spec for a
  feature review, so #174's `risk:high` signal reads the ticket's or the
  spec's labels. After the merge with #174, a code stage's depth is chosen
  from that artifact's labels and text (risk terms); `STAGE_MEASURES` has
  no measure for either code stage yet, so changed lines and risk path hits
  are still to be added as measures (the diff and `code.changed_files` are
  there to measure). Prepare's result `packet_files` now lists what the
  critics' prompts name: every file for a document packet, and for a code
  packet the diff, the changed files, the tickets and the spec, but not the
  rest of the tree (which can be thousands of files). The result adds
  `code: { base_commit, head_commit, changed_files, tree_files }`.
- **The prompt.** A code packet's file list names the diff, the changed
  files, the tickets and the spec, then tells the critic that the rest of
  the tree is under `tree/` and readable by path, and to cite code as
  `tree/<path>:<line>` or `tree/<path>:<first>-<last>` in `location` and in
  each trace step. This is in the runner's prompt assembly, not in a
  hashed asset. #188's `diff-reviewer` role does not need to repeat it.
- **The tripwire** needed no change: every tree file is a packet file, so
  each critic's copy holds the whole tree and a change anywhere in it (or a
  file added at the copy's root) voids the critic as `violated-readonly`.
- **File-and-line evidence.** At a code stage only, fan-out reads every
  `path:line` and `path:first-last` in a finding's `location` and in each
  `trace` evidence value. A path counts when it has a `/` or a file
  extension, so `10:30` is not a file, and URLs are skipped. It is looked up
  as written, then under `tree/`, then, for a diff's `a/` or `b/` prefix,
  under `tree/` without it. A path the packet does not hold, or a line below
  1 or past the file's last line, pre-rejects the finding as
  `fabricated-evidence` (after the quote check, before the hint rule). A
  code finding with no `failing-test` and no `trace` that cites a file and
  line is the hint `no-test-or-trace`. The document stages are unchanged. The
  evidence check now takes the packet indexed once per fan-out
  (`indexPacket`), so a large tree is normalised once, not once per
  finding. Whether a feature-review trace touches two tickets is the
  orchestrator's check (#189), not a mechanical one.
- **Git.** `GitRevisionPort` (in `src/ports/git.ts`, implemented by
  `GitCliAdapter`, added to `CritiqueGitPort`) has `resolveCommit`,
  `mergeBase`, `treeFiles`, `fileAt`, `diffCommits` and `changedPaths`. The
  harness's `FakeGit.commit()` now records a commit (`c1`, `c2`, ...; the
  harness makes `c1`) and serves `HEAD`, `HEAD~<n>` and those ids on one
  line of history. `codeStageRouting()` in the harness routes one Vibe
  critic on both code stages until #188 and #189 ship their routing; with
  shipped routing, a code stage's prepare reads the packet and then fails
  `invalid-routing` before writing anything.
- **Process output.** `NodeProcessAdapter` decoded each output chunk on its
  own, so a multi-byte character split across chunks became two
  replacement characters; the streams now decode. A code packet reads every
  file through it.
- **Live check.** On a scratch clone of this repository (origin pointed at
  GitHub, a scratch `HOME`), prepare refused without an entry
  (`not-opted-in`) and with documents only (`code-not-allowed`), then, after
  `allow` with `"code": true`, froze #186 and #181 through gh, 689 tree
  files, and a 1,932-line diff over `HEAD~5`. The packet root held only
  `diff`, `spec`, `tickets` and `tree`, no path had a `.git` component, and
  the tree matched `HEAD`, not the uncommitted working tree.

## What #173 settled

Budget, quota, timeouts and fallbacks (agent-kit#173) fixed these:

- **Outer timeout.** `ProcessResult` gained `timedOut`, which council's
  `NodeProcessAdapter` sets when a command outlives `timeoutMs`; it sends
  SIGTERM, then SIGKILL after a 5-second grace, so a CLI that ignores
  SIGTERM still stops. Both critic clients record a killed run as the
  failure `timeout` (not `exit-124`). The outer timer is still the CLI's own
  limit plus 60 seconds.
- **What a launch is charged.** `CriticReport.chargedUsd`, and `charged_usd`
  on the critic run and the ledger run line, beside the reported `cost_usd`.
  For Vibe it is the export's `cost_usd`, even past the critic's cap (the
  overshoot of up to one turn counts). The critic's `max_price_usd` is
  charged instead where Vibe's figure cannot be trusted: a killed run, no
  export or no cost, a `deadline` stop (at least the cap), and a zero cost
  with tokens used (an unpriced model). An argument error that writes no
  export is charged the cap too: conservative, and rare. agy is always 0.
- **Invocation lines.** Fan-out appends `{ type: "invocation", recorded_at,
  repository, flow, run_id, stage, round, critic_id, client, kind, family,
  model, status, charged_usd }` to the ledger as each launch ends, so spend
  and the agy count hold even for a run that is never recorded. They are the
  only lines the budget reads; record's run lines do not count twice. A line
  that does not parse (a write cut short) is skipped. Tests that read "every
  ledger line" now filter these out, and they carry no depth fields.
- **The flow.** Prepare takes an optional `flow`; by default it is the
  artifact's reference (`host/owner/repo#<n>`, or the markdown path), and a
  second round (`previous_run_dir`) inherits its first round's flow. The
  manifest carries `flow` and `limits`. Later stages (tickets, ticket
  review, feature review) should pass the spec's reference as `flow`, so the
  whole spec-to-ticket flow shares one cap. `/flow budget` (#193) can sum
  `charged_usd` of the `vibe` invocation lines of a repository and flow.
- **Per-flow cap.** Routing's `budget.vibe_flow_cap_usd` (5). Before each
  wave, fan-out reads the flow's spend; a Vibe critic is not launched once it
  has reached the cap, and is recorded with status `budget-skip` (exit code
  null, no stdout or export path, charged 0) and a `detail` such as `the
  flow's Vibe spend $5.2500 has reached its $5.00 cap`. The summary lists it
  as `<client> (...): budget-skip: <detail>`. A wave's critics start
  together, so one wave can overshoot by its critics' caps (plus a turn each).
  A budget skip gets no fallback retry.
- **Antigravity quota.** Routing's `antigravity_quota` (`daily_cap` 20,
  `reserve_threshold` 10, `reserved_stages` `spec` and `feature-review`).
  Prepare counts today's `agy` invocation lines (the UTC day of the clock),
  and an agy client past the quota is dropped with the new reason `quota`
  and a `detail` naming the count and the cap or the reserve; its slot falls
  back as for any other drop. A readiness failure (Linux, not signed in, not
  on PATH) is checked first, so it keeps the reason `unavailable` and
  launches nothing. Both limits are read through `RoutingSource`, so the
  per-machine override can change them, and they default to #167's values
  when routing leaves them out.
- **Fallback retry.** Each roster critic carries `fallbacks` in the manifest:
  its slot's other clients, in routing order, that the allowlist permits,
  the orchestrator does not share and that were ready at prepare (base and
  deep slots alike). After the first wave, a `failed` critic (not
  `violated-readonly`, not `budget-skip`) gets one retry through the first
  fallback whose family no critic that reviewed, nor an earlier retry,
  already has (with `distinct_families`, now in the manifest too), whose agy
  quota is not reached, and, past the cap, that is not Vibe if another one is
  left. The retry's critic id is `<critic id>-retry-<client>`, so it gets its
  own packet copy and tripwire; it has `retry_of`, the failed critic's slot,
  role, lenses, cap and time limit, and no fallbacks of its own. The summary
  line reads `..., fallback for <critic id> (<failure>)`. A stage whose
  critics and retries all failed is degraded, as before.
- **Vibe readiness.** `VibeCriticClient` now takes the env port and checks,
  launching nothing but a Keychain lookup: `vibe` on PATH (or the absolute
  path), the personal home signed in the way setup checks it (Vibe's
  Keychain item `ai.mistral.vibe`, else a non-empty `MISTRAL_API_KEY` in the
  personal home's `.env`; the environment variable does not count, since a
  critic runs with it unset), and `~/.vibe/agents/<agent>.toml`. The
  `detail`s are `vibe: <binary> is not on PATH`, setup's own `vibe: not
  signed in; run vibe and sign in with your Mistral account, then re-run
  setup`, and `vibe: the critic agent <agent> is not installed in
  ~/.vibe/agents; re-run setup`, with reason `unavailable`, which is what
  #177's skill quotes. The harness installs `vibe`, the four critic agents
  and a Keychain sign-in by default (`vibeOnPath`, `vibeSignIn`,
  `vibeAgents`); `FakeVibeProcess.commands` still holds launches only, and
  the lookups are in `checks`.
- **For #177.** A `quota` drop is not a missing CLI: its slot was refilled
  or left empty by routing, and the summary already names it, so the skill
  need not ask the owner about it. A `budget-skip` critic is not a failure
  either.
