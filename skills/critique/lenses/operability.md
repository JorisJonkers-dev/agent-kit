# Lens: operability

Check what it is like to run, observe and fix this once it is built.

- Ask how the owner finds out that it failed, and from what output.
- Look for state that is written but never cleaned up, and for state that a
  second run would trip over.
- Look for configuration the owner must change by hand, and ask whether the
  artifact says where it lives and what happens when it is missing.
- Look for limits (time, money, size, quota) without a stated behaviour when
  they are reached.
