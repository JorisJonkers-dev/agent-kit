# Lens: quality scenarios

Check that each quality claim can be measured, that each structural decision
has a check that enforces it, and that trade-offs between qualities state
which one wins.

- `unmeasured-quality`: A quality claim (fast, reliable, secure, scalable,
  simple) with no stimulus (what happens), no environment (where and under
  what load) and no numeric response measure. Quote the claim.
- `no-fitness-check`: A structural decision (a layer rule, a dependency
  direction, a size limit) with no automated check that would fail when it is
  broken. Quote the decision and say what the check would test.
- `unprioritised-tradeoff`: Two quality goals that pull against each other,
  named in the same record or the same spec, with no stated priority between
  them. Quote both goals.
- `upside-only-consequence`: A decision whose consequences list names only
  benefits. Quote the consequences section and say what cost or risk it does
  not name, if the decision has one.

The numeric response measure is a threshold a test or a fact can settle. A
claim with no threshold is a finding only with a stimulus that makes the
missing number matter.
