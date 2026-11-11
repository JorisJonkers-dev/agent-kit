---
name: build-tracker
description: Use at the start of any multi-ticket build (/implement-spec, a council run, a /flow feature in implementation) and on every ticket status change in it. Seeds a page of the spec's tickets from the issue tracker, with a dependency graph and a list per milestone, and keeps it current for the owner.
disable-model-invocation: true
---

# build-tracker

The build tracker shows the owner where every ticket of a spec stands while
it is built. Its state file is the single source of the page; the page is
rendered from it and never edited by hand. It is a view for the owner: the
issue tracker stays the source of what is done.

You are the **orchestrator**: the agent the owner works in, which hands
tickets to implementers and mergers. The runner is `build-tracker.mjs` in the
folder this file is in (see **Commands**).

## Who writes the state

Only the orchestrator writes the state file. Implementers and mergers report
back to it: each says in its report which ticket it started, finished or is
stuck on, and the orchestrator runs the commands. An implementer or merger
reading this skill reports the event and its detail (the merge commit, what
the owner must do) and never runs `init`, `sync` or `set`.

## Steps

### 1. Start the build tracker

Start it at the start of any multi-ticket build: `/implement-spec`, a
council run, or a `/flow` feature in implementation. `<ref>` is the spec: its
issue number or URL on GitHub, or the path of a local markdown spec.

Run `init --spec <ref>` with one `--milestone "<name>=<ticket>"` per
milestone, in the order the spec lists them. A milestone's ticket is the one
that closes it, such as its live check. A spec without milestones gets no
`--milestone`. When `init` refuses because a state for the spec exists, this
build continues an earlier one: run `sync --spec <ref>` instead. `sync`
keeps the earlier state's milestones. When the spec's milestones changed,
delete the state file that `path --spec <ref>` prints and run `init` again.

Then run `path --spec <ref>`. It prints the state file and the page path;
use exactly those two in every later command.

A `/flow` feature in implementation gets this step and step 2 from `/flow`
itself, through `refresh`. To give it milestones, run `init` with them
before the first `/flow` call in implementation; `refresh` keeps them.

Done when `path` has printed both paths and the state holds one entry per
ticket of the spec.

### 2. Render and publish the first page

Run `render --state <state> --out <page>`, then publish the page as
**Republish** says. Tell the owner its address, or its path where nothing
publishes it.

Done when the owner has the page's address or path, and the page lists every
ticket of the spec.

### 3. Keep it current on every status change

Each event of the build changes one ticket's status. For each event, at once
and before the build's next action, run `set`, then `render`, then republish:
one change at a time, one page per change. Never batch status changes: the
owner watches the page while the build runs, so a change that waits for the
next one shows a stale build.

| Event | Status | Note | Run |
|---|---|---|---|
| An implementer started on the ticket | `running` | none | `set --state <state> <n>=running:` |
| A merger started merging it | `review` | none | `set --state <state> <n>=review:` |
| It is merged on the integration branch | `merged` | the merge commit | `set --state <state> <n>=merged:<sha>` |
| It waits on the owner | `owner` | what is needed | `set --state <state> '<n>=owner:<what is needed>'` |
| Work on it paused or stopped | `todo` | why, and what is left | `set --state <state> '<n>=todo:<why>'` |

The page shows `todo` as Ready once every blocker is merged and as Blocked
before, `running` as Building, `review` as Merging, `merged` as Merged and
`owner` as Needs you. Two tickets that change together still get one `set`,
one `render` and one republish each, in the order they happened.

Done when, after every event, the published page shows the ticket's new
status before the build moves on.

### 4. Sync when the tickets change

When the spec's tickets change on the issue tracker (a ticket is split, a
follow-up ticket is added, an issue is closed outside the build), run
`sync --spec <ref>`, then `render`, then republish. `sync` keeps every status,
note and milestone you set on open tickets.

Done when the page lists every ticket the spec has on the issue tracker.

### 5. Finish

When the last ticket is merged, or the build stops, render and republish one
last time and give the owner the page's address in your closing report.

Done when every ticket on the published page shows Merged, or your closing
report names each ticket that is not merged and why.

## Republish

The page path from `path` never changes for a spec, so every republish
updates the same address.

- In Claude Code, the republish is the Artifact tool with the same file path
  every time: the page path from `path`. The first publish creates the page;
  each later one with that file path updates it in place.
- Elsewhere, give the owner the page path once, in step 2; each `render`
  rewrites that file.

When a republish fails, tell the owner what failed and the page path, then
carry on with the build: the local page is still current, and the next
republish catches up.

## Worked example

Spec `acme/widgets#10` has tickets #11 to #14; #14 is the live check that
closes the milestone "Widgets live". In the spec's repository:

```sh
node <this folder>/build-tracker.mjs init --spec 10 --milestone "Widgets live=14"
node <this folder>/build-tracker.mjs path --spec 10
node <this folder>/build-tracker.mjs render --state <state> --out <page>
node <this folder>/build-tracker.mjs set --state <state> 11=running:
node <this folder>/build-tracker.mjs render --state <state> --out <page>
```

`path` prints the two paths this example writes as `<state>` and `<page>`.
After the first `render`, publish `<page>` (in Claude Code, the Artifact tool
with `<page>` as its file path) and give the owner the address. An
implementer then starts on #11: the `set` marks it `running`, the second
`render` rewrites `<page>`, and the Artifact tool with the same file path
updates the page the owner already has open.

## Commands

Run the runner with `node`:

```sh
node <this folder>/build-tracker.mjs init --spec <ref> [--milestone "<name>=<ticket>"] ...
node <this folder>/build-tracker.mjs sync --spec <ref>
node <this folder>/build-tracker.mjs path --spec <ref>
node <this folder>/build-tracker.mjs refresh --spec <ref> [--landed <ticket>] ...
node <this folder>/build-tracker.mjs render --state <state file> --out <page file>
node <this folder>/build-tracker.mjs render --tickets <ticket file> [--repo <owner/repo>] --out <page file>
node <this folder>/build-tracker.mjs set --state <state file> <ticket>=<status>[:note] ...
```

- `<ref>` is the spec's issue (`#12`, `owner/repo#12` or its URL), whose
  sub-issues are the tickets, or a local markdown spec file, whose tickets are
  the numbered files (`01-name.md`) in the `issues/` folder beside it.
- `init` seeds the state from the spec's tickets on the issue tracker: each
  ticket's title and blockers, `merged` for a closed issue and `todo` for an
  open one, and the milestones in the order given. It refuses when a state
  for the spec exists, and names `sync`; when that state file is unreadable,
  it says to delete it and run `init` again instead.
- `sync` re-reads the issue tracker: it adds new tickets, updates titles and
  blockers, marks newly closed tickets `merged`, and keeps every status, note
  and milestone set on open tickets.
- `path` prints the state file and the page path for the spec. Both sit
  outside the repository, under `$XDG_STATE_HOME/agent-kit/build-tracker`
  (`~/.local/state/...` when it is unset), one folder per repository and spec.
- `refresh` is what `/flow` runs on every call for a feature in
  implementation or finishing: `init` (with no milestones) when the spec has
  no state, otherwise `sync`; then each `--landed` ticket (`12`, `#12`, or a
  local ticket's `01`) turns `merged`, keeping its note; then it renders the
  page at the path `path` prints, and prints that path as `page`. A landed
  ticket that is not one of the spec's refuses the whole refresh.
- `render` writes the page for the state file. Run it in the spec's
  repository: a local spec's tickets then link to their files on the
  remote's file view on github.com. Elsewhere they show without a link, so
  the page never carries a local path.
- `render --tickets` writes one page for every ticket of a spec that keeps a
  ticket file (such as `docs/specs/gold-band/tickets.yaml`), grouped by
  branch in the file's order, with the graph across all branches. Its source
  is the ticket file and GitHub only: each ticket shows Merged when its issue
  is closed, and Ready or Blocked when it is open, by its blockers. It reads
  no state file and writes none, so it never changes a page made with
  `init`. `--repo` names the repository the issues are in; without it the
  working directory's `origin` is used. Give the page its own path, and
  publish it as **Republish** says:
  `node <this folder>/build-tracker.mjs render --tickets docs/specs/gold-band/tickets.yaml --out <page>`.
- `set` changes the named tickets' statuses. A note after the status replaces
  the ticket's note, an empty note removes it, and no note keeps it. The
  statuses are `todo`, `running`, `review`, `merged` and `owner`.

Each command prints one JSON object. An error prints `"status": "error"` with
a `code` and a `message`, exits 1 and leaves every file as it was. A missing
state file is such an error and names `init`: run step 1. An unreadable one
(`unreadable-state`) names its path: delete that file and run `init` again.
