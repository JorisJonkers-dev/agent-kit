---
name: build-tracker
description: The build tracker for a spec that is being implemented. A state file holds each ticket's status, and the runner renders it into one page with a summary strip, a dependency graph and a list per milestone.
---

# build-tracker

The build tracker shows the owner where every ticket of a spec stands while
it is built. Its state file is the single source; the page is rendered from
it and never edited by hand.

The runner is `build-tracker.mjs` in the folder this file is in. Run it with
`node`:

```sh
node <this folder>/build-tracker.mjs init --spec <spec> [--milestone "<name>=<ticket>" ...]
node <this folder>/build-tracker.mjs sync --spec <spec>
node <this folder>/build-tracker.mjs path --spec <spec>
node <this folder>/build-tracker.mjs render --state <state file> --out <page file>
node <this folder>/build-tracker.mjs set --state <state file> <ticket>=<status>[:note] ...
```

- `<spec>` is the spec's issue (`#12`, `owner/repo#12` or its URL), whose
  sub-issues are the tickets, or a local markdown spec file, whose tickets are
  the numbered files (`01-name.md`) in the `issues/` folder beside it.
- `init` writes a new state from the spec's tickets: each open ticket `todo`,
  each closed one `merged`, with its blockers. Each `--milestone`, in order,
  names a milestone and the ticket that closes it. It refuses a spec that
  already has a state.
- `sync` brings the state up to date with the issue tracker: it adds new
  tickets, updates titles and blockers, marks newly closed tickets `merged`,
  and keeps every status, note and milestone set on open tickets.
- `path` prints where the spec's state file and page live, so they are never
  guessed. Both sit outside the repository, under
  `$XDG_STATE_HOME/agent-kit/build-tracker` (`~/.local/state/...` when it is
  unset), one folder per repository and spec.
- `render` writes the page for the state file.
- `set` changes the named tickets' statuses. A note after the status replaces
  the ticket's note, an empty note removes it, and no note keeps it. The
  statuses are `todo`, `running`, `review`, `merged` and `owner`.

Each command prints one JSON object. An error prints `"status": "error"` with
a `code` and a `message`, exits 1 and leaves every file as it was.
