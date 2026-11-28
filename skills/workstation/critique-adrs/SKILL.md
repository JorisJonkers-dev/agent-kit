---
name: critique-adrs
description: Use when the owner asks to review a repository's ADRs or decision log. Critics from other model families review the records read-only; you rule on each finding, make the editorial fixes, and ask the owner about anything that would change a decision.
disable-model-invocation: true
---

# critique-adrs

You are the **orchestrator**: critics from other model families review a
frozen copy of the repository's decision records (the **packet**), and you
are the only one who rules on their findings. `adr-review` is a **review
stage**: it runs only when the owner asks, outside any feature's flow, and
each run is a flow of its own with its own spend.

The runner does the deterministic work. It is the `critique` launcher in the `critique`
skill directory installed beside this one (`../critique/critique` from the
folder this file is in). Each command takes one JSON object and prints one:

```sh
sh <critique dir>/critique <prepare|fan-out|refute|record|allow|check-adrs|check-rewrite|check-routing|cold-read> --json '<object>'
sh <critique dir>/critique <command> --input <file.json>
```

A result with `"status": "refused"` carries a one-line `message`: give the
owner that line as it stands and stop. A result with `"status": "error"`
carries a `code` and a `message`: show both and stop. The packet and the
runner's files in the run directory stay as the runner wrote them; your own
files (`verdicts.json`) go beside them. `check-adrs` prints
`"status": "failed"` and exits 3 when the log has a structural error; that
is a result, not an error.

## Arguments

`/critique-adrs [adr ...] [base|deep]`

- **adr**: the records to review, as repository paths
  (`docs/adr/0003-review-stages-run-outside-the-feature-flow.md`). With none,
  every record in `docs/adr/` is reviewed. With some, the rest of the log
  and the glossary go in as references.
- **base** or **deep**: the **depth**. Pass it to prepare as `"depth"` only
  when the owner named it; otherwise the runner chooses.

## Model tier

This stage's model class is the tier `tier-review`; you never pick a model by hand. The critics the runner
launches are routed by `routing.json` in the critique directory, which keeps one family per critic. Any other
model work you hand off for this stage, such as a second read or a check of one finding, runs as
`model-gateway run --tier tier-review --json '{...}'` (read-only, with `repo` so the data policy applies). The tier's Pi client reaches public data only (`data_scope: public`), so for a private repository the gateway refuses the run with its policy reason: report that refusal, and do not retry on a model you pick yourself. The run writes its routing record on its ledger line (`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl`, else `~/.local/state/agent-kit/model-gateway/ledger.jsonl`; the line whose `run_id` is the run's): `tier`, `harness`, `served_model`, `attempted_fallbacks`, `fallback_fired`, `cost_usd` and `routing_capture`. Read that line instead of guessing, and say in your report which tier ran, which model served it and whether a fallback fired. A `null` field is reported as unknown, with the `routing_capture` that explains it.
Claude Code stays the interactive author and the orchestrator that rules on findings. Never send Claude traffic through LiteLLM: a tier run takes no `client`, `model`, `profile` or `harness`, so it cannot name a Claude model.

## Your model family

Prepare needs `orchestrator_family`, the vendor of the model you are running
as (not of the app you run in), so no critic shares your family:

| You are | `orchestrator_family` |
|---|---|
| Claude | `anthropic` |
| GPT (Codex) | `openai` |
| Mistral (Vibe's default) | `mistral` |
| GLM | `zhipu` |
| DeepSeek | `deepseek` |
| Gemini (Antigravity app, `agy`) | `google` |

## Steps

### 1. Prepare

Run prepare with `stage: "adr-review"`, `orchestrator_family`, `repo` (the
repository's root, `git rev-parse --show-toplevel`), `"adrs": [...]` when
the owner named records, and `depth` when given. Never pass `flow`: a
review run is its own flow.

- **Refused with `no-adrs`**: the repository has no records in `docs/adr/`.
  Say so and stop.
- **Refused with `not-opted-in`**: this repository has no **family
  allowlist** entry, so nothing may be sent yet. Offer the opt-in: name the
  families (`mistral`, `zhipu`, `deepseek`, `google`), point at
  `routing.json` in the critique directory for which critic clients each one
  feeds, and say that a base review needs two families other than yours.
  Say that the entry's code permission is off by default, and that with it
  off the review sends the records and the glossary only and skips the
  checks that need the code; turning it on (allow with `"code": true`) also
  sends a sample of the code. Its history permission (commit messages and
  issue text, turned on with allow and `"history": true`) is off by default
  too, and this stage never needs it. If the
  owner chooses families, run allow with `repo` and `families`, then
  prepare again. If they decline, run `check-adrs` with `repo` instead (it
  sends nothing anywhere), report what it found, and stop.
- **Any other refusal** (a `nedap` or remote-less repository): give the
  runner's line and stop.
- **Ready**: keep `run_dir`, `packet_dir` and `critics`. With no
  `repository` in the result the run is **documents only**: tell the owner
  so, and that the report names each check it skipped. For each entry in
  `dropped` with reason `unavailable`, quote its `detail` as it stands and
  say whether `replaced_by` took its slot or the stage runs with fewer
  critics; ask whether to continue, and stop if the owner declines. With no
  critics left at all, go on: record then reports the run as degraded.

Done when prepare returned `ready` and the owner has agreed to every
unavailable critic.

### 2. Fan out

Run fan-out with `run_dir`. It runs every critic at once and returns when the
slowest one ends, which can take as long as the longest `time_limit_s` in the
routing (`routing.json`, or `~/.config/agent-kit/critique-routing.json` where
it overrides one) plus a minute. Give the shell command that long, or run it
in the background and wait; a shell timeout that kills fan-out loses the run.

For each critic with status `failed` whose `detail` says its CLI is missing
or signed out, quote the `detail` and ask whether to continue with the other
critics' findings; stop before recording if the owner declines.

### 3. Read the pre-check

Open `<packet_dir>/check/adrs.json`, the decision-log checker's report on the
whole log. Its results are settled: build on them and never recompute them.

- **`unresolved_links`** and **`cites_superseded`**: each is a broken
  reference. Where the packet shows the right target, its fix is an
  editorial edit in step 6; otherwise it is a question in step 8.
- **`drifted`** and **`stale`**: the runner computed both from the working
  tree and its history. A drifted record names a path or a symbol (in
  backticks) that no file holds any more; a stale one states a decision date
  earlier than the last change to a file it names. They never make the check
  `failed`. Whether the record or the code is right is the owner's call:
  list each in step 8 as a question, and edit nothing for them.
- **`numbering`** gaps and duplicates and **`unknown_statuses`**: list each
  in the change list as found. Renumbering or restating a status is the
  owner's call: ask before changing either.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`steps-4-6.md`](steps-4-6.md): 4. Rule on every finding; 5. Refute; 6. Fix the records
- [`steps-7-11.md`](steps-7-11.md): 7. Check the rewrite; 8. Ask the owner; 9. Record; 10. Second round; 11. Return
