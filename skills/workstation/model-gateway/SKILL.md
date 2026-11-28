---
name: model-gateway
description: Holds the model gateway's runner and its shipped defaults; it is not a workflow of its own, since the delegate skill drives it.
---

# model-gateway

This folder is what the model gateway runs on. It does nothing on its own: an
agent calls the runner, and the `delegate` skill is what
teaches an orchestrator to plan with it. The runner answers these questions:
`models`, which client models this machine can use right now; `catalog`, every canonical model the proxy serves with what serves it and what it costs; `usage`, what
each client can do now and what it has spent; `run`, one delegation to one of
them; `batch`, a whole plan of delegations in one call; `ps`, `status`, `logs`, `tail`, `cancel` and `gc`, what the runs are doing and the means to end them; `workflow`, the life of the piece of work a delegation belongs to; and
`grant`, the owner's permission to spend their employer's money on one of
them.

## What is in here

- The runner is the Rust `model-gateway` command on PATH (setup builds it with
  `scripts/install-model-gateway-rust.sh`); this folder holds only its defaults,
  pinned in `manifest.yaml`'s `model_gateway.files`.
- `defaults.json` — the shipped defaults: the clients on this machine, the
  client models each offers with their family, difficulty tier and capability
  notes, and the caps and thresholds a delegation is held to.
- `defaults.schema.json` — the JSON Schema `defaults.json` is validated
  against. The Python asset validators check the shipped file against it, so a
  broken default fails CI rather than a delegation.

The runner reads `defaults.json` from the folder it sits in, so the two always
travel together.

## Core commands

```sh
model-gateway models [--free]                        # client models usable now, with tier and cost class
model-gateway catalog [--free]                       # every canonical model the proxy serves
model-gateway usage --json '{"days": 7}'             # what each client can do now and has spent
model-gateway run   --input run.json                 # one delegation, one result
model-gateway batch --input plan.json                # a whole plan in one call
model-gateway ps | status RUN | tail | logs RUN      # what the runs are doing
model-gateway cancel RUN                             # end a run
```

Every command also takes its input inline as `--json '{...}'`.

Models are named by address: `free/<model>`, `paid/<model>`, `auto/<model>`,
`<harness>:<pricing>/<model>` or a `tier-*` name. Each command's input, output
and refusal codes are in the part files below. Read the part you need, not all
of them. "Above" and "below" in a part file mean that file, or the order of
this list.

## Where each part lives

| Question | File |
| --- | --- |
| Which client models exist, their tiers, addresses, Pi, the live LiteLLM catalogue | [models.md](models.md) |
| What `catalog` and `usage` report | [catalog-and-usage.md](catalog-and-usage.md) |
| The `run` command: input, waiting for a free model, tiers, refusals | [run.md](run.md) |
| Sandboxes (`sbx`), launch per client, run results, process groups | [sandbox.md](sandbox.md) |
| The event log, `ps`, `status`, `tail`, `logs`, `cancel` | [events-and-status.md](events-and-status.md) |
| A delegation fails or cannot start; web tools in a Vibe run | [failures.md](failures.md) |
| Writing runs and worktrees | [writing-runs.md](writing-runs.md) |
| `batch` and the `workflow` commands | [batch.md](batch.md) |
| Watching a batch as an orchestrator (the commands, in order) | [watching-a-batch.md](watching-a-batch.md) |
| `grant`: spending an employer's money | [grants.md](grants.md) |
| Delegation profiles, roles, harnesses, tiers | [profiles-and-tiers.md](profiles-and-tiers.md) |
| The data policy and the machine override | [policy.md](policy.md) |
