# Lens: failure modes

Check what happens when things go wrong.

- For each external dependency (a process, a network call, a file, a
  service), ask what the artifact does when it fails, hangs, returns nothing
  or returns something malformed.
- Look for partial success: half of a batch done, a write that happens before
  a check that then fails, a retry that repeats a side effect.
- Look for empty, missing, duplicated and oversized input.
- Look for concurrent use: two runs at once, an edit during a run.
- A failure mode counts only if you can describe the scenario concretely.
