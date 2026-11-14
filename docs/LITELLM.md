# LiteLLM: one proxy, per-key budgets

The cheap agents (Pi, Vibe, OpenCode) reach their models (the paid ones, the
free-tier ones for public work: Google and OpenRouter, and the local GPU) through one LiteLLM proxy in the estate cluster, shared by both machines, so one budget per
key holds across machines (spec #392, stories 96-98). Gold Band's own cost and
token numbers are never used; the proxy's refusal is the only budget signal.

## The keys and their caps

| Agent id | Virtual key (secret store) | Max budget, per 30 days | Access groups (`models`) | Reaches |
|---|---|---|---|---|
| `vibe` | `LITELLM_VIBE_KEY` | 255 EUR | `canonical` | every canonical model |
| `vibe-study` | `LITELLM_VIBE_STUDY_KEY` | 127.50 EUR | `canonical` | every canonical model |
| `opencode-go` | `LITELLM_OPENCODE_GO_KEY` | 60 EUR | `canonical` | every canonical model |
| `openrouter` | `LITELLM_OPENROUTER_KEY` | none | `openrouter-free-public` | OpenRouter's free `:free` models only, nothing paid; **public work only** |
| `local` | `LITELLM_LOCAL_KEY` | none | `canonical`, `local/*` | every canonical model, and `local/<id>` for any id the GPU workstation's server (free) serves, such as `local/qwen3:32b`; the key has no cap, so use it for local models only |
| `public` | `LITELLM_PUBLIC_KEY` | 5 EUR | `google-free-public`, `canonical` | the free-tier Google models, and the paid fallbacks; **public work only** |

A key lists **access groups**, not a provider wildcard (the free `local` key alone adds its own
`local/*`: LiteLLM puts a wildcard in an access group only with an Enterprise licence, so the key
names it): a caller sends one canonical model
name (`glm-5.3`, `mistral-large`, `claude-sonnet-4-5`, ...) and the proxy routes it across the
provider accounts by priority (litellm#20), so the agent id no longer picks the provider. Every paid
deployment, and the local one, is in `canonical`. The free-tier deployments are not: each free
provider has its own public-only group (litellm#33), and only the `data_scope: public` keys hold one.
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

### The public-only keys: public, non-sensitive work only

A free-tier provider may use the prompts to train or improve its models, so every free-tier
deployment is **public work only** (agent-kit#737). The litellm repository keeps each free
provider's deployments in a public-only access group of its own, never in `canonical`:

| Public-only group | What is in it | Model names | Held by |
|---|---|---|---|
| `google-free-public` | the Google free tier first, then the paid deployments of the same model | `public/<canonical>` | `public` |
| `openrouter-free-public` | OpenRouter's free `:free` models, each alone | `openrouter/<canonical>` | `openrouter` |

One group per provider, not one shared free group: LiteLLM grants a key a model name, so a
shared group would hand each free provider's key the others' models. The `openrouter` key holds
its own group and **not** `canonical`, so it reaches nothing paid and carries no cap. `public`
also holds `canonical`, so a Gemini model falls over to the paid deployments when the free tier
refuses; its small cap bounds that.

The render enforces the keys' side (`scripts/render_registry.py`, tested in
`tests/test_registry_render.py`): a route holding any public-only group must be
`data_scope: public`, and a `data_scope: public` route must hold at least one. Several public
routes are allowed. The groups on the deployments are the litellm repository's.

**Use a public-only key for public, non-sensitive work only.** Use it for open-source code,
public documentation and questions you would post publicly. Never send it a private repository,
employer code (`nedap`), a secret, a transcript, a customer's data or anything under an NDA, and
never route a critic on a private repository to one. When unsure, use a `canonical` key: a model a
paid provider serves is reachable there too.

There is no route for OpenCode Zen's free models: OpenCode's docs say they answer HTTP 403 to
every client but OpenCode itself, so a route through the proxy would serve nothing.

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
6. OpenRouter free models only, `data_collection: deny`, in `openrouter-free-public` alone (never
   in `canonical`).

Mistral's four keys: `vibe` (255 EUR) and `vibe-study` (127.50 EUR) are Vibe-scope; `vibe-api`
(25.50 EUR) and `vibe-study-api` (12.75 EUR) are general API keys.

### Names the litellm repository must match

`canonical` (every paid and local deployment), the public-only groups `google-free-public` and
`openrouter-free-public` (the free-tier deployments,
`LITELLM_PUBLIC_GROUPS` in `scripts/render_registry.py`), the agent ids / key aliases above, and the
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
`registry/litellm-catalogue.json`, a trimmed snapshot of the catalogue in its own shape (its
`comment` says how to refresh it), and fails on one the catalogue does not list as usable or on
an OpenCode default that Vibe serves.

The defaults no longer decide which models a route offers: they describe a model (family, tier,
notes, prices) the gateway already lists from the client's own answer, and the live catalogue
(below) then decides whether it is usable.

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

A minted key takes its team's access groups (`models: ["all-team-models"]`), so changing a
route's groups in the registry is a team update on the next setup run with a token, and needs
no new key. A key minted before that holds its groups itself: delete it from the store
(`agent-kit-secret delete LITELLM_<ROUTE>_KEY`) and run setup with a token to mint it anew. This
applies to the `openrouter` key minted before it became public-only: it still lists `canonical`.

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
- The local model server's gate key: Ollama on the workstation listens on loopback, and a
  keyed gate serves its `/v1` on the tailnet address, port 11435 (nix-config
  `services.llmLocalGate`, agent-kit#569). The same key is in
  `/var/lib/secrets/llm-local-gate.key` on the workstation and in Vault as
  `local_llm_api_key` (`./bootstrap/secrets.sh --only litellm` in estate). A pod
  without it gets 401; while a game holds the GPU the gate answers 503 at once.
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
`error`, `not-chat` or `unprobed`, with free models marked. It follows the Vibe alias
probe's idea (`parseVibeAvailable`, #391) and replaces guessing from the provider's list.

### Where the live copy comes from (#568)

The catalogue repository (`JorisJonkers-dev/litellm-model-catalog`, `model-catalog.json`) is
private, so setup fetches it with the owner's GitHub login (`gh api`, raw contents) into
`$XDG_CACHE_HOME/agent-kit/litellm-catalogue.json`, else `~/.cache/agent-kit/litellm-catalogue.json`:
the published file verbatim, so the file's mtime is when it was fetched and its `updated_at` is
when it was probed. The fetch is checked before it replaces the old copy; a failed fetch is a
setup warning and the old copy stays. `python3 scripts/litellm_catalogue.py fetch` does the same
by hand, and `... show [--provider vibe]` prints what the catalogue says.

Readers:

- **The model gateway** (node and Rust) reads the live copy for each client with a
  `litellm_route` in its defaults (`vibe` on `vibe`, `opencode` on `opencode-go`): it lists only
  the models the catalogue calls usable, skips a blocked one with the provider's reason, marks a
  free one `"free": true`, and refuses a run on a refused model (`model-not-usable`). A model the
  catalogue does not list is listed as before. See the gateway's SKILL.md.
- **Setup's own writers** (the Pi providers per route) use `scripts/litellm_catalogue.py`:
  `load_catalogue()` returns the live copy, else the snapshot `registry/litellm-catalogue.json`,
  with every problem as a warning; `provider_models(catalogue, provider)` lists one provider
  route's probes; `canonical_groups(catalogue, access_groups)` lists the canonical model names a
  key with those access groups reaches. A public-only group (a free tier) is reached only through
  an access group that only `data_scope: public` routes hold in `routes.json`, or the access
  groups the catalogue names on the group itself, so no group name is fixed in the reader.

- **The Pi provider table** (`registry/generated/pi/providers.json`, rendered by
  `scripts/render_registry.py`) takes each route's models from `canonical_groups()`; no model list is
  kept by hand any more (the old `registry/litellm-access-groups.json` is gone). Only chat groups are
  listed. Each Pi model entry carries `contextWindow` (the group's `context_length`: the smallest among
  its first-tried deployments, left out when unknown) and, for a free group, an all-zero `cost`. The
  catalogue holds no price, only whether a model is free, so a paid model has no `cost` entry.
  The render must give the same text on every machine for `--check`, so it reads the checked-in
  snapshot `registry/litellm-catalogue.json`, never the live cache, and it never touches the network.
  Refresh the snapshot with the command in its `comment` (it now keeps `context_length`), then run
  `render_registry.py --write`. The catalogue does not say which public-only access group serves a
  public group, so the estate's naming stands in: a public group on route `<r>` is listed for a key
  holding an access group named `<r>-...` (`openrouter-free-public` for `openrouter/*`,
  `google-free-public` for `public/*`).

A missing or stale catalogue is a warning, never a failed run: the gateway lists unchecked when
it has none and still uses a stale one (older than two days) with a warning.
