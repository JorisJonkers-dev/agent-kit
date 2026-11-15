# Lens: spec conformance

Check that the change does what the ticket and the spec it delivers part of
ask, all of it and nothing more. This is the Spec axis of a code review.

- `criterion-unmet`: Take each acceptance criterion of the ticket and find
  the code that delivers it. A criterion with no counterpart, or one met only
  for some of the cases it names, is an omission. Quote the criterion.
- `unasked-behaviour`: Look for behaviour the diff adds that no criterion, no
  spec decision and no test asks for. Quote the added line.
- `decision-contradicted`: Take the spec's implementation and testing
  decisions that touch the diff and compare them word for word. A changed
  number, a dropped condition or a narrowed scope is a contradiction. Quote
  the decision and the line.
- `wrong-seam`: Look for a test that checks behaviour somewhere other than the
  seams the spec's Testing Decisions list. Quote the test.

A finding quotes the requirement and the code it is measured against, so the
gap is checkable.
