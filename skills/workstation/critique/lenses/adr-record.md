# Lens: ADR record

Check one decision record at a time, for what its text says, never for its
headings: a record in any format passes a check when the content is there,
wherever it sits. Each check fixes the severity of its findings, given in
brackets. A check marked "Needs `tree/`" runs only when the packet holds
code under `tree/`.

- `no-decision-sentence` (critical): Look for a record with no sentence
  that states what was decided. A record that only discusses the options,
  or the problem, leaves a later reader unable to say what holds.
- `no-rejected-alternative` (major): Look for a record that names no
  alternative it turned down, or names one with no reason it lost.
- `no-cost` (major): Look for a record that names no cost, risk or
  negative consequence of the choice: one that lists only upsides.
- `solution-in-context` (minor): Look for a context that already states
  the chosen solution, or argues for it, instead of the forces that called
  for a decision.
- `no-because` (major): Look for a choice with no "because" tied to a
  force the record states. Quote the choice and the forces, and show that
  no force is given as the reason.
- `undated-claim` (minor): Look for a claim about the outside world (a
  tool's behaviour, a price, a limit, a benchmark, what others do) with no
  date and no source, so nobody can tell when it went stale.
- `no-confirmation` (minor): Needs `tree/`. Look for a decision that a
  test, a lint rule or a script could check, where the record names no such
  check and `tree/` holds none.
- `two-choices` (major): Look for two choices in one record that could be
  reversed separately, so neither can be superseded on its own. Quote both.
- `two-readings` (major): Look for wording that two readers would carry
  out differently. Count it only when you give both readings and the
  different code or behaviour each one leads to; wording a careful reader
  gets right is not a finding.

The bracketed severity is the finding's: write it, and the runner sets it
whatever you write.
