# Lens: test seams

Check that behaviour can be tested at a seam that needs no real database,
network or clock, and that the tests check behaviour rather than structure. A
seam is a place where a test can replace a dependency, such as a driven port
with a fake.

- `needs-real-infrastructure`: A behaviour reachable only by a test that
  starts a database, a network service or a real clock. Name the behaviour and
  the test that would be needed.
- `coupled-to-internals`: A test that asserts on a private function, a call
  count or an internal field, so that a change that keeps behaviour breaks it.
  Quote the assertion.
- `no-assertion-or-pasted-value`: A test with no assertion, or one that
  asserts a value copied from the output of the code it tests, with no
  reason the value is right. Quote the assertion or the absence of one.
- `no-fake`: A driven port with no fake, or a fake that does not hold to the
  same behaviour as the real adapter. Name the port and the contract the fake
  would need to meet.

A finding names the seam the behaviour should be tested at, so the fix is
checkable.
