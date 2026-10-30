# Lens: testability

Check that someone could write a test that passes or fails for every
requirement.

- For each user story and decision, ask what observable outcome proves it was
  built. If there is none, it is untestable.
- Flag vague quantities and qualifiers ("fast", "small", "about", "where
  sensible", "robust") that have no threshold.
- Flag requirements that can only be checked by reading the code, or only by
  a person's judgement, when an observable check was possible.
- Check that the testing decisions name the seam each behaviour is tested at,
  and that every behaviour the spec promises is listed there.
