### 4. Refute

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json` with
the verdicts of step 3, leaving out `follow_up`, which comes in step 5, and
run refute with `--input <run_dir>/verdicts.json`. It sends each accepted
critical finding to one critic of a family that neither raised it nor is
yours, and returns when the slowest ends; give it as long as fan-out. With
no accepted critical finding it launches nothing. Each of its `results`
names a `finding_id` and an `outcome`:

- `refute`: the refuter showed, with a failing test or a trace the runner
  checked, that the finding does not hold. Change its verdict to `rejected`
  with `reason_code: "refuted"` and the refuter's `reply.claim` as `reason`,
  without `follow_up` or `trace_tickets`. It gets no follow-up ticket.
- `downgrade`: the defect holds but is not critical. Keep it `accepted` and
  file its follow-up in step 5 at the result's `severity`; record lowers its
  severity to that, so it does not hold the gate.
- `no-objection` or `stands`: your verdict stands.
- `skipped`: no eligible critic could run (`reason_code` and `detail` say
  why), and your verdict stands. A skipped refute is reported, not retried:
  the runner refuses a second refute of one run, and step 7 names the skip.

### 5. File a follow-up ticket per accepted finding

Each accepted finding becomes one ticket of the spec, on the spec's tracker,
blocked by nothing, fixed later through `/implement`. When an earlier
feature review already filed an open ticket for the same defect, name that
ticket instead of filing a second one.

The body holds: the finding's claim, severity and location; the failing test
in full, with its path and the command that runs it, or the trace step by
step; and the run id and report path.

- **GitHub**: read the labels the spec's other tickets carry
  (`gh issue view <n> --repo <host/owner/repo> --json labels`) and give the
  follow-up the same ones. Create it with
  `gh issue create --repo <host/owner/repo> --title ... --body-file <file> --label ...`,
  then make it a sub-issue of the spec:
  `gh issue edit <spec> --repo <host/owner/repo> --add-sub-issue <n>` (gh
  2.94+, else `gh api --method POST repos/<owner>/<repo>/issues/<spec>/sub_issues -F sub_issue_id=<database id>`).
- **Local markdown**: write `issues/<NN>-<slug>.md` beside the spec, with
  the next free number, `# <NN>: <title>`, `**Status:** ready-for-agent`,
  and the sections the other tickets there use.

Keep each finding's follow-up id: `#301`, or `07` for a local ticket.

### 6. Record

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json`, one
entry per verdict from step 3 as steps 4 and 5 left it: `finding_id`,
`verdict`, `reason`, `reason_code`, and for an accepted finding `follow_up`
(and `trace_tickets` for a trace). Run record with
`--input <run_dir>/verdicts.json`; it applies the refute round.

Record reads the spec's tickets from the tracker and refuses a `follow_up`
that is not one of them. It computes the gate: `GATE: FAIL (n critical)`
while any accepted critical finding of this run or an earlier one has an
open follow-up ticket, `GATE: PASS` otherwise. It writes the gate and the
head it holds for to `<run_dir>/gate.json`, which the PR gate reads.

### 7. Return

Return record's `summary` as the runner wrote it; its first line is the gate.
Under it, list the follow-up tickets you filed, then one line per `skipped`
result of the refute round, `Refute skipped: <finding_id> (<reason_code>)`.
While the gate is FAIL, tell the owner the PR waits: fix each critical
follow-up through `/implement`, close it once its fix is on the feature
branch (`gh issue close`, or `**Status:** done` in a local ticket), then run
`/review-pr` again on the new head. Each run's gate belongs to the head it reviewed.
