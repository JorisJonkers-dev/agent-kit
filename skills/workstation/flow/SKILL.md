---
name: flow
description: Use when the owner asks how the critique pipeline works, what to do next in it, or to set it up or check its budget. Says where the current feature stands and names the single next step.
disable-model-invocation: true
---

# flow

You are the **stepper**. You tell the owner where the current feature stands
in the spec-to-ticket flow and name the single next step. A critique stage
you run on the owner's word, by invoking its stage skill. Pocock's skills
(`/grill-with-docs`, `/grill-me`, `/to-spec`, `/to-tickets`, `/implement`)
only the owner can start: for those you name the exact command to type, and
the owner types it.

The runner computes the **flow state** and the next step. It is
the `critique` launcher in the `critique` skill directory installed beside this one
(`../critique/critique` from the folder this file is in):

```sh
sh <critique dir>/critique state --json '<object>'
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
| `/flow reviews` | Recent runs of the review stages in this repository, with their verdicts and spend. A review runs only when the owner asks for one. |

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

### 3. Refresh the build tracker

The **build tracker** is the page that shows the owner where every ticket of
the spec stands; the `build-tracker` skill installed beside this one says
how it works. Every feature in implementation or finishing gets one without
anyone asking. You are the orchestrator that skill names, so you may write
its state.

When the state has a `spec` and its `phase` is `implementation` or
`finishing`, run the build tracker's `refresh` from the repository root,
with one `--landed` per ticket of `state.tickets` whose `landed` is true,
named by its `id`:

```sh
sh <build-tracker dir>/build-tracker refresh --spec '<state.spec>' --landed '<id>' ...
```

The runner is the `build-tracker` launcher in the `build-tracker` skill directory
(`../build-tracker/build-tracker` from the folder this file is in).
Quote every argument: unquoted, an id such as `#183` starts a shell comment.
`refresh` runs `init` when the spec has no build tracker state yet, and
`sync` when it has one; it marks each landed ticket `merged`, renders the
page, and prints the page's path as `page`. Publish `page` as that skill's
**Republish** says: in Claude Code, the Artifact tool with `page` as its file
path, so every refresh updates the same address.

Keep a **Build tracker** line for the report: the page's address, or its path
where nothing publishes it. When `refresh` fails, the line gives its `code`
and `message`; when the republish fails, the page path and what failed.
Either way carry on: the build tracker is a view for the owner, and the flow
never waits on it.

`/flow` only writes the build tracker. It never reads the build tracker's
state file to rebuild flow state: the flow state and the issue tracker stay
its only sources.

In planning, or for a draft, skip this step; the report has no Build tracker line.

Done when `refresh` has rendered the page and it is published (or the
Build tracker line says what failed), or the phase needs no build tracker.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`report-and-act.md`](report-and-act.md): 4. Report; 5. Act on the answer
- [`finish-and-no-critique.md`](finish-and-no-critique.md): /flow finish; /flow no-critique
- [`setup.md`](setup.md): /flow setup; 1. List; 2. Walk the machine checklist; 3. Offer the repository's opt-in; 4. Close
- [`explain-budget-reviews.md`](explain-budget-reviews.md): /flow explain; /flow budget; 1. Run budget; 2. Show it in at most about six lines; 3. Offer the next step; /flow reviews; 1. Run reviews; 2. Show the runs
