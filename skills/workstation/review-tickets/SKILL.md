---
name: review-tickets
description: Use after /to-tickets and before the first /implement. A script checks the spec's tickets for dependency cycles, dangling blockers and uncited stories, a critic from another model family lints them read-only; you fix ticket structure on the tracker and ask the owner before changing scope.
disable-model-invocation: true
---

# review-tickets

You are the **orchestrator**: a deterministic ticket checker and a critic
from another model family review a frozen copy of the spec and its tickets
(the **packet**), and you are the only one who rules on what they found.

The runner does the deterministic work. It is the `critique` launcher in the `critique`
skill directory installed beside this one (`../critique/critique` from the
folder this file is in). Each command takes one JSON object and prints one:

```sh
sh <critique dir>/critique <prepare|fan-out|refute|record|allow|check-tickets> --json '<object>'
sh <critique dir>/critique <command> --input <file.json>
```

A result with `"status": "refused"` carries a one-line `message`: give the
owner that line as it stands and stop. A result with `"status": "error"`
carries a `code` and a `message`: show both and stop. `check-tickets` prints
`"status": "failed"` and exits 3 while the tickets hold a dependency cycle;
that is a result, not an error. The packet and the runner's files in the run
directory stay as the runner wrote them; your own files (ticket copies,
`verdicts.json`) go beside them.

## Arguments

`/review-tickets [spec] [base|deep]`

- **spec**: the spec issue (`181`, `#181`, an issue URL) whose sub-issues are
  the tickets, or, with a local markdown tracker, the spec's path (its
  tickets are the files of the `issues/` folder beside it). Without one, use
  the spec `/to-tickets` worked from in this conversation; failing that, ask
  the owner.
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

Run prepare with `stage: "tickets"`, `orchestrator_family`, `repo` (the
repository's root, `git rev-parse --show-toplevel`), `spec`, and `depth` when
given. Prepare runs the ticket checker itself and writes its report into the
packet as `check/tickets.json`, beside `spec/` and `tickets/`.

- **Refused with `no-tickets`**: the spec has no tickets yet. Say so and stop;
  the next step is `/to-tickets`.
- **Refused with `not-opted-in`**: this repository has no **family
  allowlist** entry, so nothing may be sent yet. Offer the opt-in: name the
  families (`mistral`, `zhipu`, `deepseek`, `google`) and point at
  `routing.json` in the critique directory for which critic clients each
  feeds (the tickets stage runs DeepSeek Flash, and Mistral Small at deep
  depth). If the owner chooses families, run allow with `repo` and
  `families`, then prepare again. If they decline, run `check-tickets` with
  `repo` and `spec` instead (it sends nothing anywhere), act on its report
  as in steps 3 and 6, and skip the critic's steps.
- **Any other refusal** (a `nedap` or remote-less repository, an unreadable
  tracker): give the runner's line and stop.
- **Ready**: keep `run_dir`, `packet_dir`, `artifact` and `critics`. For each
  entry in `dropped` with reason `unavailable`, quote its `detail` as it
  stands and say whether `replaced_by` took its slot or the stage runs with
  fewer critics; ask whether to continue, and stop if the owner declines.
  With no critics left at all, go on: the checker's report still holds.

### 2. Fan out

Run fan-out with `run_dir`. It returns when the critics end, which can take
the stage's `time_limit_s` in routing (`routing.json`, or
`~/.config/agent-kit/critique-routing.json` where it overrides one) plus a
minute; give the shell command that long. For a `failed` critic whose
`detail` says its CLI is missing or signed out, quote the `detail` and ask
whether to continue.

### 3. Read the checker's report

Open `<packet_dir>/check/tickets.json`. Its results are mechanical; act on
each kind:

- **`cycles`**: each cycle lists tickets where each is blocked by the next
  and the last by the first. For each, read every ticket in it and find the
  blocker link that is wrong: the blocked ticket does not need what the
  blocker delivers. That link's removal is a structural fix (step 6). When
  every link in the cycle looks needed, the tickets are cut wrong: ask the
  owner how to split them (step 7).
- **`dangling_blockers`**: a blocker that is no ticket of this spec. Most are
  tickets of another spec, which is fine: check each one exists. One that
  names nothing (a typo, a deleted ticket) is a dependency edit.
- **`coverage`**: with `checked`, each story in `uncited` is a spec
  requirement no ticket cites. Find whether some ticket delivers it anyway:
  then citing the story there is a structural fix; otherwise delivering it
  changes scope, so it is a question.
- **`size.over_criteria`**: tickets above the criterion limit; weigh them
  with the critic's `too-big` findings in step 4.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`steps-4-10.md`](steps-4-10.md): 4. Rule on every finding; 5. Refute; 6. Fix the tickets; 7. Ask the owner; 8. Record; 9. Check again; 10. Comment and return
