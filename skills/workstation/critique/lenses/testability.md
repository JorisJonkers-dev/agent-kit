# Lens: testability

Check that someone could write a test that passes or fails for every
requirement.

- `no-observable-outcome`: For each user story and decision, ask what
  observable outcome proves it was built. If there is none, it is
  untestable.
- `vague-quantity`: Flag vague quantities and qualifiers ("fast", "small",
  "about", "where sensible", "robust") that have no threshold.
- `judgement-only`: Flag requirements that can only be checked by reading the
  code, or only by a person's judgement, when an observable check was
  possible.
- `seam-unnamed`: Check that the testing decisions name the seam each
  behaviour is tested at, and that every behaviour the spec promises is
  listed there.
- `rule-at-port`: Check that domain rules are tested at a port, with no
  database, network or framework in the test. A rule that can only be
  tested through one of them is a finding.
