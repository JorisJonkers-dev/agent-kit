## /flow finish

1. Find the feature and read its state (steps 1 and 2). Unless `next` is the
   `feature-review` stage, refresh and report as `/flow` does (steps 3 and
   4) and stop: that report names what is still due.
2. Load the `review-pr` skill and follow it with the spec (and the
   resume inputs when `next` has `resume_run_id`).
3. Read the state again, then refresh the build tracker as step 3 of `/flow`
   says. Once the gate passes, that is the build's last render: the page
   shows the finished build. When the gate fails, the refresh adds the
   follow-up tickets.
4. Reply with the gate line as the runner wrote it, one line per follow-up
   ticket the review filed, the Build tracker line, and the next step: `done` when
   the gate passes for the current head, else the `/implement` of the first
   follow-up.

## /flow no-critique

Find the feature (step 1), then record `{"no_critique": true}`
(`/flow critique` records `false`). From then on the next step skips every
critique stage: grill, `/to-spec`, `/to-tickets`, one `/implement` per
ticket, then done, with no feature review. The ledger keeps the choice, so
the flow shows critique was skipped on purpose. Then refresh the build
tracker and report as `/flow` does (its steps 3 and 4): a feature built
without critique gets its build tracker too.

