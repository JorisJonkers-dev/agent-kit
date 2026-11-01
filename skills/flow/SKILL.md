---
name: flow
description: Use when the owner asks how the critique pipeline works, what to do next in it, or to set it up or check its budget. Says where the current feature stands and names the single next step.
---

# flow

You are the **stepper**. You tell the owner where the current feature stands
in the spec-to-ticket flow and name the single next step. A critique stage
you run on the owner's word, by invoking its stage skill. Pocock's skills
(`/grill-with-docs`, `/grill-me`, `/to-spec`, `/to-tickets`, `/implement`)
only the owner can start: for those you name the exact command to type, and
the owner types it.

The runner computes the **flow state** and the next step. It is
`critique.mjs` in the `critique` skill directory installed beside this one
(`../critique/critique.mjs` from the folder this file is in):

```sh
node <critique dir>/critique.mjs state --json '<object>'
```

A result with `"status": "refused"` carries a one-line `message`: give the
owner that line as it stands and stop. A result with `"status": "error"`
carries a `code` and a `message`: show both and stop.

Two folders come up below. The **runs folder** is
`$(git rev-parse --path-format=absolute --git-common-dir)/critique/`: one
directory per critique run, named by its run id. The **flows folder** is
`flows/` inside it: one saved flow state per feature (`*.json`, with `flow`,
`spec`, `draft` and `updated_at`), shared by every worktree.

## Arguments

| Command | What it does |
|---|---|
| `/flow [spec \| draft name]` | Where the feature stands and its next step. Without an argument, the feature is found from the branch and the tracker. |
| `/flow finish` | The feature review and its gate, once every ticket has landed. |
| `/flow no-critique` | Steps the feature through the plain Pocock flow, skipping critique on purpose. `/flow critique` turns critique back on. |
| `/flow setup` | Walks this machine's critique checklist one item at a time, then offers this repository's opt-in. |
| `/flow explain <thing>` | A short explanation of one stage, lens, depth, routing, the gate or the budget, read from the pipeline README. |
| `/flow budget` | The Vibe spend for this flow against its cap, Antigravity invocations today against the daily cap and the reserve, and which stages would fall back now. |

## /flow

### 1. Find the feature

Read the repository root (`git rev-parse --show-toplevel`) and the branch
(`git branch --show-current`). The feature is the first of these that
applies:

1. **The owner named it**: a spec (`182`, `#182`, an issue URL, or a local
   spec's path) or a draft name.
2. **The branch names an issue** (`feat/182-widgets`, `impl/192-stepper`) on
   a GitHub tracker. For each number in the branch, read
   `gh issue view <n> --json number,parent`. An issue with a parent is a
   ticket, and its parent is the spec. An issue without one is the spec when
   it has sub-issues (`gh api repos/<owner>/<repo>/issues/<n>/sub_issues
   --jq length` above 0); with none, ask the owner whether it is the spec.
3. **A saved flow belongs to the branch**: in the flows folder, a state
   whose `draft` is the branch name (use its `spec`) or whose `flow` is
   `draft:<branch>` (use the draft). The spec's flow wins over its draft's.
4. **Other saved flows exist**: list the three most recently updated, each
   with its `spec` or `draft` and `updated_at`, and ask which one, or
   whether this is a new feature.
5. **Nothing matches**: a new feature. It is a draft named after the branch;
   on the repository's default branch, ask the owner for a short name
   (letters, digits, `.`, `_`, `-`, `/`).

Done when you hold a spec or a draft name.

### 2. Read the state

Run state with `repo` and `spec`, or `repo` and `draft`. Keep `state` and
`next`.

Some steps happen where the runner cannot see them. When `next` names one,
ask the owner once whether it is done, before you report:

| `next` | Ask | On yes |
|---|---|---|
| `/grill-with-docs` | Has the grilling finished? | Record `{"grilled": true}`. |
| `/to-spec` | Has `/to-spec` written the spec? Propose the newest open issue (`gh issue list --state open --limit 5 --json number,title,createdAt`), or the newest spec file for a local tracker. | Run state with `repo`, the confirmed `spec` and the `draft`, which links the spec's flow to the draft. Later calls name the spec alone. |
| `/grill-me` for a question | Did the grilling settle question `<finding_id>`? | Record `{"answered": [{"run_id": ..., "finding_id": ...}]}`. |
| `/implement <ticket>` | Is `<ticket>` already committed? A low-risk commit leaves no record. | Record `{"landed": ["<ticket>"]}`. |

To record, run state again with the same `repo`, `spec` or `draft`, and
`"record": {...}`; its result replaces the one you kept.

Done when state returned `read` and every question above that applies has
an answer.

### 3. Report

Reply in at most about eight lines:

```
<spec or draft:name> · <phase>[ · critique skipped on purpose]
Stages: decisions <status> (<run ids>) · spec … · tickets … · feature review …
Tickets: #183 reviewed, landed · #184 skipped-low-risk, landed · #185 not-reviewed
Questions: <n> open; critical <finding_id> (<stage>, run <run_id>): <question>
Gate: none | PASS for <short head> | <gate line>
Next: <step>. <reason>
<offer>
```

- **Stages**: each stage's `status` with its `run_ids`; `not-run` needs none.
- **Tickets**: during implementation and finishing only, in the state's
  order (dependency order), each with its `review` and whether it `landed`.
- **Questions**: the open needs-human questions, critical first. Take each
  question's text from `## Questions for you` in the run's `report.md` in
  the runs folder; without it, give the finding id alone.

The last two lines come from `next`:

| `next.kind` | Next | Offer |
|---|---|---|
| `stage` | the stage skill (table below), with `next.reason` | "Run it now?" |
| `command` | "type `<next.command>`", with `next.reason` | "Type `/flow` when it is done." For `/implement`: "Start it in a fresh session, one session per ticket." For `/grill-with-docs` on a new feature: "Type `/grill-with-docs <idea>`, then `/flow`." |
| `done` | `next.reason` | none |

| `next.stage` | Stage skill | Inputs you give it |
|---|---|---|
| `decisions` | `critique-decisions` | `"flow": "draft:<name>"` for its prepare, so the run joins the draft's flow |
| `spec` | `critique-spec` | the spec; with `previous_run_id`, `"previous_run_dir"` = that run's directory in the runs folder, for the second round |
| `tickets` | `check-tickets` | the spec |
| `ticket-review` | `review-ticket` | `next.ticket` and the spec |
| `feature-review` | shown as `/flow finish` | (see `/flow finish`) |

With `resume_run_id`, the stage owes a run that was prepared but not
recorded: finish that run rather than prepare another. Its directory in the
runs folder holds `manifest.json` (with `packet_dir` and `critics`). Follow
the stage skill from its fan-out step with that `run_dir`, or, when
`findings.json` is already there (fan-out's result), from its ruling step.

Done when the reply ends with the next step, its reason and the offer.

### 4. Act on the answer

- **Yes to a stage**: load the stage skill (by name through your agent's
  skill tool, or by reading `../<skill>/SKILL.md` from this file's folder)
  and follow it with the inputs above. When it returns, go back to step 2
  for the new state and report again.
- **The owner reports a step from the table in step 2**: record it and
  report again.
- **No**: stop. The saved state keeps the place for the next `/flow`, in
  this session or a later one.

## /flow finish

1. Find the feature and read its state (steps 1 and 2). Unless `next` is the
   `feature-review` stage, report as `/flow` does and stop: that report
   names what is still due.
2. Load the `review-feature` skill and follow it with the spec (and the
   resume inputs when `next` has `resume_run_id`).
3. Read the state again, then reply with the gate line as the runner wrote
   it, one line per follow-up ticket the review filed, and the next step:
   `done` when the gate passes for the current head, else the `/implement`
   of the first follow-up.

## /flow no-critique

Find the feature (step 1), then record `{"no_critique": true}`
(`/flow critique` records `false`). From then on the next step skips every
critique stage: grill, `/to-spec`, `/to-tickets`, one `/implement` per
ticket, then done, with no feature review. The ledger keeps the choice, so
the flow shows critique was skipped on purpose. Report as `/flow` does.

## /flow setup

Takes this machine, then this repository, from nothing to ready for a
critique. You are a **guide**: one item at a time, each with its reason, its
check and its fix, and the owner decides at every item. The runner keeps the
results per machine, so a later `/flow setup` re-runs only what changed or
failed. Every check runs through the runner's `setup` command:

```sh
node <critique dir>/critique.mjs setup --json '<object>'
```

| Input | What it does |
|---|---|
| `{"action": "list", "repo": "<repository root>"}` | The checklist with each item's state, and the repository's opt-in. Leave `repo` out outside a repository. Launches no critic. |
| `{"action": "check", "check": "<item id>"}` | Runs one item and records its result. Add `"confirmed": true` once the owner has agreed to the item's cost. |
| `{"action": "quota", "daily_cap": <n>, "reserve_threshold": <n>}` | Stores the Antigravity plan's limit in this machine's routing override and checks the quota again. |

An error result carries a `code` and a `message`: show both and stop.

### 1. List

Run `list`, with `repo` when `git rev-parse --show-toplevel` names one. Keep
`items` and `repository`.

Open with one line: how many items are `current` (they passed, and nothing
they depend on changed since) and how many are due.

Done when `list` returned `"status": "listed"`.

### 2. Walk the machine checklist

Take the items in the runner's order, skipping the `current` ones. For each:

1. **Say why.** Give its `title`, then its `why` as it stands, in one
   sentence.
2. **Check.** Run `check` with the item's `id`.
   - `not-available`: give the item's `detail`. The read-only proofs all
     answer this way until agent-kit#178 lands, so give them together in one
     line once the first one answers it.
   - `needs-confirmation`: the item costs money or quota. Its `cost` says
     which: `vibe` is one trivial prompt on the personal Mistral account
     (capped at $0.05), `agy` is one of today's Antigravity invocations. The
     first time each cost comes up in this session, ask the owner before
     spending it; on yes, run `check` again with `"confirmed": true`, and
     pass `"confirmed": true` for later items of that cost without asking
     again. A no is a skip.
   - `checked`: the result is the item's `last`.
3. **Report.** One line: `pass` or `fail`, with `last.detail`.
4. **Offer the fix** on a fail: the item's `fix`. For `agy-quota`, ask for
   the plan's daily request limit and the reserve to keep for the spec and
   feature reviews (propose half the limit), then run `quota` with both.
5. **Wait.** The owner says the fix is done (run `check` again for the
   item), moves on, or says skip. Skip works at every item and records
   nothing.

Done when every item that was not `current` has passed, failed or been
skipped, with the owner's word at each.

### 3. Offer the repository's opt-in

By `repository.status`:

- `refused`: give its `message`; the repository sends nothing, so there is
  nothing to offer.
- `opted-in`: one line with its `families`, and whether `code` is on.
- `not-opted-in`: two separate offers, each with its own yes, no or skip:
  1. **Families**: which model families this repository may send documents
     to. Propose `routed_families`; the owner may name fewer. On yes, run
     the runner's `allow` command (`critique.mjs allow --json '<object>'`)
     with `{"repo": ..., "families": [...]}`.
  2. **Code**, only once families are allowed: whether the code stages
     (ticket review and feature review) may send this repository's code to
     those families. On yes, run `allow` again with the same families and
     `"code": true`.

Without `repository` (outside a repository), skip this step.

Done when the owner has answered each offer, or the repository was refused
or already opted in.

### 4. Close

One line: passed, failed, skipped and not available, counted over the
whole checklist (`current` items count as passed). Then offer `/flow`.

## /flow explain

The explanations live in one place: `README.md` in the critique skill
directory (`../critique/README.md` from the folder this file is in). Read it
on every call and explain from what it says now, never from memory or from
this file, so the answer moves with the README. When it is missing, say the
`critique` skill is not installed beside this one, re-run agent-kit setup,
and stop.

1. **Find the section** for `<thing>`:

   | The owner names | README section |
   |---|---|
   | a stage, its skill, or "stages" | `## Stages`: the stage's row, and the paragraphs under the table that bear on it |
   | a lens, or "lenses" | `## Lenses`: the lens's line |
   | depth, base or deep | `## Depth` |
   | routing, fallbacks or retuning | `## Routing` |
   | a gate, the hooks or the kill switch | `## Gates and the kill switch` |
   | budget, quota or cost | `## Budget and quota` |

   A stage or lens is any name in that section's table or list. With no
   `<thing>`, or one no section covers, list the six things above that can
   be explained, and stop.

2. **Explain** in at most about six lines, in the README's words and terms,
   from that section alone: for one stage or lens, only its row or line and
   what bears on it. End with a line naming the README section to read for
   more.

Done when the reply covers the one thing asked, and every claim in it is in
that README section.

## /flow budget

Shows what the next costly stage would draw on, before it runs. You
**report**: the runner computes every figure from the ledger and routing,
this machine's routing override included, by the rules prepare and fan-out
enforce the limits with. Never work a figure out yourself, round it
differently, or add a stage the runner did not list.

### 1. Run budget

Find the feature (step 1 of `/flow`), then run the runner's `budget`
command with `repo` and `spec`, or `repo` and `draft`:

```sh
node <critique dir>/critique.mjs budget --json '{"repo": "<repository root>", "spec": "<spec>"}'
```

Done when it returned `"status": "read"`, or you stopped on a refusal or
an error.

### 2. Show it in at most about six lines

```
Vibe: $<vibe.spent_usd> of the $<vibe.cap_usd> cap for this flow
Antigravity: <antigravity.invocations> of <antigravity.daily_cap> today (<antigravity.day>, UTC); past <antigravity.reserve_threshold> only <antigravity.reserved_stages> may use it
Would fall back now:
- <stage>: <what happens>
```

- Show amounts with two decimals.
- When `vibe.flows` has two entries, add the split to the Vibe line:
  `(spec $x.xx, draft $y.yy)`. Each flow key is held to the cap on its
  own: the decisions stage charges the draft's, every later stage the
  spec's, so a stage's line can differ from the total.
- List only the `stages` whose `vibe` or `antigravity` is not null, in the
  runner's order. For each:
  - `antigravity` set: for each `falls_back` entry, "the `<client>` slot
    runs through `<to>` instead" ("is left empty" when `to` is null; say
    "deep" for a deep slot), then the `antigravity` reason in brackets.
  - `vibe` set: "its Vibe critics would be skipped", then the `vibe`
    reason in brackets. A skipped critic is not a failure and gets no
    fallback.
- When no stage is listed, write `Would fall back now: nothing`.

The fallback named is routing's. When the stage runs, prepare also applies
the repository's family allowlist, the orchestrator's family and whether
each client is ready on this machine; say so only when the owner asks why
a different critic ran.

Done when the owner has the lines.

### 3. Offer the next step

End with one sentence. If the stage `/flow` would run next falls back or
skips, say so, and that the caps live in this machine's routing override
(`~/.config/agent-kit/critique-routing.json`); `/flow explain budget` says
how to change them. Never edit the override or routing unless the owner
asks. Otherwise offer to return to `/flow`.
