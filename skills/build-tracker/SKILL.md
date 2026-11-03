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
node <this folder>/build-tracker.mjs render --state <state file> --out <page file>
node <this folder>/build-tracker.mjs set --state <state file> <ticket>=<status>[:note] ...
```

- `render` writes the page for the state file.
- `set` changes the named tickets' statuses. A note after the status replaces
  the ticket's note, an empty note removes it, and no note keeps it. The
  statuses are `todo`, `running`, `review`, `merged` and `owner`.

Each command prints one JSON object. An error prints `"status": "error"` with
a `code` and a `message`, exits 1 and leaves every file as it was.
