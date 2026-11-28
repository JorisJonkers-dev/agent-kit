### 7. Check the rewrite

`adr-record` runs again on every rewritten ADR: re-check the rewritten
record against the nine checks of `lenses/adr-record.md`, beside the runner,
and label the decision sentences its text now states. A defect the rewrite
introduced is a bug in your own edit: fix it with a further edit of step 6,
or undo the edit, before continuing. The labelled sentences are what
check-rewrite reads.

Then run check-rewrite; it reads the working tree, writes nothing, and
sends nothing anywhere:

```sh
sh <critique dir>/critique check-rewrite --input <file.json>
```

Its input is `repo` (the repository's root), `original_sentences` (the
decision sentences you quoted from the frozen packet), one entry per
rewritten file in `rewritten` (`path`, and the `decision_sentences` you
labelled in it), and for a split `split` (`original`, and the `files` the
split created). It confirms each original sentence appears byte-identical
in exactly one rewritten file, that no rewritten file labels a decision
sentence that is not an original one, and that a split marks the original
superseded by the split files. Quote `original_sentences` only from the
records you edited, list in `rewritten` every file that now holds one of
them (the records you edited and the files a split created), and run it
only when you edited a record. It checks the sentences you label, so it
catches a changed decision only as far as your labels are complete; a
duplicated sentence counts every occurrence, in one file or across files.

- `"status": "passed"`: the rewrite kept every decision; go on to the cold
  read below.
- `"status": "failed"` and exit 3: `failures` names the check that failed —
  `sentence-preserved` (`missing` or `duplicated`; restore the record the sentence was quoted from, and for `duplicated` also the files in `found_in`),
  `no-new-decision-sentence`, or `split-superseded` (with `unmarked`). For
  each failure, restore the original text of the record it names from the
  frozen packet (`<packet_dir>/artifact/<path>`; for a `missing` sentence,
  the record you quoted it from; a split failure also deletes the records
  the split created), and ask the owner the new-decision question: is this
  change a new decision or a mistake? A mistake keeps the restored record
  as it was. A new decision is a change outside the allowed list: it
  becomes a draft superseding or amending ADR, written from the owner's
  answer as step 8 describes. When the owner defers, the record stays
  restored.

When check-rewrite passes, the cold read is the independent check on the
rewrite: a client that never saw this review reads each rewritten ADR on its
own and says what it decided, what it rejected and why, and when to revisit.

```sh
sh <critique dir>/critique cold-read --input <file.json>
```

Its input is `run_dir` (this run's directory) and `files` (the rewritten ADRs,
as repository paths). For each file the runner builds a one-file packet under
`<run_dir>/cold-read/<n>/artifact/<basename>` holding only that rewritten ADR:
no glossary, no other records, no tree, no spec. The `cold-reader` role runs
on it through the Mistral Small client routing's `cold_read` section names,
capped at 0.03 USD per check, and each launch is logged like any critic's, so
its spend counts in the run's flow spend. The result is
`<run_dir>/cold-read.json`, and the command prints it: per file `path`,
`status` (`answered` or `unavailable`), `answers` (`decided`, `rejected`,
`revisit`), `missing` (the answers left empty) and, when it could not run, a
`reason`.

The command does not judge the answers. For each file, compare each answer
with the ADR's own decision sentence, its rejected alternatives and its
revisit condition. An answer that is wrong, and a missing one (listed in
`missing`), is an omission: restore that ADR's original text from the frozen
packet (`<packet_dir>/artifact/<path>`; a split also deletes the records the
split created) and list the ADR in the report `record` writes in step 9. A file reported
`unavailable` is reported, never treated as a pass: say which file and why,
and leave the record as it is. A run is cold-read once, so what step 8
changes afterwards (an owner's answer applied as an edit, a draft ADR) is
not cold-read: run check-rewrite again on those edits, and say in the
report that they were not cold-read. Then go on to step 8.

### 8. Ask the owner

Ask each `needs-human` question one at a time, critical ones first, and wait
for the answer before the next. When the owner answers:

- **The answer changes a decision** (a new rationale, alternative, cost or
  reading, or which of two records stands): draft a new record that
  supersedes the old one, or an amending record when the answer only
  narrows or extends it, in the log's own format, numbered after the last
  record, with the answer in its text. Show the owner the draft and write
  it only once they approve it; then mark the old record superseded or
  amended, linking the new one.
- **The answer only supplies a missing fact** the record already implies:
  add it as an edit of step 6 only when the owner says so; otherwise draft
  an amending record as above.
- **A `no-confirmation` follow-up**: ask whether to file an issue for the
  check; file it on the tracker only when the owner agrees.

Add each change to the change list and mark the verdict `"answered": true`.
When the owner defers, leave it unanswered.

### 9. Record

Write `{"run_dir": ..., "verdicts": [...]}` to `<run_dir>/verdicts.json`, one
entry per verdict from step 4 as steps 5 to 8 left it: `finding_id`,
`verdict`, and `reason`, `reason_code`, `answered`, `severity` and
`severity_reason` where they apply. Run record with
`--input <run_dir>/verdicts.json`; it applies the refute round. Keep its
`summary`, `report_path` and `second_round_due`.

### 10. Second round

When record says `second_round_due` (more than three critical findings were
still accepted after the refute round), the revised records get fresh
critics: go back to step 1 with the same arguments plus
`"previous_run_dir": <this run_dir>`, and do steps 1 to 8 again. The runner
allows two rounds at most; record never asks for a third.

### 11. Return

Give the owner the change list of every round: the edits in the working
tree are not committed, so they review and commit them. Then return the last
round's `summary` as the runner wrote it, with one line above it per
`skipped` result of each round's refute,
`Refute skipped: <finding_id> (<reason_code>)`. A documents-only run's
summary names the checks it skipped, and its report says why; keep that
line. With two rounds, put one line before the summary naming the first
round's run id, verdict counts and report path.
