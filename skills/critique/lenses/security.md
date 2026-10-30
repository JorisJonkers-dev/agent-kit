# Lens: security

Check what the artifact exposes and to whom.

- Follow every piece of data that leaves the machine or the repository. Ask
  who receives it and whether a decision allows that.
- Look for untrusted text (file contents, tool output, a model's reply) that
  could end up treated as an instruction or a command.
- Look for secrets, credentials or accounts that could be used, logged or
  sent where they should not be.
- Look for checks that can be bypassed: a deny rule with an exception path, a
  default that is permissive, a check done after the action it guards.
