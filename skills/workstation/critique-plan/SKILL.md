---
name: critique-plan
description: Use after /improve-codebase-architecture has grilled a refactor candidate and before /to-spec, or to critique an existing refactoring plan. Drafts the plan when none exists, then critics from other model families check its steps, their order and their anchors; you rule on each finding and ask the owner whatever would change the target. Nothing changes until the owner answers.
disable-model-invocation: true
---

# critique-plan

You are the **orchestrator**: critics from other model families read a frozen
copy of the refactoring plan and a sample of the code it targets (the
**packet**) and report what is wrong with its steps, their order and their
anchors. You rule on each finding, and every finding that would change the
target goes to the owner as a question. Nothing changes until the owner
answers.

The runner does the deterministic work. It is the `critique` launcher in the `critique`
skill directory installed beside this one (`../critique/critique` from the
folder this file is in). Each command takes one JSON object and prints one:

```sh
sh <critique dir>/critique <state|check-plan|prepare|fan-out|refute|record|allow> --input <file.json>
```

A result with `"status": "refused"` carries a one-line `message`: give the
owner that line as it stands and stop. A result with `"status": "error"`
carries a `code` and a `message`: show both and stop. Keep your own files
(the input files, `verdicts.json`) in the run directory beside the runner's.

## Arguments

`/critique-plan [base|deep]`: the **depth**. Pass it to prepare as
`"depth"` only when the owner named it; otherwise the runner chooses.

When `/flow` starts this stage it names the draft flow
(`draft:<name>`). Pass it to prepare as `"flow"`, so the run counts in that
draft; without one, the run is a flow of its own. The owner starts the first
run; the file it drafts marks the draft as a refactor from then on.

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

### 1. Find or draft the plan

Run `state` with `{"repo": <root>, "draft": "<name>"}`. It reports
`plan_path`, and `refactor` says whether the plan file exists.

- **`refactor` false**: there is no plan yet. Draft it from the grilling in
  this conversation and write it at `plan_path`, creating folders as needed.
  Never commit it: the plan lives under `.scratch/<draft>/refactoring-plan.md`
  when the repository tracks specs as local markdown, otherwise in the git
  common directory beside the flow state.
- **`refactor` true**: read the plan at `plan_path` and critique it as it
  stands.

The plan has these sections: `## Goal` (the outcome it unblocks; say whether
behaviour changes), `## Target` (modules and interfaces in glossary terms,
seams, dependency rules, ADRs relied on, superseded or needed),
`## Safety net`, `## Steps`, `## Not doing`, `## Open decisions`.

Each step is `### S01: <title>` followed by these fields as
`- **Field**: value` lines:

| Field | Holds |
|---|---|
| `Kind` | exactly one of `characterize`, `tidy`, `structure`, `behaviour`, `expand`, `migrate`, `contract`, `remove-transitional` |
| `Technique` | the named refactoring the step applies |
| `Anchor` | a backticked fact row path, an ADR id like `ADR-0004`, or `prerequisite-of:Sn` |
| `Touches` | the files and modules the step edits |
| `Safety net` | the tests that catch a mistake in this step |
| `Main stays green because` | why the branch can merge after this step alone |
| `Rollback` | how to undo the step |
| `Transitional element` | the toggle or shim it adds, and the step that removes it |
| `Blocked by` | the steps that must land first |
| `Done when` | the observable condition that closes the step |

The rules the plan must keep:

- One Kind per step.
- Behaviour steps only when the Goal says behaviour changes; tidy first, in
  its own steps.
- Structure work on untested code is blocked by a characterize step.
- Every expand has a migrate and a contract.
- Every toggle or shim has a removal step.
- Steps form a DAG and are worked from the leaves.
- Each step fits one session and ships alone.
- File paths appear only inside fact anchors (backticks) and Touches, never
  in prose.

Then run `check-plan` with `{"plan": <plan_path>, "repo": <root>}` and repair
the plan until it exits clean. It exits 3 while a rule is broken, and each
error names a step and a rule.

Show the owner the draft only when there was no plan before, and ask for a
"yes" before critics read it.

Done when the plan exists at `plan_path`, `check-plan` exits clean, and the
owner has said yes to a draft you wrote.

### 2. Prepare

Run prepare with `stage: "plan"`, `orchestrator_family`, `repo` (the
repository's root, `git rev-parse --show-toplevel`), `flow` (the draft flow),
and `plan` (an explicit path) and `depth` when given.

This stage sends code: a sample of the tree at the head of the branch the
plan targets, named with `head` when it is not HEAD. So the allowlist entry's
code permission must be on.

- **Refused with `code-not-allowed`**: the entry exists but may not send
  code. Offer allow with `"code": true`, then prepare again. If the owner
  declines, stop: nothing was sent.
- **Refused with `not-opted-in`**: this repository has no **family
  allowlist** entry, so nothing may be sent yet. Offer the opt-in: name the
  families (`mistral`, `deepseek`, `zhipu`), point at `routing.json` in the
  critique directory for which critic clients each one feeds, and say that
  this stage's base roster needs DeepSeek and Mistral (DeepSeek Flash with
  the incremental-safety lens and Mistral Small with the evidence-anchoring
  lens), and that the deep roster adds GLM with the coupling and test-seams
  lenses. Say that the entry's history permission (commit messages and issue
  text, turned on with allow and `"history": true`) is off by default and
  that this stage does not need it. If the owner chooses families, run allow
  with `repo` and `families`, then prepare again. If they decline, stop:
  nothing was sent.
- **Refused with `no-plan`**: there is no plan to send. Draft it (step 1) and
  prepare again.
- **Any other refusal**: give the runner's line and stop.
- **Ready**: keep `run_dir`, `packet_dir` and `critics`. For each entry in
  `dropped` with reason `unavailable`, quote its `detail` as it stands and
  say whether `replaced_by` took its slot or the stage runs with fewer
  critics, then ask whether to continue, and stop if the owner declines. A
  `quota` drop needs no question. With no critics left at all, go on: the
  runner reports the stage as degraded and the flow continues.

Depth is deep above 8 steps or with a migrate or contract step that touches
stored data. The runner decides; pass `depth` only when the owner named it.

Done when prepare returned `ready` and the owner has agreed to every
unavailable critic.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`steps-3-10.md`](steps-3-10.md): 3. Fan out; 4. Rule on every finding; 5. Refute; 6. Revise the plan and ask the owner; 7. Record; 8. Second round; 9. Gate; 10. Return
