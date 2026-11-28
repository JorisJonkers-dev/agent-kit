## Rate each task

Give every task one tier and a one-line reason: what makes it that hard.

| Tier | It means | Example |
|---|---|---|
| trivial | Mechanical, no judgement | Rename a symbol; summarise a file |
| routine | Well specified, one module, has tests | Add a flag to one command with its tests |
| hard | Crosses modules, is subtle, or touches security | A change across modules; a subtle bug; a security-relevant change |
| frontier | Open-ended | A design with unclear requirements; a hard debugging session with no repro |

Write the rating as `<task>: <tier> - <reason>`; a task with no reason is not
rated yet: read more or ask the owner.

## Splitting tasks

Split a task until each piece fits one fresh context window and one tier.

- Separate writing from reviewing: the model that wrote a change does not review
  it.
- Make pieces independent, with disjoint files, so they run in parallel.
- Name each dependency: a piece that needs another's output says so.
- Claim a ticket with `claim acquire <repo> <key> --issue <n>` before starting it, and release it with
  `claim release <repo> <key> --issue <n>` when done, so two agents never work the same ticket.
- Rate the pieces again after the split. A split that leaves a piece at the same
  tier for the same reason did not help; split along a different line.

