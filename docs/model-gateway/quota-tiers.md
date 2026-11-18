# Quota-aware tier choice (#907)

`model-gateway run --tier T` reads how much of the plan the tier draws on is used, and drops to the next cheaper
tier when a window is at 80%. Part of #916 (the design is in the free-inference plan, litellm#57).

## The ladder and what "next" means

```
tier-free  <  tier-vibe  <  tier-go-cheap  <  tier-strong-open        (cheapest first)
```

A drop goes **down** this list and never up: a run never costs more than the tier it asked for. "Next" is the next
cheaper rung that still serves the role and has quota:

- `tier-strong-open` and `tier-go-cheap` both draw on Go's allowance. When Go is the reason, the drop from the first
  passes over the second (it is out too) and lands on `tier-vibe`.
- `tier-vibe` drops to `tier-free`, which has no plan to run out of. It is the floor.
- A tier that is not on the ladder (`tier-review`, which already tries free models first) is never moved.
- A cheaper rung is a candidate only when the defaults state its tier and this machine holds its key.
- If every rung down to the floor is out, the last one is chosen and the decision says so.

## What is read

| Signal | Where | Applies to |
|---|---|---|
| Go allowance | `usage.opencode.allowance` (USD) in the response body of an OpenCode Go request, summed per run by the routing proxy and written on the ledger line as `allowance_usd`. The windows are summed from the ledger: rolling 5 hours, rolling week, and the 30-day cycle anchored at 2026-11-04T10:41Z | `tier-go-cheap`, `tier-strong-open` |
| Account monthly cap | LiteLLM's `x-litellm-response-cost` per request (ledger `litellm_cost_usd`), summed per account (the part of `litellm_model_id` before the `/`) from the 1st of the month (UTC), against `vibe` 255, `vibe-study` 127, `vibe-api` 25.5, `vibe-study-api` 12.7 | `tier-vibe` |
| Key-level cap | `GET /key/info` with the tier's own key (a workstation key may call it on itself): `spend` against `max_budget`. Used only when the ledger holds no signal for the tier | every ladder tier |

There is no endpoint that reports the remaining Go allowance (checked 2026-11-18: LiteLLM's `/spend/logs` and
`/global/spend/report` need the admin role). The gateway therefore sums what each response reported. The 5-hour
and weekly limits are configured, not read: they default to OpenCode Go's published 12 USD and 30 USD, and the cycle
limit to the account cap of 40 USD in litellm's `catalog/routing.json`. Spend that did not go through the gateway
(Console, another client) is not seen; the key-level cap is the backstop for that.

An account is out when any of its windows is at the threshold. A tier is out when every one of its accounts is: a
Vibe tier with one account left still serves. With no signal at all (no allowance recorded, no key cap) the tier is
chosen as asked and the decision says `no-signal`.

## The decision on the ledger line

Every `--tier` run's ledger line carries `tier_decision`: `requested`, `chosen`, `reason` and `signals` (one entry per
window read: tier, source, account, window, used, limit, ratio). The run's `tier` member is the tier it ran as.

## Parallelism

A tier takes only so many runs at once. Each tier has that many slot files under the gateway's state directory
(`tier-slots/`), taken with the same exclusive create the ledger lock uses; a run holds one until it ends and waits for
one otherwise. A slot older than `slot_stale_s` belongs to a run that died and is broken. Defaults: `tier-free` 4,
`tier-vibe` 3, `tier-go-cheap` 2, `tier-strong-open` 1, `tier-review` 3, any other tier 2. A run that waits longer than
`slot_wait_s` is refused with `tier-busy`.

## Settings

All optional, under `settings.quota` of `defaults.json` or the machine file:

```json
{"settings": {"quota": {
  "threshold": 0.8,
  "go_five_hour_usd": 12, "go_weekly_usd": 30,
  "accounts": {"opencode-go": 40, "vibe": 255},
  "parallel": {"tier-vibe": 3}, "default_parallel": 2,
  "slot_wait_s": 600, "slot_stale_s": 7200
}}}
```
