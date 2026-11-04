---
name: model-gateway
description: Holds the model gateway's runner and its shipped defaults; it is not a workflow of its own, since the delegate skill, coming later, drives it.
---

# model-gateway

This folder is what the model gateway runs on. It does nothing on its own: an
agent calls the runner, and the `delegate` skill (a later ticket) is what
teaches an orchestrator to plan with it. The runner answers these questions:
`models`, which client models this machine can use right now; `usage`, what
each client can do now and what it has spent; `run`, one delegation to one of
them; `workflow`, the life of the piece of work a delegation belongs to; and
`grant`, the owner's permission to spend their employer's money on one of
them.

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

## The `usage` command

`usage` reports every client the gateway knows: whether it can run now, what
it has spent, what its own plan has left, and how close it is to the plan
limit the owner set for it. It prints one JSON object on stdout and exits 0.

```sh
node model-gateway.mjs usage --json '{"days": 7, "workflow": "spec-293"}'
node model-gateway.mjs usage --input <file.json>
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

## The `run` command

`run` hands one task to one client model, read-only or writing, and reports
what came of it. It prints one JSON object on stdout and exits 0, whether the delegation
succeeded or failed; only a refusal is an error, and it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
node model-gateway.mjs run --json '{"workflow":"review","client":"vibe","model":"mistral-large-4-0","mode":"read-only","prompt":"Review src/app.ts","repo":"/path/to/repo"}'
node model-gateway.mjs run --input <file.json>
```

The input is `workflow`, `client`, `model` (an alias `models` listed), `mode`,
`prompt`, and optionally `repo`, `carries` (`{"code":bool,"history":bool}`,
only for an inline run), `time_limit_s` (default 900), `max_price_usd` and
`provider` (which a Hermes run must name, and which this machine must allow),
`escalate` (default `true`; `false` turns off the retry, the climb and the
fall-through below), `max_attempts` (default 3) and `headless` (a boolean that
changes nothing else: a run marked headless is refused and launched exactly
like any other).
`workflow` is required (`code: "invalid-input"`), and is a name: a letter or
digit, then letters, digits and `. _ : / -` only, at most 80 characters. Every
command that takes a workflow name holds it to that rule, because the name
reaches the shell command the gateway suggests. A run can go to `client:
"vibe"`, `"agy"` (Antigravity), `"claude"` (Claude Code), `"opencode"`,
`"hermes"` or `"codex"`, in `mode: "read-only"` or `"write"` (below); any other
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
- `write-needs-repository` — a writing run names no `repo`.
- `provider-not-allowed` — a Hermes run must name a `provider`, and it must be
  one this machine lists in `clients.hermes.providers` (empty by default, so
  every Hermes run is refused until the owner names one). `openrouter` is
  refused whatever the configuration says, and so is a model id that starts
  with `openrouter/`: the owner forbade OpenRouter. An OpenCode model is named
  `<provider>/<model>`, and only the providers `clients.opencode.providers`
  names (default `["opencode-go"]`) are allowed; every other provider is
  refused, and so is `opencode/<model>`, OpenCode's pay-per-token Zen, even
  when the machine lists it: a run never reaches a pay-per-token model.
- `workflow-ended` — the run names a workflow that ended, so its caps and
  grants lapsed with it (below).
- `no-grant` — a Codex run whose workflow no live grant covers. The message
  names the workflow and the `grant write` call that would add one. It applies
  to every mode, to every attempt of a delegation, and to a `headless` run.
- `cap-exceeded` — the run's estimate does not fit what its cap has left: the
  metered-personal caps for a metered-personal run, the grant for a Codex run
  (below).
- `worktree-failed` — git could not add the throwaway worktree, or create a
  writing run's worktree off its base (below). A
  worktree git cannot remove after the run is not an error: the result is
  returned with a `cleanup_warning`.
- `unknown-model` — a model the catalogue cannot place, or one this machine
  disabled.

A run goes in place only on a client model with a passing read-only proof:
setup's recorded result for the critic client of the client's own kind (Vibe,
or Antigravity as the Gemini critic) whose `model` matches, in critique's
routing (the shipped `routing.json`, overridden by
`~/.config/agent-kit/critique-routing.json`):
`checks["read-only:<client>"].result == "pass"` in
`~/.local/share/agent-kit/critique/machine-setup.json`. Every other model,
Vibe's included, has none and works in a throwaway worktree: `git worktree add
--detach <temporary directory> HEAD` of the `repo`, which sees the committed
files and none of the working copy's edits, and is removed with `git worktree
remove --force` after the run whatever its outcome. A run without a `repo`
works in a fresh empty temporary directory, deleted the same way. A cleanup
that fails never fails the run: the result carries a `cleanup_warning` naming
the directory left behind, and a run that threw keeps its own error. The result
and the ledger line are the same for every client. Claude Code, OpenCode,
Hermes and Codex have no critic client at all, so every one of their runs is a
throwaway one.

A launched Vibe run is non-interactive:

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

The other clients, each in its working directory and with
`AGENT_KIT_DELEGATION=1` in its environment. Claude Code, OpenCode, Hermes and
Codex have no time limit of their own, so the outer timer stops them at
`time_limit_s`; Antigravity's own `--print-timeout` fires first, a minute
before the outer one:

```sh
agy --print <prompt> --model <id> --output-format json --effort <low|medium|high> \
    --mode plan --print-timeout <s>s
claude -p --model <id> --output-format json --permission-mode plan \
    --allowedTools Read,Grep,Glob            # the prompt on stdin
opencode run --model <provider>/<model> --format json <prompt>
hermes -z <wrapped prompt> --usage-file <run dir>/usage.json -m <model> --provider <provider>
codex exec --json --skip-git-repo-check -m <model> \
    -c model_reasoning_effort="<low|medium|high>" \
    -s <read-only|workspace-write> -C <dir>   # the prompt on stdin
```

Codex runs non-interactively with its own sandbox: `-s read-only` for a
read-only run and `-s workspace-write` for a writing one, so the sandbox is
Codex's, not the gateway's. Its reasoning effort is the model id's last word
(`gpt-5.5-high` is `high`), else `medium`. `--json` makes it print one event
per line: the answer is the last `agent_message` item, and the tokens come from
the `turn.completed` usage (`input_tokens` counts the cached ones too, which
are reported separately). Codex prices nothing, so its dollars are the tokens
times the catalogue's `prices` for the model, like Vibe's and Antigravity's
when they report none.

Antigravity's effort is the id's last word (`gemini-3.8-flash-high` is
`high`), else `medium`. Tokens and dollars come from each client's own output
(agy's `usage`, Claude Code's `usage` and `total_cost_usd`, the sum of
OpenCode's `step_finish` events); agy prices nothing, as it is quota, not
money. `max_price_usd` is passed to Vibe only.

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
"summary"}`, plus the `cleanup_warning` above when it applies, and a failed
run adds a `reason`: `timeout`, `parse-error`, `exit-<n>`, `rate-limited`,
`auth`, `launch-failed` (the client could not be started at all),
`denied-action` (Antigravity was denied an action), `client-error` (the
client's own output reports an error), `failed` (Hermes' own usage report
counts the run as failed) or, for a writing run, `check-failed`,
`check-output-failed` or `stage-failed`. `rate-limited` and `auth` are what the
client's own error channel — its stderr, the error its structured output
carries, or its exit code — said, never the delegate's answer, and they are
what `usage` reads afterwards to report the client unavailable. The raw output
is kept either way. A client at or above `plan_warning_ratio` of a plan window
adds `warnings` to the result, the same lines `usage` prints for it.

A run with a `repo` keeps its directory in that repository's git common
directory, `<git common dir>/model-gateway/runs/<run_id>/`, so a worktree's
runs sit with the repository's; an inline run keeps it in
`$XDG_STATE_HOME/agent-kit/model-gateway/runs/<run_id>/`, else
`~/.local/state/agent-kit/model-gateway/runs/<run_id>/`. It holds `prompt.md`,
`input.json`, `output.json` (the client's full output), `stderr.txt` (what the
client printed on stderr, when anything) and `result.json`, and a writing run's
`check-output.txt`. `run_id` is a UTC timestamp plus a short
random suffix.

Every run also appends one JSON line to the machine-wide ledger,
`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl` (else
`~/.local/state/...`): workflow, client, model, family, mode, repository key,
tokens, dollars, outcome, duration and run id. Writes take an exclusive lock
file beside it, so two gateway processes never interleave a line. Dollars come
from the client's own report when it prices the run — Vibe's `export.json`, or
Hermes' `usage.json` — else from the tokens times the catalogue's `prices` for
the model.

## When a delegation fails or cannot start

A delegation does not come straight back when it fails. It is retried, climbs
to a stronger model, and swaps a client that cannot start, all inside one `run`
call, and every step is a run of its own with its own run id, run directory and
ledger line. `"escalate": false` turns all of it off; `max_attempts` (default
3: the first attempt, the retry, one climb) is how many runs a delegation may
make in all.

- **A failed result** is a run whose status is `failed`: a timeout, output the
  adapter cannot parse, a non-zero exit, or a writing run whose check failed.
  Refusals (`policy-refused`, `provider-not-allowed`, `cap-exceeded`,
  `no-grant`, `no-estimate`, `workflow-ended` and the other error codes) are
  errors before anything is launched and are never retried. The raw or partial output stays in
  the run directory (`output.json`, and `stderr.txt` when the client printed
  any) beside the `reason` in `result.json`.
- **Retry.** The first failure is retried once on the same client model. The
  retry's prompt is the caller's prompt plus `A previous attempt failed:
  <reason>; <the last 1,000 characters of the check's output for a failed
  check, else of the client's stderr, else of its output>`. A writing run's
  retry gets a fresh worktree; the failed one is kept like any other until the
  workflow ends.
- **Climb.** A second failure moves to the next stronger subsidised model: a
  catalogue `tier` higher than the failing model's, whatever the failing
  model's own cost class, in the repository's permitted list (the family
  allowlist and the nedap rule, through the data policy above), never a model
  the machine disabled, ordered by tier and then catalogue order. It never
  moves into a metered class (a metered-personal run climbs only to a
  subsidised model, and a metered-employer model is never a climb target at
  all), and never to a model the catalogue gives no tier. A model with no
  catalogue tier is never climbed to or swapped to, and is retried once and
  then returned, because there is no way to say which model is stronger. A
  climbed model that fails is retried once, then climbed from again. When no
  stronger model is left, or `max_attempts` is spent, the last result comes
  back with `"escalation": "exhausted"`. An attempt whose client could not
  start does not count toward `max_attempts`, though it is listed in
  `attempts`.
- **Fall-through.** A client that cannot start is swapped for another
  permitted client model of the same cost class whose tier is at least the
  requested model's. Cannot start means what `usage` reports as
  `unavailable` or `unable_to_start` for the client (not installed, signed out,
  a plan window at 100%, a recent rate-limit or auth error), or a run whose
  `reason` is `launch-failed`, `rate-limited` or `auth`; a swap is not a retry
  and does not use the retry. A metered-employer model is in that pool only
  while a live grant covers the workflow, so a Codex model is swapped in only
  when the owner allowed it; `no-grant` and `cap-exceeded` are refusals, which
  no attempt ever retries. The gateway never moves into a costlier cost
  class on its own: when no client model of the cost class can start it
  returns `{"status":"unavailable","workflow","client","model","message",
  "reasons":[{"client","model","reason"}],"attempts":[...]}`, exit 0, and the
  orchestrator decides. That `unavailable` is only for a delegation no attempt
  of which ever ran: when an earlier attempt really ran and failed and a later
  one cannot start with nothing to swap to, the last real result comes back
  instead, with `escalation: "stopped"` and `stopped_reason` naming what the
  next attempt could not start with. In a `nedap` repository only Claude Code
  and Codex are permitted, so Claude Code never falls through to Vibe or
  Antigravity.

The result of a delegation is the last run's result plus `attempts`, a short
list of `{run_id, client, model, status, reason?}` in order (one entry when
nothing went wrong, or with `escalate: false`); `skipped`, the client models
passed over because they could not start; `escalation: "exhausted"` as above;
and, on a retry or a climb, `attempt` (its place, 2 and up) and `attempt_of`
(the first run's id), which each such run's ledger line carries too. A later
attempt the gateway refuses (the workflow ended meanwhile) ends the delegation
with the last result, `escalation: "stopped"` and `stopped_reason` naming the
refusal. The first attempt's refusals are errors as always.

## Writing runs

`"mode": "write"` hands a code change to a client model. It takes `repo`,
`base` (a ref) and `check` (a command string) besides the usual input; a run
without `repo` is `write-needs-repository`, and one without `base` or `check`
is `invalid-input`. `base` and `check` on a read-only run are `invalid-input`
too. The data policy is the same as for any run that reads a repository: the
entry must allow the model's family and `code`.

```sh
node model-gateway.mjs run --json '{"workflow":"spec-293","client":"claude","model":"claude-sonnet-5-5","mode":"write","prompt":"Add the --verbose flag","repo":"/path/to/repo","base":"main","check":"npm test"}'
```

1. The gateway creates a worktree off `base` at
   `<git common dir>/model-gateway/worktrees/<run_id>`, on a new branch
   `gateway/<run_id>` (`git worktree add -b`). Whatever the model's read-only
   proof, a writing run works there and never in the repository itself.
2. The delegate runs in the worktree with write permission: Vibe through the
   `delegate-write` agent (`read_file`, `grep`, `write_file` and `edit`, no
   shell, nothing outside the worktree, no subagents; setup installs it beside
   `delegate-readonly`), Claude Code with `--permission-mode acceptEdits` (and
   `--allowedTools Read,Grep,Glob,Edit,Write`), Antigravity with `--mode
   accept-edits`, OpenCode with its default agent, Codex with `-s
   workspace-write`. The prompt adds "Edit only
   files inside the working directory. Do not commit."
3. When the delegate has finished, the gateway stages everything in the
   worktree with `git add -A` and lists what that staged, and only then runs
   `check` there with `sh -c` (held to `time_limit_s`; its output, stdout then
   stderr, goes to `check-output.txt`; a check that cannot start has exit code
   127). It never stages again, so whatever the check writes (coverage, a
   build, a rewritten lockfile) is not part of the delegate's diff and stays an
   unstaged change in the worktree. It never commits, merges or pushes: the
   worktree and the branch are the caller's to take.

The result is the read-only result and `worktree` (the path), `branch`,
`staged_files` (relative to the worktree) and `check`
`{"command","exit_code","output_path"}`. `status` is `ok` only when the
delegate finished and the check exited 0; a failing check is `failed` with
`reason: "check-failed"`, and its worktree is kept and staged all the same. A
run whose delegate did not finish (`timeout`, `exit-<n>`, `parse-error`, ...)
leaves its worktree unstaged and kept, runs no check and has no `check` in its
result. If the gateway itself fails after the delegate ran (git cannot stage the
worktree, git cannot list what it staged, or the check's output cannot be
written), the run is `failed` with `reason: "stage-failed"`,
`"stage-list-failed"` or `"check-output-failed"`, the worktree and branch in
the result and git's or the file system's message in `worktree_error`. A run
whose staged files git could not list says so with `staged_files: null` rather
than an empty list, which would read as a delegate that changed nothing.

Every writing worktree is entered in `kept-worktrees.json` in the gateway's
state directory (`$XDG_STATE_HOME/agent-kit/model-gateway/`, else
`~/.local/state/...`), whatever the run came to, and stays until the workflow
that made it ends. `workflow end` ends it; `worktrees clean` then removes what
it kept:

```sh
node model-gateway.mjs workflow end --json '{"name":"spec-293"}'
node model-gateway.mjs worktrees clean --json '{"workflow":"spec-293"}'
node model-gateway.mjs worktrees clean --json '{"all":true}'
```

It removes each kept worktree of that workflow (`git worktree remove --force`)
and its branch, and prints `{"status":"ok","removed":[...],"failed":[...],"branch_kept":[...]}`;
a worktree it cannot remove stays on the list with the reason, for the next
call. Once the worktree is removed the entry is gone, and a branch git would not
delete (checked out elsewhere, or already gone) is named in `branch_kept` with
git's message.
Take what you want from a worktree (commit its staged diff on a branch of your
own) before the workflow ends: cleaning discards it.

## The `workflow` commands

`workflow start`, `workflow end` and `workflow list` keep a workflow's life.
`start` and `end` take `{"name": "<id>"}`, held to the workflow-name rule above;
`list` takes `{}`. Each prints one
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
lapsed with it — a new piece of work needs a new name. A run joins its
workflow only once every refusal has passed, so a refused run leaves no
workflow started behind it. `workflow end` on a workflow this machine never
started is `code: "unknown-workflow"`.

The workflows live in `$XDG_STATE_HOME/agent-kit/model-gateway/workflows.json`,
else `~/.local/state/agent-kit/model-gateway/workflows.json`, beside the
ledger and under the same lock: a change reads the file, applies itself and
writes the result through a temporary file that is renamed into place, so two
gateway processes never lose each other's change.

## Grants: spending the owner's employer money

Codex is the one `metered-employer` client: its runs are billed to the owner's
employer, not to the owner. A Codex run therefore needs a **grant** — the
owner's permission for one named workflow, with a dollar cap — and the gateway
never writes one itself and never renews one.

**Ask the owner first.** A grant is written by the agent, after it asked the
owner and got an answer, and it records both the question and the answer so a
grant written without asking stands out. Never write one on your own
initiative, and never widen one: a bigger cap or a longer life is a new
question to the owner.

```sh
node model-gateway.mjs grant write --json '{"workflow":"spec-293","client":"codex","cap_usd":5,"question":"may I spend up to $5 of nedap money on spec-293?","answer":"yes, up to five dollars"}'
node model-gateway.mjs grant list  --json '{}'
```

`grant write` takes `workflow`, `client` (a metered-employer client, `codex`
today), `cap_usd`, `question` and `answer`, and prints
`{"status":"ok","grant":{...}}` with what the grant has left. It refuses an
empty `question` or `answer`, a `cap_usd` that is not above 0, a client that is
not metered-employer (`invalid-input`, naming its cost class), a client the
gateway does not know (`unknown-model`), a workflow that ended
(`workflow-ended`) and a workflow and client a live grant already covers
(`grant-exists`, naming that grant, its cap and what it has left): the owner
re-approves only after it lapsed, so a second grant can never quietly double
the cap. A grant lapses 24 hours after it was written, or when its workflow's
own 24 hours run out first, whichever comes first.

The write is the last thing that can fail: when the ledger cannot be read
afterwards, the result is still `ok`, with the grant it wrote, `"room": null`
and a `warning` saying what it could not read. Never ask the owner again
because of that warning — the grant stands.

`grant list` prints `{"status":"ok","grants":[...],"lapsed":[...]}`. Each live
grant carries its `cap_usd`, `spent_usd` (what its workflow has spent on that
client's metered-employer runs since the grant was written), `reserved_usd`
(what live reservations of that client hold) and `remaining_usd`. A lapsed
grant is in `lapsed` with a `reason`: `workflow-ended`, `expired` or
`cap-spent`. A grant lapses as `cap-spent` only once settled spend reaches its
cap: one whose cap is merely reserved by a run in flight is still live, and the
next run that does not fit it is refused `cap-exceeded`. The gateway never
renews a lapsed grant; ask the owner again.

The grants live in the machine configuration,
`$XDG_CONFIG_HOME/agent-kit/model-gateway-grants.json`, else
`~/.config/agent-kit/model-gateway-grants.json`, under the same lock and
temporary-file write as the workflows. **Never inside a repository**: a grants
path that lies inside any git repository is refused, for reading and for
writing alike (`code: "grants-in-repository"`, naming the repository), because
a grant is the owner's and not the project's.

A grant covers one workflow **and one client**: a grant for `codex` never
covers another metered-employer client on the same workflow, and its room
counts only that client's runs and reservations.

A Codex run is refused `no-grant` when no live grant covers its workflow and
client, and `cap-exceeded` when its estimate does not fit what the grant has
left. Before it launches, the run's estimate — the same estimate the
metered-personal caps use — is reserved against the grant's remaining cap in
one locked critical section, and the run's actual spend replaces the
reservation when it ends. The gateway settles the reservations whose runs died
before it reads a grant, so a dead run's reservation never makes a grant look
spent for the run that asks next. A refused run reserves nothing and starts no
workflow. The order is always `requireWorkflowOpen` → reserve →
`startWorkflow`.

## The check runs the delegate's code

The gateway runs the caller's `check` with `sh -c` inside the worktree the
delegate has just written, so the check is only as trustworthy as the delegate:
a delegate that edits the project's test entry point (the `test` script in
`package.json`, a `Makefile` target, a test runner's config) can make the check
run code of its own choosing, with your privileges and outside the worktree.
Send a writing delegation only to a client model you trust with that, and read
the staged diff before you commit it.

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
  cost class. `providers` is the list of providers a run may reach through the
  client, by the name its own command line takes: the ones an OpenCode model
  may name (`provider-not-allowed` otherwise; `["opencode-go"]` by default) and
  the ones a Hermes run may name (`hermes --provider`; empty by default, so no
  Hermes run is allowed until the owner lists one). `openrouter` stays refused
  whatever it says.
- `clients.<id>.plan_limits` is what that client's plan allows, window by
  window: a list of `{"window": "5h"|"day"|"week"|"month", "limit_usd": n}`,
  or `{"window": ..., "limit_runs": n}` for a plan that is not billed in
  dollars. Each window reaches back from now, and its spend comes from the
  ledger. The list replaces the client's whole plan, so an override that names
  one window names them all. The shipped defaults carry none: the owner's
  plans are personal and live here. The owner's OpenCode Go plan, for example:

  ```json
  {
    "clients": {
      "opencode": {
        "plan_limits": [
          {"window": "5h", "limit_usd": 12},
          {"window": "week", "limit_usd": 30},
          {"window": "month", "limit_usd": 60}
        ]
      }
    }
  }
  ```

  At or above `plan_warning_ratio` of any window, `usage` and every `run`
  result for that client carry a warning; at 100% of any window the client is
  `unable_to_start` in `usage`.
- A client field the gateway does not know is an error naming it, rather than a
  setting quietly ignored: a client has `binary`, `cost_class`,
  `model_source`, `families`, `provider_cost_classes`, `providers` and
  `plan_limits`, and nothing else.
- `models`: an entry matching `client` and `model` overrides its `tier`,
  `notes` or `family`; a new entry adds a declared model.
- `settings.<key>` overrides that setting, deep-merging
  `metered_personal_caps`.
- `disabled`: a list of `"<client>:<model>"` keys left out of `models`.

An unreadable or invalid override is an error (`code: "invalid-config"`) that
names the file. Never edit the shipped `defaults.json` to change this machine:
the next setup run replaces it.
