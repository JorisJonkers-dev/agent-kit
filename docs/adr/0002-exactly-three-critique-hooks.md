# Exactly three critique hooks may ship

The estate ships no agent hooks. The knowledge-recall hooks were retired with
the knowledge base they wrote into, setup still purges them, and the manifest
said not to add a hooks section back without an ADR. This is that ADR. It
lifts the rule for three hooks of the critique pipeline (#182) and for nothing
else: a commit gate, a PR gate and a session-start line. Any other hook still
needs its own ADR, and the manifest validator fails CI for a hook it doesn't
name.

Every critique gate is advisory without them. A risky commit can be made
before `/review-ticket` has run, and a PR can be opened while
`/review-feature` says `GATE: FAIL`. A skill can't stop a tool call it never
sees. A hook that runs before the tool call can.

## The three hooks

| Name | Event | Runs before | Why it exists |
| :- | :- | :- | :- |
| `critique-commit-gate` | `PreToolUse` on `Bash` | a command that runs `git commit` | Blocks a risky staged diff until a passing `/review-ticket` run exists for that exact staged diff, so a risky ticket can't be committed unreviewed. Risk comes from the deterministic part of the triage only (risk paths, size, `risk:high`). The triage model is never called. |
| `critique-pr-gate` | `PreToolUse` on `Bash` | `gh pr create`, or `git push` to a branch outside the feature branch patterns | Blocks unless the latest `/review-feature` run for the current head says `GATE: PASS`, so a failing feature never reaches review. |
| `critique-session-start` | `SessionStart` | the first model turn | Prints one line suggesting `/flow setup` in an opted-in repository whose machine setup is missing or failing, and stays silent otherwise. |

The hooks are thin. Each one passes its payload to the critique runner's `gate`
command (#197), so the gate rules live in one tested place, and prints what
that command decides.

### Why there is no `after-skill` hook

The brief also proposed a hook that would run after a planning skill and
suggest the next critique stage. Pocock's planning skills (`/grill-with-docs`,
`/grill-me`, `/to-spec`, `/to-tickets`, `/implement`) are not
model-invocable: the owner types them. A typed skill is not a tool call, so no
`PreToolUse` or `PostToolUse` hook sees it run or finish. A prompt-level event
(`UserPromptSubmit`, or Claude Code's `UserPromptExpansion`) fires before the
skill starts, not after it ends, and can't tell whether it succeeded. `/flow`
reads the flow state instead and names the next step, which needs no hook.

## Rules every hook follows

- **Inert unless opted in.** A hook does nothing, and exits 0 with no output,
  in a repository that has no **Family allowlist** entry, has a `nedap` remote
  (the owner segment of any remote, on any host) or has no remote. The hooks
  are user-scope, so this is what keeps them off every other repository.
- **No model and no network.** A hook reads local files only: the staged
  diff, the flow state, the run directories and the per-machine
  configuration.
- **Well under one second.** The runner enforces an internal deadline below
  one second. Each hook is also registered with a short `timeout` (a few
  seconds) as a backstop. Both agents default to 600 seconds otherwise.
- **Fail open on its own errors.** A crash, an unreadable state file, a missed
  deadline or anything else that goes wrong inside the hook logs and allows.
  It never blocks work.
- **Fail closed only on an explicit decision.** A hook blocks only when the
  gate rules say so: a risky staged diff with no matching passing
  `/review-ticket` run, or a head whose latest `/review-feature` run is missing
  or says `GATE: FAIL`. The block is one line that explains it and names the
  skill to run.
- **Kill switch.** `CRITIQUE_OFF=1` in the environment, or critique disabled
  in the per-machine critique configuration, makes every hook a no-op.
- **Merged by name.** Setup adds the hooks to the agent's hook file by name and
  removes them on uninstall. The operator's own hooks survive, and so does the
  purge of the retired knowledge hooks.

## Scope: Claude Code and Codex only

Setup installs the hooks into Claude Code's `settings.json` and Codex's
`hooks.json`. Setup already edits both files when it purges the retired hooks,
and they share one layout: event, then matcher groups, then a `hooks` list of
`{"type": "command", "command": ..., "timeout": ...}` handlers.

Vibe and Antigravity get no critique hooks until someone checks their hook
events against the installed CLIs. Vibe has a `hooks.toml` that setup already
merges, but nobody has checked its events, payloads or blocking rules for this
use.

## Agent conventions, as checked

These facts were checked on 2026-11-01 against the versions installed on the
owner's machine, not taken from the brief. No hook was installed: the Claude
Code probe passed its hooks with `--settings` for one headless run, and nothing
was written to an agent home.

```console
$ claude --version
2.1.316 (Claude Code)
$ codex --version
codex-cli 0.166.0
$ codex features list | grep -w hooks
hooks                                    stable             true
```

### What both agents do

| Convention | Claude Code 2.1.316 | Codex 0.166.0 |
| :- | :- | :- |
| Payload | One JSON object on stdin. `PreToolUse` carries `hook_event_name`, `cwd`, `tool_name: "Bash"` and the command in `tool_input.command`. | The same shape, plus `turn_id` and `model`. `Bash` and unified exec (`exec_command`) both match as `Bash` and put the command in `tool_input.command`. |
| `SessionStart` payload | `source` is `startup`, `resume`, `clear`, `compact` or `fork`. | The same five `source` values. |
| Block with JSON | Exit 0 and print `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "..."}}`. The model sees the reason. | The same shape. The older `{"decision": "block", "reason": "..."}` is also accepted. |
| Block with exit code | Exit 2. The model sees stderr, prefixed with the hook's command. | Exit 2 with the reason on stderr. Exit 2 with empty stderr is a failed hook and does **not** block. |
| Any other exit code | A non-blocking error. The tool call goes ahead. | A failed hook. The tool call goes ahead. |
| Timeout | Doesn't block a `PreToolUse` call, unless `onFailure: "block"` is set. | A failed hook. The tool call goes ahead. |
| Context at session start | Plain-text stdout from `SessionStart` is added to the model's context. So is `hookSpecificOutput.additionalContext`. | Plain-text stdout from `SessionStart` is added as developer context. So is `hookSpecificOutput.additionalContext`. |
| Trust | Hooks in `settings.json` run as configured. | User hooks are skipped until the owner trusts them in `/hooks`. Trust is recorded against the hook's hash, so a changed hook needs trusting again. |

So the hooks block by printing the JSON `deny` and exiting 0. Both agents
accept that shape, and it shows the model the reason alone, where an exit-2
block in Claude Code also shows the hook's whole command line. Every other
outcome (another exit code, a crash, a timeout) lets the call through on both
agents, which is the fail-open rule. The hooks never set Claude Code's
`onFailure: "block"`. The session-start hook prints its one line as plain text.

### Claude Code: live probe

A probe hook (`probe-hook.sh`) appended each payload to a log. On
`SessionStart` it printed `PROBE-SESSION-CONTEXT: the codeword is heliotrope.`
On `PreToolUse` it exited 2 with a reason on stderr for `git commit`, printed
the JSON `deny` for `git push`, and exited 1 with `probe: simulated crash` on
stderr for `echo crash-probe`. One headless session ran with the probe passed
for that run only (`$P` is a scratch directory outside any repository):

```console
$ claude -p --setting-sources local --settings "$P/claude-probe-settings.json" \
    --model haiku --allowedTools Bash --output-format stream-json --verbose \
    --include-hook-events < "$P/probe-prompt.txt"
```

The prompt asked the model to quote any `PROBE-SESSION-CONTEXT` line it could
see, then run `git commit --allow-empty -m probe`, `git push origin main` and
`echo crash-probe` and report each result verbatim.

Payloads the hook received (`session_id`, `cwd` and `transcript_path` cut):

```json
{"hook_event_name":"SessionStart","source":"startup"}
{"prompt_id":"b096f7d1-…","permission_mode":"default","effort":{"level":"medium"},"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git commit --allow-empty -m probe","description":"Create empty probe commit"},"tool_use_id":"toolu_01BncUtR5LuC6fnyE1q4DXgo"}
```

Hook events in the stream, then the tool results the model got:

```text
hook_response SessionStart:startup  exit_code 0  outcome success  stdout "PROBE-SESSION-CONTEXT: the codeword is heliotrope.\n"
hook_response PreToolUse:Bash       exit_code 2  outcome error    stderr "probe: commit blocked by exit 2; run /review-ticket\n"
hook_response PreToolUse:Bash       exit_code 0  stdout {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",...}}
hook_response PreToolUse:Bash       exit_code 1  outcome error    stderr "probe: simulated crash\n"

tool_result is_error=true   PreToolUse:Bash hook error: [PROBE_LOG=$P/claude-payloads.jsonl $P/bin/probe-hook.sh]: probe: commit blocked by exit 2; run /review-ticket
tool_result is_error=true   PreToolUse:Bash hook error: probe: push blocked by JSON deny; run /review-feature
tool_result is_error=false  crash-probe
```

The model's reply began: "I found one line in my context that starts with that
prefix: `SessionStart:startup hook success: PROBE-SESSION-CONTEXT: the codeword
is heliotrope.`". So plain stdout from `SessionStart` reached the model, both
block forms stopped the call, and the crashing hook let `echo` run.

### Codex: documentation and source at the installed tag

The Codex facts come from the hooks documentation
(<https://developers.openai.com/codex/hooks.md>, read on 2026-11-01) and from
the source and generated schemas at the installed version's tag,
`rust-v0.166.0`:

```console
$ gh api "repos/openai/codex/contents/codex-rs/hooks/schema/generated/pre-tool-use.command.output.schema.json?ref=rust-v0.166.0" \
    --jq .content | base64 -d | jq -c '.definitions.PreToolUsePermissionDecisionWire'
{"enum":["allow","deny","ask"],"type":"string"}
$ gh api "repos/openai/codex/contents/codex-rs/hooks/schema/generated/session-start.command.input.schema.json?ref=rust-v0.166.0" \
    --jq .content | base64 -d | jq -c '.properties.source'
{"enum":["startup","resume","clear","compact","fork"],"type":"string"}
```

`codex-rs/hooks/src/events/pre_tool_use.rs` at that tag decides the outcome
from the exit code. Exit 0 parses the JSON and blocks on a block reason. Exit 2
blocks only when stderr is non-empty, and otherwise records "PreToolUse hook
exited with code 2 but did not write a blocking reason to stderr". Any other
code, or no code, marks the run `Failed` without blocking. In
`codex-rs/hooks/src/engine/command_runner.rs` a timeout sets the error
`hook timed out after {timeout_sec}s`, which `pre_tool_use.rs` treats as a
failed run that doesn't block. The documentation
adds that `permissionDecision: "ask"`, `continue`, `stopReason` and
`suppressOutput` are parsed but unsupported on `PreToolUse`, and that a hook
returning them is marked failed while the tool call continues. The hooks must
not print those fields.

No live Codex run was made. Running hooks that the owner hasn't trusted needs
`codex exec --dangerously-bypass-hook-trust`, which this session's permission
mode refused. Trusting a probe hook in `/hooks` would have written trust state
to `~/.codex`, which counts as installing it. The live check in #198 runs the
installed hooks in Codex once the owner has trusted them, and confirms these
facts there.

## Consequences

- `manifest.yaml` declares a `hooks` section naming the three hooks, each with
  its event, its agents and this ADR. `scripts/validate_manifest.py` accepts
  those three names and fails on any other, and a pytest test runs that check
  against the checked-in manifest, so `uv run pytest` in CI catches a stray
  hook. A `settings` section stays forbidden.
- #197 builds the runner's `gate` command, with the inert, kill-switch,
  deadline and fail-open rules above. #198 writes the three scripts, merges
  them into the Claude Code and Codex homes by name, removes them on
  uninstall, and adds the hook items to `/flow setup`.
- Codex skips the hooks until the owner trusts them in `/flow setup` through
  `/hooks`, and asks again whenever a hook's definition changes. Setup keeps
  each hook's command stable, a fixed path with no version in it, so an
  upgrade doesn't change the definition Codex trusts. Setup never bypasses
  hook trust.
- The block reason is the single line the runner returns. The hooks print it
  as the JSON `deny`'s `permissionDecisionReason` and exit 0. They don't block
  with exit 2.
- The purge of the retired knowledge hooks stays. Its basenames don't overlap
  the new hooks' names.
