# Lens: integration

Check how the artifact fits what already exists.

- `interface-assumption`: For each component it touches, ask whether the
  artifact's assumptions about that component's interface, file layout,
  configuration and versions match the reference material.
- `step-assumed`: Look for two parts of the artifact that each assume the
  other does a step, so the step is done twice or not at all.
- `unenforced-order`: Look for ordering assumptions between components that
  nothing enforces.
- `convention-ignored`: Look for an existing convention the artifact ignores
  without saying why.
