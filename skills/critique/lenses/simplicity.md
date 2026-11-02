# Lens: simplicity

Check whether the artifact asks for more than its problem needs.

- `unneeded-mechanism`: Look for mechanisms that no user story requires.
- `duplicate-mechanism`: Look for two mechanisms that solve the same problem.
- `single-use-option`: Look for configuration, options or extension points
  with only one value or one user.
- `droppable-step`: Look for steps that could be dropped without losing a
  stated outcome.

A simplicity finding must name the requirement that the simpler version
still meets.
