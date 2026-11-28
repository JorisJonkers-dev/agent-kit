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

## Choose by tier

A LiteLLM tier is the unit you plan with: you name the tier and LiteLLM walks its chain, so you never pick the
model. Read the tier names and chains in `registry/litellm-tiers.json`; the gateway runs one with
`model-gateway run --tier <tier> --json '{...}'` (a tier is not combined with `client`, `model`, `profile` or
`harness`; an unknown tier is `unknown-tier`; `batch` takes `client` and `model`, so a tier delegation is a
`run`).

| Work | Tier |
|---|---|
| Public reading and trivial summaries; research first | `tier-free` |
| Routine implementation with tests, and `/tdd` | `tier-go-cheap` |
| Research when the free tier cannot answer | `tier-vibe` |
| Review, critique and `/code-review` | `tier-review` |
| Open-weight work above `tier-go-cheap`; an escalation | `tier-strong-open` (refused with `tier-key-missing` until a key is provisioned) |

Move up one tier only with a reason you can write down, and never to a costlier ceiling than the task needs. The
free tiers are public-data only: the data policy keeps a repository's code off them, so a private task goes to
`tier-vibe` or `tier-go-cheap`. The run writes its routing record on its ledger line (`$XDG_STATE_HOME/agent-kit/model-gateway/ledger.jsonl`, else `~/.local/state/agent-kit/model-gateway/ledger.jsonl`; the line whose `run_id` is the run's): `tier`, `harness`, `served_model`, `attempted_fallbacks`, `fallback_fired`, `cost_usd` and `routing_capture`. Read that line instead of guessing, and say in your report which tier ran, which model served it and whether a fallback fired. A `null` field is reported as unknown, with the `routing_capture` that explains it.
Claude Code stays the interactive author and the orchestrator that rules on findings. Never send Claude traffic through LiteLLM: a tier run takes no `client`, `model`, `profile` or `harness`, so it cannot name a Claude model.

