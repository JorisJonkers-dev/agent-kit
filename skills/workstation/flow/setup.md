## /flow setup

Takes this machine, then this repository, from nothing to ready for a
critique. You are a **guide**: one item at a time, each with its reason, its
check and its fix, and the owner decides at every item. The runner keeps the
results per machine, so a later `/flow setup` re-runs only what changed or
failed. Every check runs through the runner's `setup` command:

```sh
sh <critique dir>/critique setup --json '<object>'
```

| Input | What it does |
|---|---|
| `{"action": "list", "repo": "<repository root>"}` | The checklist with each item's state, and the repository's opt-in. Leave `repo` out outside a repository. Launches no critic. |
| `{"action": "check", "check": "<item id>"}` | Runs one item and records its result. Add `"confirmed": true` once the owner has agreed to the item's cost. |
| `{"action": "quota", "daily_cap": <n>, "reserve_threshold": <n>}` | Stores the Antigravity plan's limit in this machine's routing override and checks the quota again. |
| `{"action": "kill-switch", "disabled": true \| false}` | Turns every critique hook off (`true`) or back on (`false`) in this machine's critique configuration and checks the kill switch again. |

An error result carries a `code` and a `message`: show both and stop.

### 1. List

Run `list`, with `repo` when `git rev-parse --show-toplevel` names one. Keep
`items` and `repository`.

Open with one line: how many items are `current` (they passed, and nothing
they depend on changed since) and how many are due.

If the result carries `notes`, say each one as it stands before the first
item: they name a setting this machine's routing override still holds that
routing no longer reads, such as the removed `budget.vibe_flow_cap_usd`, and
the owner should delete it.

Done when `list` returned `"status": "listed"`.

### 2. Walk the machine checklist

Take the items in the runner's order, skipping the `current` ones. For each:

1. **Say why.** Give its `title`, then its `why` as it stands, in one
   sentence.
2. **Check.** Run `check` with the item's `id`.
   - `not-available`: give the item's `detail`. The Antigravity read-only
     proof answers this way until agent-kit#178 lands, and a Pi client's
     always does: Pi has no sandbox to prove, so every Pi run is checked
     against its packet instead.
   - Claude Code, Codex, Hermes and OpenCode each have a read-only item
     with no critic behind it. It is recorded as `unproven` with its reason,
     at no cost and without launching anything. Report it as unproven, never
     as a pass: the gateway runs that client's read-only work in a throwaway
     worktree.
   - A Vibe client's read-only proof first checks, for free, that the
     runner's search server cannot reach past the packet. Then it asks for
     confirmation, like any `vibe` item, and runs one trivial prompt. That
     prompt shows `run_typescript` refusing shell commands, file writes and
     process starts while a search still completes.
   - `needs-confirmation`: the item costs money or quota. Its `cost` says
     which: `vibe` is one trivial prompt on the personal Mistral account
     (capped at $0.05), `agy` is one of today's Antigravity invocations,
     `pi` is one trivial prompt through Pi on a LiteLLM route's budget. The
     first time each cost comes up in this session, ask the owner before
     spending it; on yes, run `check` again with `"confirmed": true`, and
     pass `"confirmed": true` for later items of that cost without asking
     again. A no is a skip.
   - `checked`: the result is the item's `last`.
3. **Report.** One line: `pass`, `fail` or `unproven`, with `last.detail`.
4. **Offer the fix** on a fail: the item's `fix`. For `agy-quota`, ask for
   the plan's daily request limit and the reserve to keep for the spec and
   feature reviews (propose half the limit), then run `quota` with both.
   For `hooks-installed`, a Codex hook that is not trusted yet (or changed
   since it was trusted) is fixed in Codex itself: tell the owner to open
   `/hooks` in a Codex session and trust the critique hooks it names, then
   check again; never bypass Codex's hook trust.
   For `kill-switch`, the item passes whether the hooks are on or off: its
   detail says which. Offer to switch them the other way only when the
   owner asks, then run `kill-switch` with `"disabled": true` or `false`.
5. **Wait.** The owner says the fix is done (run `check` again for the
   item), moves on, or says skip. Skip works at every item and records
   nothing.

Done when every item that was not `current` has passed, failed or been
skipped, with the owner's word at each.

### 3. Offer the repository's opt-in

By `repository.status`:

- `refused`: give its `message`; the repository sends nothing, so there is
  nothing to offer.
- `opted-in`: one line with its `families`, and whether `code` and
  `history` are on.
- `not-opted-in`: three separate offers, each with its own yes, no or skip:
  1. **Families**: which model families this repository may send documents
     to. Propose `routed_families`; the owner may name fewer. On yes, run
     the runner's `allow` command (`critique allow --json '<object>'`)
     with `{"repo": ..., "families": [...]}`.
  2. **Code**, only once families are allowed: whether the code stages
     (ticket review and feature review) may send this repository's code to
     those families. On yes, run `allow` again with the same families and
     `"code": true`.
  3. **History**, only once families are allowed: the history permission,
     off by default, lets a stage send this repository's commit messages
     and issue text to those families. Without it, a stage leaves history
     out. On yes, run `allow` again with the same families and
     `"history": true`.

A repository GitHub says is public needs none of these: the allowlist does
not apply to it, and every family, the public-only LiteLLM routes among them,
may read its code and history. There is nothing to offer for that.

Without `repository` (outside a repository), skip this step.

Done when the owner has answered each offer, or the repository was refused
or already opted in.

### 4. Close

One line: passed, failed, skipped and not available, counted over the
whole checklist (`current` items count as passed). Then offer `/flow`.

