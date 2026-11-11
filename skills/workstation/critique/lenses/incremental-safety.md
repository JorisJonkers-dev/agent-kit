# Lens: incremental safety

Check that each step is a change small and safe enough to ship on its own and
to keep main working. Each check fixes the severity of its findings, given in
brackets. A step that keeps main green at every point is no finding.

- `mixed-change` (major): Look for a step whose Technique or Done-when changes
  structure and behaviour together, whatever its Kind says. Quote the step
  lines that show both, so the reader sees the two changes in one step.
- `no-safety-net` (critical): Look for a step that changes code the plan itself
  says is untested, or whose Safety net names no test, with no characterize
  step before it in the Blocked-by graph. Trace the Blocked-by fields back from
  the step and show that no characterize step sits on the path.
- `big-bang-change` (major): Look for a step that changes an interface and all
  its callers at once, where a parallel change (expand, migrate, contract)
  would let callers move one at a time. Quote the step and the callers it
  names.
- `broken-main-order` (critical): Look for an order of steps that leaves main
  broken between two steps: a step removes or changes what a later step, or a
  caller not yet migrated, still needs. Give it as a trace through the steps'
  Blocked by fields, step by step, so the break shows at one point.
- `no-rollback` (critical): Look for an irreversible step (a data migration, a
  contract that drops a column, table or public API, a deletion of stored
  data) whose Rollback says none or gives no way back, with no notice that the
  owner accepts the one-way step.
- `scaffold-left` (major): Look for a toggle, shim, adapter or flag that a step
  adds (its Technique or Done-when says so) and that no step removes, even
  where its Transitional element field says none. Name the step that adds it
  and show that no step removes it.

The bracketed severity is the finding's: write it, and the runner sets it
whatever you write. A plan that keeps main green at every step is no finding;
prefer a few findings you can quote over many you can only describe.
