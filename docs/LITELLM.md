# LiteLLM: one proxy, per-key budgets

The cheap agents (Pi, Vibe, OpenCode) reach their paid models through one
LiteLLM proxy in the estate cluster, shared by both machines, so one budget per
key holds across machines (spec #392, stories 96-98). Gold Band's own cost and
token numbers are never used; the proxy's refusal is the only budget signal.

## The caps

| Agent id | Virtual key (secret store) | Max budget |
|---|---|---|
| `vibe` | `LITELLM_VIBE_KEY` | 255 EUR |
| `vibe-study` | `LITELLM_VIBE_STUDY_KEY` | 128 EUR |
| `opencode-go` | `LITELLM_OPENCODE_GO_KEY` | 60 EUR |

`registry/estate-tooling.yaml` (`litellm:`) is the source; the render writes
`registry/generated/litellm/routes.json` (base URL, header, one route per agent
id with its key variable, cap and models, never a key value). LiteLLM counts
`max_budget` in its own cost unit (USD): the number is entered as the cap, and
it is the owner's to rescale if the exchange rate matters.

## What a request carries

The proxy is internal only: its host is `audience: authenticated`, so the edge
signs a caller in first. A request carries an auth-api service token as
`Authorization: Bearer` (`LITELLM_EDGE_TOKEN`, from your own sign-in; no
`LITELLM` service exists in auth-api yet) and its virtual key in
`x-litellm-api-key`. Pi, Vibe and OpenCode are configured from `routes.json`
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

```bash
curl -sS https://litellm.jorisjonkers.dev/key/generate \
  -H "Authorization: Bearer $EDGE_TOKEN" -H "x-litellm-api-key: Bearer $MASTER" \
  -H 'Content-Type: application/json' \
  -d '{"key_alias":"vibe","max_budget":255,"models":["vibe/zai-glm-5-3-flash","vibe/deepseek-v4-1-flash-1017"]}'
```

Repeat for `vibe-study` (128) and `opencode-go` (60) with their models, then run
setup in a terminal and paste each key when asked.

## The window and reset

**Not measured yet.** The caps are set without `budget_duration`, so each is
a lifetime cap until measured. The ticket asks this section to state the
window and reset as measured on the real proxy, with the request that measured
them; that needs the deployed proxy and a throwaway key (`max_budget` 0.01,
called until refused, HTTP code recorded). Record the result here then.
