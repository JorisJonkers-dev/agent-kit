# Lens: aggregates

Check that each aggregate protects its own invariants and that a change to it
goes through its root in one transaction. An invariant is a rule that must
hold after every change, such as a total that may not exceed a limit.

- `skipped-invariant`: An invariant enforced by one caller and not by another
  caller of the same aggregate. Give the input that breaks it, and the caller
  that skips the check.
- `two-aggregates-one-transaction`: One transaction that writes two
  aggregates, with no decision that allows it. Quote the transaction and name
  the two aggregates.
- `root-bypassed`: A repository, setter or query that changes or reads an
  aggregate's inner entity without going through its root. Quote the line.
- `eventually-unmechanised`: A rule described as "eventually" (consistent,
  reconciled, cleaned up) with no mechanism that makes it so: no job, event,
  retry or compensating step is named. Quote the rule.
- `unit-mixed`: A value with a unit (money, duration, byte count, percentage)
  held in a bare number and passed where another unit is expected. Cite the
  call site and the parameter it reaches.
