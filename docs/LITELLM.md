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

The proxy is internal only: its host is `audience: authenticated`, so the edge
signs a caller in first. A request carries an auth-api service token as
`Authorization: Bearer` (`LITELLM_EDGE_TOKEN`, pasted once at setup like the
keys; no `LITELLM` service exists in auth-api yet, so it is minted by hand) and its virtual key in
`x-litellm-api-key`. Pi, Vibe and OpenCode will be configured from `routes.json`
with those two values. The Pi and launcher wiring that reads it lands with the
Gold Band instances (B1); this repository ships the table and the gate.

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
  --key-env LITELLM_VIBE_KEY --edge-token-env LITELLM_EDGE_TOKEN --model vibe/zai-glm-5-3-flash
```

## Making the keys (owner, once the proxy is deployed)

With the proxy's master key (never stored in this repository):

Put the headers in a curl config on stdin so no token reaches argv:

```bash
curl -sS --config - https://litellm.jorisjonkers.dev/key/generate <<CFG
header = "Authorization: Bearer $EDGE_TOKEN"
header = "x-litellm-api-key: Bearer $MASTER"
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
them; that needs the deployed proxy and a throwaway key (`max_budget` 0.01,
called until refused, HTTP code recorded). Record the result here then.

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
