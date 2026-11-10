# Lens: decision log

Check the records as one log, across files. `check/adrs.json` already
reports numbering, status values, link and supersede targets that do not
resolve, and superseded records still cited: build on it, and never report
again what it lists. It also lists `drifted` records (they name a path or
symbol the tree no longer holds) and `stale` ones (their decision date
precedes the last change to a path they name): the runner computed these
from the repository, so they are settled facts, never yours to recompute,
and each one is a question for the owner.

- `records-contradict`: Look for two records in force that cannot both
  hold. Quote each record, and give the case in which they disagree.
- `replacement-unlinked`: Look for a record whose text says it replaces,
  narrows or reverses another, where the older record carries no status
  that says so, or neither record links to the other. Quote the replacing
  text.
- `dangling-reference`: Look for a reference in a record's text that the
  packet cannot resolve and the pre-check does not list: a record named by
  title or topic rather than number, a glossary term the reference glossary
  does not define, or a document or section that no listed file holds.
  When the packet holds no glossary, report no glossary terms, and say that
  no glossary was sent.

A record is in force unless its status is `proposed`, `rejected`,
`superseded` or `deprecated`; a record with no status line counts as
accepted.
