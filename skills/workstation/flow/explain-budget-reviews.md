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
sh <critique dir>/critique budget --json '{"repo": "<repository root>", "spec": "<spec>"}'
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
  own: the decisions and plan stages charge the draft's, every later stage the
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

## /flow reviews

The **review stages** (`adr-review`, `decision-recovery` and `architecture-review`) look at a repository's decision
records or code, not at one feature. They sit outside the feature's flow:
`/flow` never names one as the next step, and a review is never due. Each
review run is a flow of its own, keyed by its run id, with its own Vibe cap,
so a review never draws on a feature's budget. This entry only lists them.

### 1. Run reviews

Run the runner's `reviews` command with the repository root
(`git rev-parse --show-toplevel`); add `"limit": <n>` when the owner asks
for more or fewer than the last 10:

```sh
sh <critique dir>/critique reviews --json '{"repo": "<repository root>"}'
```

A refusal carries a one-line `message`: give it as it stands and stop.

Done when it returned `"status": "read"`, or you stopped on a refusal or
an error.

### 2. Show the runs

One line per entry of `runs`, newest first, as the runner ordered them:

```
<stage> · <run_id>[ · round 2] · <accepted> accepted, <needs-human> questions, <rejected> rejected, <hint> hints · $<spent_usd>
```

- Take each figure from the entry: `stage`, `run_id`, `round`, the counts in
  `verdicts` and `spent_usd`, with two decimals. Never work one out
  yourself.
- A run with `recorded` false is not recorded yet: write `not recorded` in
  place of the counts.
- With no runs, write `No review runs yet.`

End with one line: a review runs only when the owner asks for one. Do not
offer to start one, and never call one due.

Done when the owner has the lines.
