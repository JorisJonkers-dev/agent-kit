### 6. Refute

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json` with
the verdicts of step 5, leaving out `regression_test` and `fixed`, which come
in step 7, and run refute with `--input <run_dir>/verdicts.json`. It sends
each accepted critical finding to one critic of a family that neither raised
it nor is yours, and returns when the slowest ends; give it as long as
fan-out. With no accepted critical finding it launches nothing. Each of its
`results` names a `finding_id` and an `outcome`:

- `refute`: the refuter showed, with a failing test or a trace the runner
  checked, that the finding does not hold. Change its verdict to `rejected`
  with `reason_code: "refuted"` and the refuter's `reply.claim` as `reason`.
  It is not fixed, and its test stays out of the change.
- `downgrade`: the bug holds but is not critical. Keep it `accepted` and fix
  it in step 7; record lowers its severity to the result's `severity`, so
  it no longer holds the commit.
- `no-objection` or `stands`: your verdict stands.
- `skipped`: no eligible critic could run (`reason_code` and `detail` say
  why), and your verdict stands. A skipped refute is reported, not retried:
  the runner refuses a second refute of one run, and step 9 names the skip.

### 7. Fix inside the TDD loop

Each accepted finding is fixed now, as part of the ticket, before its
commit:

1. For a `test-fails` finding, copy its test from the scratch worktree into
   the owner's working tree at the same repository path and stage it: it is
   the ticket's regression test from now on, and its path is the verdict's
   `regression_test`. For a `trace-walked` finding, write the failing test
   yourself first.
2. The test is red. Make the smallest change that turns it green, run the
   repository's tests, and stage the fix.
3. Give the verdict `"fixed": true` once its test passes with the fix
   staged.

A finding the owner chooses to leave for later keeps no `fixed`. Then remove
the scratch worktree: `git worktree remove --force "<run_dir>/worktree"`.

Done when every accepted finding is fixed and staged or the owner chose to
leave it, and the scratch worktree is gone.

### 8. Record

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json`, one
entry per verdict from step 5 as steps 6 and 7 left it: `finding_id`,
`verdict`, `reason`, `reason_code`, and for an accepted finding
`regression_test` (with `test-fails`) and `fixed`. Run record with
`--input <run_dir>/verdicts.json`; it applies the refute round.

Record writes `<run_dir>/review.json` and a line of the critique directory's
`ticket-reviews.jsonl`: whether the review passed, failed or was degraded
(no critic reported), and a hash of the staged
change as it stands now, fixes and regression tests included. A commit gate
matches the staged change against that hash, so record last: anything staged
after it needs another review.

### 9. Return

Return triage's `summary` line, one line per `skipped` result of the refute
round, `Refute skipped: <finding_id> (<reason_code>)`, then record's
`summary` as the runner wrote it. When the summary ends with "do not
commit", the commit is still blocked, and you say so plainly:

- **An accepted critical finding is unfixed**: the commit waits until it is
  fixed, staged and reviewed again with `/review-implementation`.
- **No critic reported, so the review is degraded**: nobody reviewed the
  change, and the commit gate stays shut. Name each critic and why it did
  not report (`ticket_review.critic_failures` in record's result). The
  commit waits for another `/review-implementation` run in which a critic reports;
  only the owner can turn the gate off with the kill switch.
