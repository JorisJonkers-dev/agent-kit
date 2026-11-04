---
name: model-gateway
description: Holds the model gateway's runner and its shipped defaults; it is not a workflow of its own, since the delegate skill, coming later, drives it.
---

# model-gateway

This folder is what the model gateway runs on. It does nothing on its own: an
agent calls the runner, and the `delegate` skill (a later ticket) is what
teaches an orchestrator to plan with it. The runner answers three questions:
`models`, which client models this machine can use right now; `usage`, what
each client can do now and what it has spent; and `run`, one read-only
delegation to one of them.

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
- `config` — the models Vibe's own configuration declares, listed by the alias
  `vibe --model` accepts. The gateway reads `$VIBE_HOME/config.toml`, else
  `~/.vibe/config.toml`.
- `declared` — the shipped catalogue, for Codex and Claude Code.

A listed model takes its family, tier and notes from the catalogue entry that
matches its id. A model the catalogue does not know is still listed, with its
family worked out from its id, `"tier": null` and no notes: the `delegate`
skill must not pick a null-tier model, because nobody has judged what it
handles well. A model whose family the gateway cannot work out is not offered
at all: it is named in `skipped` with the reason, so it never vanishes
silently, and adding it to the catalogue in `defaults.json` or the machine
override brings it back.

A listing that fails, or a Vibe configuration that cannot be read, leaves that
client out and names it in `missing` with the reason; the other clients are
still listed. Hermes lists nothing until its own ticket lands, and is not
reported missing for that.

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
only for an inline run), `time_limit_s` (default 900) and `max_price_usd`.
`workflow` is required (`code: "invalid-input"`). A run can go to `client:
"vibe"`, `"agy"` (Antigravity), `"claude"` (Claude Code) or `"opencode"`, in
`mode: "read-only"` or `"write"` (below); any other client or mode is
`code: "not-supported-yet"`.

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
- `provider-not-allowed` — an OpenCode model is named `<provider>/<model>`, and
  only the providers `clients.opencode.providers` names (default
  `["opencode-go"]`) are allowed. Every other provider is refused, and so is
  `opencode/<model>`, OpenCode's pay-per-token Zen, even when the machine lists
  it: a run never reaches a pay-per-token model.
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
and the ledger line are the same for every client.

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
`AGENT_KIT_DELEGATION=1` in its environment. Claude Code and OpenCode have no
time limit of their own, so the outer timer stops them at `time_limit_s`;
Antigravity's own `--print-timeout` fires first, a minute before the outer one:

```sh
agy --print <prompt> --model <id> --output-format json --effort <low|medium|high> \
    --mode plan --print-timeout <s>s
claude -p --model <id> --output-format json --permission-mode plan \
    --allowedTools Read,Grep,Glob            # the prompt on stdin
opencode run --model <provider>/<model> --format json <prompt>
```

Antigravity's effort is the id's last word (`gemini-3.8-flash-high` is
`high`), else `medium`. Tokens and dollars come from each client's own output
(agy's `usage`, Claude Code's `usage` and `total_cost_usd`, the sum of
OpenCode's `step_finish` events); agy prices nothing, as it is quota, not
money. `max_price_usd` is passed to Vibe only.

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
client's own output reports an error) or, for a writing run, `check-failed`,
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
`input.json`, `output.json` (the client's full output) and `result.json`, and
a writing run's `check-output.txt`. `run_id` is a UTC timestamp plus a short
random suffix.

Every run also appends one JSON line to the machine-wide ledger,
`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl` (else
`~/.local/state/...`): workflow, client, model, family, mode, repository key,
tokens, dollars, outcome, duration and run id. Writes take an exclusive lock
file beside it, so two gateway processes never interleave a line. Dollars come
from the client's own output when it prices the run, else from the tokens times
the catalogue's `prices` for the model.

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
   accept-edits`, OpenCode with its default agent. The prompt adds "Edit only
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
that made it ends. Until a workflow store exists, the caller says when it
ended:

```sh
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
  cost class. `providers` is the list of providers a run may reach through
  OpenCode (`provider-not-allowed` otherwise); it defaults to `["opencode-go"]`.
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
- `models`: an entry matching `client` and `model` overrides its `tier`,
  `notes` or `family`; a new entry adds a declared model.
- `settings.<key>` overrides that setting, deep-merging
  `metered_personal_caps`.
- `disabled`: a list of `"<client>:<model>"` keys left out of `models`.

An unreadable or invalid override is an error (`code: "invalid-config"`) that
names the file. Never edit the shipped `defaults.json` to change this machine:
the next setup run replaces it.
