### 4. Report

Reply in at most about eight lines:

```
<spec or draft:name> · <phase>[ · critique skipped on purpose]
Stages: decisions <status> (<run ids>) · plan … (a refactor only) · spec … · tickets … · feature review …
Tickets: #183 reviewed, landed · #184 skipped-low-risk, landed · #185 not-reviewed
Build tracker: <page address or path>
Questions: <n> open; critical <finding_id> (<stage>, run <run_id>): <question>
Gate: none | PASS for <short head> | <gate line>
Next: <step>. <reason>
<offer>
```

- **Stages**: each stage's `status` with its `run_ids`; `not-run` needs none.
- **Tickets**: during implementation and finishing only, in the state's
  order (dependency order), each with its `review` and whether it `landed`.
- **Build tracker**: during implementation and finishing only, the line step 3
  kept: the build tracker page's address or path, or what failed.
- **Questions**: the open needs-human questions, critical first. Take each
  question's text from `## Questions for you` in the run's `report.md` in
  the runs folder; without it, give the finding id alone.

The last two lines come from `next`:

| `next.kind` | Next | Offer |
|---|---|---|
| `stage` | the stage skill (table below), with `next.reason` | "Run it now?" |
| `command` | "type `<next.command>`", with `next.reason` | "Type `/flow` when it is done." For `/implement`: "Start it in a fresh session, one session per ticket." For `/grill-with-docs` on a new feature: "Type `/grill-with-docs <idea>`, then `/flow`." |
| `done` | `next.reason` | none |

| `next.stage` | Stage skill | Inputs you give it |
|---|---|---|
| `decisions` | `review-grill` | `"flow": "draft:<name>"` for its prepare, so the run joins the draft's flow |
| `plan` | `critique-plan` | `"flow": "draft:<name>"` for its prepare; with `previous_run_id`, `"previous_run_dir"` for the second round |
| `spec` | `review-spec` | the spec; with `previous_run_id`, `"previous_run_dir"` = that run's directory in the runs folder, for the second round |
| `tickets` | `review-tickets` | the spec |
| `ticket-review` | `review-implementation` | `next.ticket` and the spec |
| `feature-review` | shown as `/flow finish` | (see `/flow finish`) |

A draft is a refactor when its plan file exists (`state.refactor`, at
`state.plan_path`): its next stage is `plan` in place of `decisions`. After
`/improve-codebase-architecture` has grilled a candidate, suggest
`/critique-plan`, which drafts the plan when none exists.

With `resume_run_id`, the stage owes a run that was prepared but not
recorded: finish that run rather than prepare another. Its directory in the
runs folder holds `manifest.json` (with `packet_dir` and `critics`). Follow
the stage skill from its fan-out step with that `run_dir`, or, when
`findings.json` is already there (fan-out's result), from its ruling step.

Done when the reply ends with the next step, its reason and the offer.

### 5. Act on the answer

- **Yes to a stage**: load the stage skill (by name through your agent's
  skill tool, or by reading `../<skill>/SKILL.md` from this file's folder)
  and follow it with the inputs above. When it returns, go back to step 2
  for the new state, then refresh and report again (steps 3 and 4).
- **The owner reports a step from the table in step 2**: record it, then
  refresh and report again (steps 3 and 4).
- **No**: stop. The saved state keeps the place for the next `/flow`, in
  this session or a later one.

