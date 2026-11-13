---
name: critique-stats
description: Report how each critique critic client performs per stage, from the ledger, ending with routing suggestions for the owner to act on.
disable-model-invocation: true
---

# critique-stats

You **report**; the owner retunes. The runner reads the user's ledger (one
metadata line per critic run and per finding, written by every critique's
record step) and computes every figure. Your job is to run it, show its
report, and turn each suggestion into a decision the owner can make.

The runner is the `critique` launcher in the `critique` skill directory installed
beside this one (`../critique/critique` from the folder this file is in):

```sh
sh <critique dir>/critique stats --json '<object>'
```

A result with `"status": "refused"` carries a one-line `message`: give the
owner that line as it stands and stop. A result with `"status": "error"`
carries a `code` and a `message`: show both and stop.

## Arguments

`/critique-stats [here | <host/owner/repo>] [since YYYY-MM-DD] [until YYYY-MM-DD]`

Build the input object from what the owner typed:

| owner typed | input field |
|---|---|
| nothing | `{}`: every repository, every date |
| `here` | `"repo"`: this repository's root (`git rev-parse --show-toplevel`) |
| `github.com/owner/repo` | `"repository"`: that allowlist key as written |
| `since 2026-10-01` | `"since": "2026-10-01"` |
| `until 2026-10-31` | `"until": "2026-10-31"` |

Both dates are included. Pass a date the owner gave in other words
("last week") as the `YYYY-MM-DD` it means, and say which dates you used.

## Steps

### 1. Run stats

Run the `stats` command with that input.

Done when it returned `"status": "reported"`, or you stopped on a refusal or
an error.

### 2. Show the report

Print the result's `report` field as it stands: one table per stage, the
calibration table, the removal candidates, then the suggestions. Add nothing to the figures.

Done when the owner has the report.

### 3. Turn each finding into a decision

For each line under **Suggestions** and each **removal candidate**, give the
owner one short paragraph: what the line means for the stage, and the change
it points at. Routing is agent-kit's `skills/workstation/critique/routing.json` (each
stage's `base` roster and the `fallbacks` map), with this machine's override
`~/.config/agent-kit/critique-routing.json` merged over it. A change to
either is the owner's edit; offer to explain the change, and leave both
files as they are.

With no suggestion and no removal candidate, say so in one line, and name the
rows marked `(too few)`: those cells need more critiques before they can
drive routing.

Done when every suggestion and removal candidate has its paragraph, or the
one line says there are none.

## Reading the figures

Answer the owner's questions from these, and from the matching fields of the
result's `cells`:

- **Cell**: one critic client on one stage with one prompt. A changed role,
  lens, preamble, output contract or schema gives a new prompt hash, so a
  revised prompt is judged on its own findings. Suggestions use each client's
  latest prompt only.
- **Precision**: accepted over accepted plus rejected. Hints and needs-human
  findings are left out. `adjudicated` is that accepted plus rejected count.
- **Unique**: accepted findings no other critic client raised.
- **Fabricated**: the share of the cell's findings rejected because a quote is
  in no packet file.
- **Calibration**: whether a critic is right over time. Only a cell with
  `min_adjudicated_findings` adjudicated findings has a row, and a cell with
  fewer has `calibration: null`. `earlier` and `recent` are precision over the
  older and the newer half of the cell's accepted and rejected findings, ordered by when
  their run was recorded (the odd finding goes to the newer half), with the
  count in brackets; `drift` is recent minus earlier, so a falling number
  means the critic is getting worse. The `critical`, `major` and `minor`
  columns are precision per recorded severity: a critic whose criticals are
  accepted less often than its minors is not calibrated. A dash means nothing
  was adjudicated in that slice. Calibration never drives a suggestion; it is
  there for the owner to read.
- **Cost per accepted**: the cell's known cost over its accepted findings. A
  dash means no accepted finding or no priced run (Antigravity reports no
  cost).
- **Failed** and **timed out**: shares of the cell's runs. A run that
  violated read-only counts as failed. Vibe reports a price-limit stop the
  same way as a deadline, so both count as timed out.
- **Wasted**: the known cost of the cell's runs that a price or time limit
  stopped (Vibe's `price_limit` or deadline, or a run the outer timer
  interrupted) with no findings, and in brackets how many there were. A dash
  means none. When any run was stopped, a line under the heading sums it
  over every row.
- **Thresholds**: `min_adjudicated_findings` (a cell needs this many before
  it gets a suggestion) and `removal_features` (a stage with no unique
  accepted catch across this many of its latest features is a removal
  candidate) live in routing's `stats` section, which the override can
  change. A feature is one spec. A review stage belongs to no feature, so it
  is judged by that many of its latest runs instead.
