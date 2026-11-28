### 5. Refute

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json` with
the verdicts of step 4, as step 7 describes them, and run refute with
`--input <run_dir>/verdicts.json`. It sends each accepted critical gap to one
critic of a family that neither raised it nor is yours, and returns when the
slowest ends; give it as long as fan-out. With no accepted critical gap it
launches nothing. Each of its `results` names a `finding_id` and an
`outcome`:

- `refute`: the refuter showed, with evidence the runner checked, that the
  gap does not hold. Change its verdict to `rejected` with
  `reason_code: "refuted"` and the refuter's `reply.claim` as `reason`. It
  is not asked.
- `downgrade`: the gap holds but is not critical. Keep it `accepted`;
  record lowers its severity to the result's `severity`, and step 6 ranks
  it at that severity.
- `no-objection` or `stands`: your verdict stands.
- `skipped`: no eligible critic could run (`reason_code` and `detail` say
  why), and your verdict stands. A skipped refute is reported, not retried:
  the runner refuses a second refute of one run, and step 7 names the skip.

### 6. Ask the owner

Rank the accepted gaps by severity, critical first, and keep the first seven.
Ask them one at a time, in the grilling style: the question, the scenario
that forces it, the options, and your recommended answer, worded so "yes"
accepts it. Wait for each answer before the next question.

- **Answered**: write the answer the way `/grill-with-docs` does, following
  the `domain-modeling` skill: a resolved term goes into the glossary the
  packet was built from, and a decision that is hard to reverse, surprising
  without context and the result of a real trade-off gets an ADR. Mark the
  verdict `"answered": true`.
- **"skip"**: the owner leaves it open. Write nothing; give the verdict
  `"reason_code": "skipped-by-owner"`.
- **Past the seventh**: not asked; give each `"reason_code": "not-asked"`.

Done when each of the seven has an answer or a skip.

### 7. Record and return

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json`, one
entry per verdict from step 4 as steps 5 and 6 left it (`finding_id`,
`verdict`, and `reason`, `reason_code` and `answered` where they apply), and
run record with `--input <run_dir>/verdicts.json`; it applies the refute
round.

Return record's `summary` as the runner wrote it, with one line above it per
`skipped` result of the refute round,
`Refute skipped: <finding_id> (<reason_code>)`, and one line after it per
decision doc you changed. The stage has no gate: the owner may go on to
`/to-spec` whatever was skipped.
