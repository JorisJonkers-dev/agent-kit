# LiteLLM: one proxy, per-key budgets

The cheap agents (Pi, Vibe, OpenCode) reach their models (the paid ones, the
OpenRouter free ones and the local GPU) through one LiteLLM proxy in the estate cluster, shared by both machines, so one budget per
key holds across machines (spec #392, stories 96-98). Gold Band's own cost and
token numbers are never used; the proxy's refusal is the only budget signal.

## The caps

| Agent id | Virtual key (secret store) | Max budget | Provider |
|---|---|---|---|
| `vibe` | `LITELLM_VIBE_KEY` | 255 EUR | Mistral, the Vibe key |
| `vibe-study` | `LITELLM_VIBE_STUDY_KEY` | 128 EUR | Mistral, the Vibe-study key |
| `opencode-go` | `LITELLM_OPENCODE_GO_KEY` | 60 EUR | OpenCode Go |
| `openrouter` | `LITELLM_OPENROUTER_KEY` | 5 EUR | OpenRouter; prefer the `:free` ids |
| `local` | `LITELLM_LOCAL_KEY` | none | the OpenAI-compatible server on the GPU workstation (free) |

Each route is a provider wildcard (`vibe/*`, `openrouter/*`, ...): a caller sends
`<agent id>/<provider model id>` and any model the provider offers works. The key's
`models` is that wildcard. Which models an account can actually use is the live
catalogue's job (below); a listed model can still be refused.

`registry/estate-tooling.yaml` (`litellm:`) is the source; the render writes
`registry/generated/litellm/routes.json` (base URL, header, one route per agent
id with its key variable, cap (`null` is no cap) and models, never a key value). LiteLLM counts
`max_budget` in its own cost unit (USD): the number is entered as the cap, and
it is the owner's to rescale if the exchange rate matters.

## What a request carries

The proxy is anonymous at the edge (litellm#17): LiteLLM authenticates every request
itself, so a request carries its virtual key as a standard `Authorization: Bearer sk-...`, which
any client can send. Pi, Vibe and OpenCode will be configured from `routes.json` with that one
header. The Pi and launcher wiring that reads it lands with the Gold Band instances (B1); this
repository ships the table and the gate.

**Legacy edge token.** While the host still signs callers in (before litellm#17 is deployed), the
edge wants an auth-api service token as `Authorization: Bearer`, so the key goes in
`x-litellm-api-key` instead. LiteLLM always accepts that header and prefers it to
`Authorization`, so it works in both states. The tools use that two-header form only while
`LITELLM_EDGE_TOKEN` is stored (`litellm-gate --edge-token-env`, `measure_litellm_budget.py`).
Once the anonymous edge is live, remove `LITELLM_EDGE_TOKEN` from the store and the tools send
the plain bearer. The auth-api service-token path for LiteLLM is then unused.

## A budget refusal fails the node

`litellm-gate check` (`scripts/crates/litellm-gate`) makes one one-token
request with the agent id's key. A 2xx passes (exit 0). LiteLLM's refusal for a
spent key, HTTP 400 `budget_exceeded` (also 429), ends the node in failure
(exit 1, naming the node and the key variable); so does any other refusal or an
unreachable proxy. The key is read from the environment, sent through curl's
stdin config, and never printed. The stub tests drive both: the same stub
below the cap passes, and exhausted fails.

```bash
litellm-gate check --node research-fetch --url https://litellm.jorisjonkers.dev/v1 \
  --key-env LITELLM_VIBE_KEY --model vibe/zai-glm-5-3-flash
```

## Making the keys (owner, once the proxy is deployed)

With the proxy's master key (never stored in this repository):

Put the header in a curl config on stdin so no key reaches argv:

```bash
curl -sS --config - https://litellm.jorisjonkers.dev/key/generate <<CFG
header = "Authorization: Bearer $MASTER"
header = "Content-Type: application/json"
data = "{\"key_alias\":\"vibe\",\"max_budget\":255,\"models\":[\"vibe/*\"]}"
CFG
```

Repeat for `vibe-study` (128), `opencode-go` (60) and `openrouter` (5) with their
wildcards, and for `local` without `max_budget`, then run setup in a terminal and
paste each key when asked. `key_alias` must be the agent id: the dashboards and
alerts group by it.

## The window and reset

**Not measured yet.** The caps are set without `budget_duration`, so each is
a lifetime cap until measured. The ticket asks this section to state the
window and reset as measured on the real proxy, with the request that measured
them. `scripts/measure_litellm_budget.py` makes that request: one
`GET /key/info` with the agent id's key (no model call, no spend), printing only
`max_budget`, `budget_duration`, `budget_reset_at` and `spend`, and the request
with both credentials redacted. It reads the key and the edge token from the
environment or the secret store, where setup put them:

```bash
python3 scripts/measure_litellm_budget.py --agent vibe
```

Result, to be pasted here by the owner (`_pending_`): the request line, the four
fields, the date it was run, and one sentence: is the cap a lifetime cap
(`budget_duration` null) or a window that resets (and when)?

```text
_pending_
```

## What the owner adds

- The OpenRouter key, with a 5 EUR credit limit set on OpenRouter, written to Vault
  as `openrouter_api_key` (`./bootstrap/secrets.sh --only litellm` in estate).
- The local model server, reachable from the cluster: Ollama on the workstation
  already serves an OpenAI-compatible `/v1` (nix-config `ollama-rocm.nix`, port
  11434), but a Project's pods reach nothing outside the cluster except the
  public internet, so the `local` route answers 502 until the follow-up ticket
  makes it reachable.
- Optionally a private `JorisJonkers-dev/litellm-model-catalog` repository and a
  fine-grained token on it, as `catalog_publish_token`.

## Prices and the budget gate

A cap counts only what LiteLLM can price. A model with no price records spend 0
and its key's budget does not bite; the estate alert `LiteLLMSpendBlind` fires
when a paid key moves tokens and records no spend. The free OpenRouter ids and the
local route cost nothing by design.

## Telemetry and the live catalogue

Telemetry is metadata only (model, key alias, tokens, cost, latency, status; never
prompt or response text), exported as Prometheus metrics and OpenTelemetry traces
and shown on the `LiteLLM` Grafana dashboard: see the litellm repository's
README. The same repository publishes the live model catalogue: each route's
provider list, probed with one 1-token request per model through that route's key
and classified `usable`, `blocked` (reason kept), `unknown`, `budget-exhausted`,
`error`, `not-chat` or `unprobed`, with free models marked. The model gateway and
the Gold Band agent ids read it instead of a static list (follow-up ticket); it
follows the Vibe alias probe's idea (`parseVibeAvailable`, #391) and replaces
guessing from the provider's list.
