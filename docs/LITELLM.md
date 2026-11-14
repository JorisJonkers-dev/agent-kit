# LiteLLM: one proxy, per-key budgets

The cheap agents (Pi, Vibe, OpenCode) reach their models (the paid ones, the
OpenRouter free ones, the free-tier Google ones for public work and the local GPU) through one LiteLLM proxy in the estate cluster, shared by both machines, so one budget per
key holds across machines (spec #392, stories 96-98). Gold Band's own cost and
token numbers are never used; the proxy's refusal is the only budget signal.

## The keys and their caps

| Agent id | Virtual key (secret store) | Max budget, per 30 days | Access groups (`models`) | Reaches |
|---|---|---|---|---|
| `vibe` | `LITELLM_VIBE_KEY` | 255 EUR | `canonical` | every canonical model |
| `vibe-study` | `LITELLM_VIBE_STUDY_KEY` | 127.50 EUR | `canonical` | every canonical model |
| `opencode-go` | `LITELLM_OPENCODE_GO_KEY` | 60 EUR | `canonical` | every canonical model |
| `openrouter` | `LITELLM_OPENROUTER_KEY` | 5 EUR | `canonical` | every canonical model (its free `:free` deployments are in it; the cap bounds any paid fallback) |
| `local` | `LITELLM_LOCAL_KEY` | none | `canonical` | every canonical model, the GPU workstation's server (free) among them; the key has no cap, so use it for local models only |
| `public` | `LITELLM_PUBLIC_KEY` | 5 EUR | `google-free-public`, `canonical` | the free-tier Google models, and the paid fallbacks |

A key lists **access groups**, not a provider wildcard: a caller sends one canonical model
name (`glm-5.3`, `mistral-large`, `claude-sonnet-4-5`, ...) and the proxy routes it across the
provider accounts by priority (litellm#20), so the agent id no longer picks the provider. The
litellm#19 sync puts every deployment but a Google free-tier one, OpenRouter and local included, in `canonical`;
there are no per-route groups.
Which models an account can actually use is the live catalogue's job (below); a listed model
can still be refused.

`registry/estate-tooling.yaml` (`litellm:`) is the source. The render writes
`registry/generated/litellm/routes.json` (base URL, header, one route per agent id with its
key variable, cap (`null` is no cap), `budget_duration`, `data_scope` and access groups, never a
key value), the team each key goes into (in `registry/generated/workstation-secrets.json`) and
`registry/generated/litellm/make-admin-token.sh` (the owner's one-hour management key, below). LiteLLM counts `max_budget` in its own cost unit (USD): the number is entered as the
cap, and it is the owner's to rescale if the exchange rate matters.

**The budget resets.** Every capped route's team carries `budget_duration: 30d`, so a cap is a monthly
allowance that LiteLLM resets, not a lifetime cap that runs out for good. The render refuses a
capped route without it. The caps on the keys are a safety net; the provider accounts' own
monthly pools (vibe 255, vibe-study 127.50, vibe-api 25.50, vibe-study-api 12.75) are
capped on the deployments, in the litellm repository.

### The public key: public, non-sensitive work only

`public` is the only key with the `google-free-public` access group, and the Google free-tier
deployments are in that group alone, so every other key reaches a Gemini model only through its paid
deployments (the tests prove the keys' side in `tests/test_registry_render.py`: only `public` lists the group; the group on the deployments is the litellm repository's, and the render refuses
`google-free-public` on any route not marked `data_scope: public`, or a second public route).

**Google may use free-tier prompts to improve its products, so the `public` key is for
public, non-sensitive work only.** Use it for open-source code, public documentation and
questions you would post publicly. Never send it a private repository, employer code
(`nedap`), a secret, a transcript, a customer's data or anything under an NDA. When unsure, use
another key: the Gemini model is served by the others too, through the paid deployments.
Setup mints it like the other LiteLLM keys (`LITELLM_PUBLIC_KEY`, unexported).

### Provider priority (what the keys route to)

Set in the litellm repository (litellm#20); recorded here because the agent ids depend on it.

1. A model Vibe serves (including GLM 5.3, GLM 5.3 Flash, DeepSeek V4.1 Flash): `vibe` and
   `vibe-study` at 2:1, then `vibe-api` and `vibe-study-api` at 2:1, then stop. **No other
   provider serves a model Vibe serves**, OpenCode Go included.
2. Models only the general Mistral keys serve (audio, OCR, embeddings): `vibe-api` and
   `vibe-study-api` at 2:1.
3. Claude only through the `anthropic` key.
4. Gemini: Google free tier first, only inside `google-free-public`.
5. OpenCode Go only for models no Mistral, Google or Anthropic key serves. Its base is
   `https://opencode.ai/inference/go/openai/v1`.
6. OpenRouter free models only, `data_collection: deny`.

Mistral's four keys: `vibe` (255 EUR) and `vibe-study` (127.50 EUR) are Vibe-scope; `vibe-api`
(25.50 EUR) and `vibe-study-api` (12.75 EUR) are general API keys.

### Names the litellm repository must match

`canonical` (every canonical model), `google-free-public` (the Google free-tier deployments),
the agent ids / key aliases above, and the
canonical model names the gateway defaults use (`registry/litellm-catalogue.json` lists what
the catalogue serves).

## The gateway's models resolve through LiteLLM

The model-gateway defaults (`skills/workstation/model-gateway/defaults.json`) name only models
the catalogue lists as usable: Vibe's `deepseek-v4-1-flash-1017`, `glm-5-3`,
`mistral-large-latest`, `mistral-medium-latest`, `mistral-small-latest`, `zai-glm-5-3-flash`, and
OpenCode Go's `kimi-k2.7-code`, `mimo-v2.6-pro`, `mimo-v2.6-flash`, `qwen3.8-flash`. Because Vibe
serves GLM 5.3, GLM 5.3 Flash and DeepSeek V4.1 Flash, the OpenCode Go entries for them are
gone: OpenCode Go is not used for a model Vibe serves. `mistral-large-4-0-3` became
`mistral-large-latest`, and the deprecated `mistral-medium-3.5` (retires 2027-04-30) became
`mistral-medium-latest`. `tests/test_litellm_models.py` checks every default against
`registry/litellm-catalogue.json`, a snapshot of the catalogue's usable ids (its `comment` says
how to refresh it), and fails on one the catalogue does not list or on an OpenCode default that
Vibe serves.

## What a request carries

The proxy is anonymous at the edge (litellm#17): LiteLLM authenticates every request
itself, so a request carries its virtual key as a standard `Authorization: Bearer sk-...`, which
any client can send. Pi, Vibe and OpenCode will be configured from `routes.json` with that one
header. The Pi and launcher wiring that reads it lands with the Gold Band instances (B1); this
repository ships the table and the gate.

The old sign-in edge's auth-api service token, `LITELLM_EDGE_TOKEN`, is retired: setup removes
it from the secret store, and neither `litellm-gate` nor `measure_litellm_budget.py` sends a second
header any more (`--edge-token-env` is gone from `litellm-gate`).

## A budget refusal fails the node

`litellm-gate check` (`scripts/crates/litellm-gate`) makes one one-token
request with the agent id's key. A 2xx passes (exit 0). LiteLLM's refusal for a
spent key, HTTP 400 `budget_exceeded` (also 429), ends the node in failure
(exit 1, naming the node and the key variable); so does any other refusal or an
unreachable proxy. The key is read from the environment, sent through curl's
stdin config, and never printed. The stub tests drive both: the same stub
below the cap passes, and exhausted fails. `tests/exit_status.rs` runs the built
executable against a local stub proxy and asserts the process exit status (1 with
the one stderr line `litellm-gate: node <node> failed: budget exhausted for key
<VAR>`, 0 below the cap), so the exit code a node sees is proven, not just the
library's return value.

```bash
litellm-gate check --node research-fetch --url https://litellm.jorisjonkers.dev/v1 \
  --key-env LITELLM_VIBE_KEY --model glm-5.3-flash
```

## Making the keys (setup mints them)

Each route is a LiteLLM **team** that holds the route's access groups and budget, and each host
has its **own key** in that team, named `<agent>@<host>` (for example `vibe@enschede-rx7900xtx-1`).
Every host's key draws on the one team budget, and a lost machine's key is deleted alone.

Setup mints them itself (`mint: litellm` in the registry, done by `scripts/agent-kit-keys.py`).
When a key is missing from the secret store, or LiteLLM's `GET /key/info` refuses it, setup asks
once for a **LiteLLM admin token**: a management key that expires in one hour, which the owner
makes with the proxy's master key:

```bash
read -rs LITELLM_MASTER_KEY && export LITELLM_MASTER_KEY
registry/generated/litellm/make-admin-token.sh
```

The script calls `/key/generate` with `key_type: management` (management routes only, no model
call) and `duration: 1h`, the master key in a curl config on stdin so it reaches no argv, and
prints the new key. Paste it when setup asks; setup never stores it. To skip the prompt, put it
in `LITELLM_ADMIN_TOKEN` for the run instead. With the token, setup:

1. makes each route's team (`/team/new`, `team_alias` = the agent id) or brings an existing one
   to the registry's `max_budget`, `budget_duration` and access groups (`/team/update`), so a
   budget change is one registry edit and a setup run with the token;
2. deletes the route's old shared key (alias = the agent id), which both machines once used;
   the other machine's next setup run mints its own;
3. mints each missing key into its team (`/key/generate`, `key_alias` `<agent>@<host>`),
   replacing a key under the same alias that this host's store lost, checks it against
   `/key/info` and stores it.

Without a token setup says which keys it skipped and changes nothing on the proxy.

## The window and reset

Every capped route's team is made with `budget_duration: 30d`, so LiteLLM resets its spend
every 30 days. `scripts/measure_litellm_budget.py` confirms it on the real proxy: one `GET /key/info` with
the agent id's key (no model call, no spend), printing only `max_budget`, `budget_duration`,
`budget_reset_at` and `spend` (a minted key's team's, read with a second `GET /team/info`), and
the request with the key redacted. It reads the key from the
environment or the secret store:

```bash
python3 scripts/measure_litellm_budget.py --agent vibe
```

A minted key draws on its route's team, so the window and reset are the team's. Measured on the
real proxy on 2026-11-14 at 12:45 UTC, with this host's `vibe@enschede-rx7900xtx-1` key:

```text
request:
  GET https://litellm.jorisjonkers.dev/key/info
  Authorization: Bearer <LITELLM_VIBE_KEY, redacted>
GET https://litellm.jorisjonkers.dev/team/info?team_id=f1551bd0-f418-4f6e-b194-9d97414d07d6
  Authorization: Bearer <LITELLM_VIBE_KEY, redacted>
budget:
  max_budget: 255.0
  budget_duration: "30d"
  budget_reset_at: "2026-12-01T00:00:00Z"
  spend: 0.0001643
```

The cap is 255 (LiteLLM's unit), the window is `30d`, and LiteLLM resets the spend on
2026-12-01, then every 30 days. Every host's `vibe` key counts against this one team budget.

## What the owner adds

- The OpenRouter key, with a 5 EUR credit limit set on OpenRouter, written to Vault
  as `openrouter_api_key` (`./bootstrap/secrets.sh --only litellm` in estate); on OpenRouter, training
  providers off for free models. The general Mistral keys, the Google and the Anthropic keys
  (`vibe_api_general_key`, `vibe_study_api_general_key`, `google_api_key`, `anthropic_api_key`) are
  written the same way (estate#382); Anthropic: a $100 monthly spend limit, auto-reload off;
  OpenCode Go: Extra Usage off.
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
