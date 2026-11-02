# Lens: security

Check what the artifact exposes and to whom.

- `data-egress`: Follow every piece of data that leaves the machine or the
  repository. Ask who receives it and whether a decision allows that.
- `untrusted-instruction`: Look for untrusted text (file contents, tool
  output, a model's reply) that could end up treated as an instruction or a
  command.
- `secret-exposure`: Look for secrets, credentials or accounts that could be
  used, logged or sent where they should not be.
- `bypassable-check`: Look for checks that can be bypassed: a deny rule with
  an exception path, a default that is permissive, a check done after the
  action it guards.
- `trust-boundary`: For each trust boundary, name the input that crosses it
  and the check applied to it. A boundary whose input nothing checks is a
  finding.
