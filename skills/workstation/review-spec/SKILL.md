---
name: review-spec
description: Use after /to-spec produced or changed a spec. Critics from other model families review the spec read-only; you rule on each finding against the evidence, fix the spec, and ask the owner about anything that touches a decision.
disable-model-invocation: true
---

# review-spec

You are the **orchestrator**: critics from other model families review a frozen
copy of the spec (the **packet**), and you are the only one who rules on their
findings.

The runner does the deterministic work. It is the `critique` launcher in the `critique`
skill directory installed beside this one (`../critique/critique` from the
folder this file is in). Each command takes one JSON object and prints one:

```sh
sh <critique dir>/critique <prepare|fan-out|refute|record|allow> --json '<object>'
sh <critique dir>/critique <command> --input <file.json>
```

A result with `"status": "refused"` carries a one-line `message`: give the
owner that line as it stands and stop. A result with `"status": "error"`
carries a `code` and a `message`: show both and stop. The packet and the
runner's files in the run directory stay as the runner wrote them; your own
files (the spec copy, `verdicts.json`) go beside them.

## Arguments

`/review-spec [spec] [base|deep]`

- **spec**: the issue (`167`, `#167`, an issue URL) or, with a local markdown
  tracker, the spec's path. Without one, use the spec `/to-spec` published in
  this conversation; failing that, ask the owner.
- **tier**: run the critique on the LiteLLM tiers instead of the routed
  critics (see "Tier review" below).
- **base** or **deep**: the **depth**. Pass it to prepare as `"depth"` only
  when the owner named it; otherwise the runner chooses.

## Model tier

This stage's model class is the tier `tier-review`; you never pick a model by hand. The critics the runner
launches are routed by `routing.json` in the critique directory, which keeps one family per critic. Any other
model work you hand off for this stage, such as a second read or a check of one finding, runs as
`model-gateway run --tier tier-review --json '{...}'` (read-only, with `repo` so the data policy applies). The tier's Pi client reaches public data only (`data_scope: public`), so for a private repository the gateway refuses the run with its policy reason: report that refusal, and do not retry on a model you pick yourself. The run writes its routing record on its ledger line (`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl`, else `~/.local/state/agent-kit/model-gateway/ledger.jsonl`; the line whose `run_id` is the run's): `tier`, `harness`, `served_model`, `attempted_fallbacks`, `fallback_fired`, `cost_usd` and `routing_capture`. Read that line instead of guessing, and say in your report which tier ran, which model served it and whether a fallback fired. A `null` field is reported as unknown, with the `routing_capture` that explains it.
Claude Code stays the interactive author and the orchestrator that rules on findings. Never send Claude traffic through LiteLLM: a tier run takes no `client`, `model`, `profile` or `harness`, so it cannot name a Claude model.

## Tier review

`/review-spec [spec] tier` replaces steps 1 and 2 with one command. Claude Code
is the author; the reviewers are three model families from the `tier-review`
chain, free first, then Vibe Flash, then Go Flash, none of them yours.

```sh
sh <critique dir>/critique tier-review --json '{"repo": ..., "spec": ..., "orchestrator_family": "anthropic"}'
```

It prepares the run, fans it out, and writes `<run_dir>/synthesis.md`: each
reviewer with the model it asked for, the model LiteLLM says served it and its
fallback count, then the findings by severity with who raised them. Its result
carries the same routing per reviewer (`reviewers[].routing`); the gateway ledger
holds one record per reviewer with `tier`, `litellm_model_id`, `served_model`,
`attempted_fallbacks`, `fallback_fired` and `routing_capture`.

- **Refused**: the result's `message` says why (`not-opted-in` and the other
  prepare refusals, `too-few-families` with what was passed over, or
  `tier-not-ready` when Pi, its key or sbx is missing). Give the
  line as it stands and stop. Free steps are for a repository GitHub says is
  public; a private one, or one whose visibility cannot be read, starts at
  Vibe Flash.
- **`status: failed`**: fewer than three families reviewed (a reviewer LiteLLM
  served from another model counts as no review, and is retried through a family
  not yet in the review). Say so; the synthesis still lists what was found.
- **`reviewed`**: you are the coordinator. Read `synthesis.md`, then do steps 3
  and 5 to 9 on `run_dir`; the synthesis you write is your verdicts and the
  `summary` record returns. The models of `tier-strong-open` are not in this
  chain: they live only there.

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

Run prepare with `stage: "spec"`, `orchestrator_family`, `repo` (the
repository's root, `git rev-parse --show-toplevel`), `spec`, and `depth` when
given. Leave `comments` out, so earlier change comments stay out of the
packet; pass `"comments": true` only when the owner says decisions live in the
issue's comments.

- **Refused with `not-opted-in`**: this repository has no **family
  allowlist** entry, so nothing may be sent yet. Offer the opt-in: name the
  families (`mistral`, `zhipu`, `deepseek`, `google`), point at
  `routing.json` in the critique directory for which critic clients each one
  feeds, and say that a base critique needs two families other than yours.
  If the owner chooses families, run allow with `repo` and `families`, then
  prepare again. If they decline, stop: nothing was sent. In the offer, say
  that the entry's code permission and its history permission (commit
  messages and issue text, turned on with allow and `"history": true`) are
  both off by default, and that this stage needs neither.
- **Any other refusal** (a `nedap` or remote-less repository, an unreadable
  tracker): give the runner's line and stop.
- **Ready**: keep `run_dir`, `packet_dir`, `artifact` and `critics`. For each
  entry in `dropped` with reason `unavailable`, quote its `detail` as it
  stands (it is setup's wording, for example `antigravity: needs sign-in: …`)
  and say whether `replaced_by` took its slot or the stage runs with fewer
  critics. Then ask whether to continue with the remaining critics, and stop
  if the owner declines. With no critics left at all, go on: fan-out and
  record then report the stage as degraded, and the flow continues.

Done when prepare returned `ready` and the owner has agreed to every
unavailable critic.

### 2. Fan out

Run fan-out with `run_dir`. It runs every critic at once and returns when the
slowest one ends, which can take as long as the longest `time_limit_s` in the
routing (`routing.json`, or `~/.config/agent-kit/critique-routing.json` where
it overrides one) plus a minute. Give the shell command that long, or run it in
the background and wait; a shell timeout that kills fan-out loses the run.

For each critic with status `failed` whose `detail` says its CLI is missing or
signed out, quote the `detail` and ask whether to continue with the other
critics' findings; stop before recording if the owner declines. A `violated-readonly` critic is already discarded; the summary
warns about it.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`steps-3-9.md`](steps-3-9.md): 3. Rule on every finding; 4. Refute; 5. Fix the spec; 6. Ask the owner; 7. Record; 8. Second round; 9. Comment and return
