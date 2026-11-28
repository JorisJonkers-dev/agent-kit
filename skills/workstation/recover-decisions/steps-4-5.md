### 4. Ask the owner

Rank the `needs-human` candidates by severity, critical first, ties in the
order of the findings, the way the runner ranks them. Ask the first ten, one at
a time, and wait for each answer before the next. The rest are **deferred**:
you ask nothing about them, the report lists them, and they stay in the run
directory until a later run asks. Never ask an eleventh question.

Each question gives what the code shows (the sites), what the history says or
that it says nothing, and the candidate's draft, then asks four things: is
this your decision; why; what did you reject; and what does it cost. Word it
so that "yes" confirms the draft.

- **The owner confirms it and answers.** Draft the record from the candidate:
  the facts marked observed stay, and every inferred clause is replaced by the
  owner's own words. Write it in the log's house format with **no status
  line**, here for a log of records like the existing ones:

  ```markdown
  # <the decision, as a sentence>

  <Context and decision: In the context of <situation>, we decided for
  <option>, because <the owner's reason>.>

  We rejected <the alternative the owner named>: <why it lost, in the owner's
  words>.

  ## Consequences

  - <the cost the owner named>
  ```

  Use the numbering and the folder (`docs/adr/`) of the records already
  there. The record always holds a rejected alternative with its reason and a
  cost: when the owner gave none, ask once more, and if they still cannot,
  write nothing and leave the candidate open. Never write a reason the owner
  did not give, and never copy the critic's inferred rationale. Show the owner
  the draft and write it only once they approve it. Mark the verdict
  `"answered": true` with `"reason_code": "answered-by-owner"`.
- **"It was an accident."** Not a decision: write nothing to the decision log.
  Add a note to `<run_dir>/refactoring-notes.md` (the candidate, its sites and
  the owner's words) for a refactoring plan. Mark the verdict
  `"answered": true` with `"reason_code": "accident"`.
- **"skip".** The owner leaves it open. Write nothing; give the verdict
  `"reason_code": "skipped-by-owner"`.

Until the owner accepts a candidate, nothing of it is written under the
decision log: candidates stay in the run directory and in the report.

Done when each of the ten (or fewer) has an answer or a skip.

### 5. Record and return

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json`, one
entry per verdict from steps 3 and 4 (`finding_id`, `verdict`, and `reason`,
`reason_code` and `answered` where they apply), and run record with
`--input <run_dir>/verdicts.json`. It names the questions asked
(`questions`), the candidates deferred (`deferred`) and writes the report,
which lists the deferred ones; it refuses an answer for a deferred candidate.

Return record's `summary` as the runner wrote it, with one line after it per
ADR you wrote (its path), and one for each accident noted. Then suggest
`/critique-adrs` on the ADRs you wrote, naming their paths, as the owner's
next step, and stop: run nothing more, and spend nothing more. A run that
wrote no ADR suggests nothing.
