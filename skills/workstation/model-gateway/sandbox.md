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
    --allowedTools Read,Grep,Glob --strict-mcp-config --disable-slash-commands
                                             # the prompt on stdin
opencode run --standalone --model <provider>/<model> --format json <prompt>
                                             # HOME, XDG_*, OPENCODE_CONFIG in <run dir>/client-home
hermes -z <wrapped prompt> --usage-file <run dir>/usage.json -m <model> --provider <provider> \
    -t web --ignore-rules                    # -t web,file for a writing run; HERMES_HOME in <run dir>/client-home;
                                             # --safe-mode off a LiteLLM route
# Pi, in its own sandbox with <dir> mounted (read-only for a read-only run), its agent directory written
# and the proxy reachable (grants: --read <dir> --write <run dir>/<client>/pi-agent --net-connect <proxy port>):
  pi -p --mode json --no-session --offline --provider litellm-<route> --model <id> \
    --tools read,grep,find,ls --no-extensions --no-skills --no-prompt-templates \
    --no-themes --no-context-files --no-mcp --no-approve --system-prompt <read-only delegate>
                                             # the prompt on stdin
codex exec --json --skip-git-repo-check --ignore-user-config -m <model> \
    -c model_reasoning_effort="<low|medium|high>" \
    -s <read-only|workspace-write> -C <dir>   # the prompt on stdin
```

A Pi run is the read tools alone (`read`, `grep`, `find`, `ls`) in a read-only
run, and for a writing run `--write <dir>` (and each git directory the run may
write) with `edit` and `write` besides and no shell; the gateway stages and
checks afterwards as for any client. Its output is Pi's JSON event stream: the
answer is the last assistant message's text, the tokens are the sum of what
every assistant message counted (`input` plus `cacheRead`), and the dollars are
what Pi priced them at from the model's entry in `models.json`, none for a
model that names no price. Pi exits 0 after a provider error, so a stream that
ends in an assistant message with `stopReason` `error` or `aborted`, or has no
assistant message, is a failed run with the error's own text.

**Docker Sandboxes (`sbx`) are required (ADR-0019, #961).** Every launch the gateway
makes (`run`, `batch`, `matrix` and the critique fan-out) runs in one, with no fallback,
and `agent-launcher confine` is retired for gateway workers. A machine where `sbx`
is missing, not signed in or cannot start a microVM refuses each launch with
`sandbox-unavailable`, naming the fix, before anything is made. Install and sign in with
`brew trust docker/tap && brew install docker/tap/sbx`, then `sbx login`.

`AGENT_KIT_SANDBOX` is unset or empty on a normal machine, which is `sbx`. `sbx-dry-run` starts nothing:
the launch is prepared, what it made is removed, and it is
refused (`sandbox-dry-run`) with the Docker Sandboxes plan its grants translate
to: mounts (`PATH` or `PATH:ro`), the variables passed in by name (never their
values), ports, and the gaps sbx does not hold the same way. `confine` is refused
(`sandbox-unknown`, retired); any other value is refused the same way, before anything is made.

`sbx` runs a Pi, OpenCode, Vibe, Claude Code, Codex or Hermes launch in its own Docker Sandbox, from
Docker's prebuilt `docker.io/sbx/<client>-image` (`pi`, `opencode`, `vibe`, `claude`, `codex`, `hermes-agent`),
pinned by digest to the newest tag dated at least two days ago (a cooldown, so a
fresh build never runs at once). Its variables reach the client through a 0600
env file that is removed after the run. A launch refuses (`sandbox-unsupported`)
before sbx is asked anything for Antigravity (its sign-in is the macOS login keychain and its state is in
the owner's `~/.gemini`, which no sandbox holds),
a plan with no written mount, or a route that names no base URL (a Vibe model on
Mistral's own provider names none and is allowed `api.mistral.ai:443`; its
`MISTRAL_API_KEY` is read from the keychain outside and passed by name, and a
login the gateway cannot read there is refused as a keychain gap). Claude Code gets its
`CLAUDE_CODE_OAUTH_TOKEN` the same way and reaches `api.anthropic.com:443` alone, never the LiteLLM proxy.
Codex signs in with the copy of `auth.json` in its run home (the home is mounted written, the owner's
file never is), reaches `chatgpt.com`, `api.openai.com` and `auth.openai.com` on 443, runs with its own sandbox off
(`-s danger-full-access`, the microVM is the sandbox), and the tokens it refreshed go back to the owner's file
after the run. Hermes takes its LiteLLM key by name and the proxy host from its run home's `config.yaml`
(a Hermes provider of the owner's own, with no `base_url`, is refused). `sandbox-unavailable`
when `sbx version` does not answer (the install command is named), or when no image
is recorded and the refresh failed. The records are `sbx-images.json` under the
gateway's state directory, one per client with its `tag`, `digest` and `checked_at`.
A launch refreshes a record that is missing or older than a day. `model-gateway sandbox refresh [--json]`
asks Docker Hub for the newest image of each enabled client and records it; it exits 1
only when every client failed and holds no record. `model-gateway sandbox status [--json]`
prints the records and makes no network call.

Sandboxes do not stack up disk: each run's sandbox is removed with `sbx rm --force`
when the run ends, however it ended. The images of superseded digests do, so a refresh
removes the gateway's images that no client pins any more (never one a running sandbox
uses). `model-gateway sandbox prune [--json] [--dry-run] [--all]` does it on demand and
also removes the `mg-*` sandboxes of gateways that are gone; it reports `freed_bytes`
(the fall in `sbx df`'s image total). `--all` also removes images that are not the
gateway's, never a pinned or used one. Setup runs it after the image pre-pull.

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
client printed on stderr, written for every launched run, empty when it printed
nothing), `events.jsonl` (below) and `result.json`, and a writing run's
`check-output.txt`. `run_id` is a UTC timestamp plus a short
random suffix. `output.json` and `stderr.txt` grow while the client runs, as
the gateway reads its pipes, and are rewritten whole when it ends.

Every launched run's result says how the client's process ended:
`exit_code` (`null` when a signal ended it or it never started), `signal`
(`SIGKILL`, only when one ended it), `stderr_tail` (the last 4 KB of its
stderr, `""` when it printed none) and, when the run failed, `failure`:
`kind` and one line of `reason`, plus `output_tail` (the last 2 KB of its
output) when its stderr was empty. The kinds are `spawn-failed` (never
started), `signalled` (a signal ended it, or the gateway was stopped and
stopped it), `timed-out`, `sandbox-denied` (its stderr says the sandbox
refused something; the sandbox's own denial log is not captured),
`harness-error` (it ran and failed by its own account: the error its output
carries, else the last line of its stderr, else the exit code) and
`gateway-error` (the gateway stopped before the result was written). Read
`failure` before retrying by hand; `reason` stays the slug the retry and the
climb read.

The client starts as the leader of a process group of its own. Its time limit
sends SIGTERM to the whole group, and SIGKILL five seconds later; what it
leaves running in the group when it exits is killed. A gateway stopped with
SIGINT, SIGTERM or SIGHUP while a client runs passes the stop on to the group,
writes the run's result and starts no further launch.

