# LiteLLM: one proxy, per-key budgets

The cheap agents (Pi, Vibe, OpenCode) reach their models (the paid ones, the
free ones for public work as `free/<model>`, and the local GPU) through one LiteLLM proxy in the estate cluster, shared by both machines, so one budget per
key holds across machines (spec #392, stories 96-98). Gold Band's own cost and
token numbers are never used; the proxy's refusal is the only budget signal.

## The keys and their caps

| Agent id | Virtual key (secret store) | Max budget, per 30 days | Access groups (`models`) | Reaches |
|---|---|---|---|---|
| `vibe` | `LITELLM_VIBE_KEY` | 255 EUR | `canonical` | every canonical model |
| `vibe-study` | `LITELLM_VIBE_STUDY_KEY` | 127.50 EUR | `canonical` | every canonical model |
| `opencode-go` | `LITELLM_OPENCODE_GO_KEY` | 60 EUR | `canonical` | every canonical model |
| `free` | `LITELLM_FREE_KEY` | none | `free`, `tier-free` | every free model, only as `free/<model>` (each group every free copy of one model across the free providers), and the `tier-free` tier name; never `canonical`, so nothing paid is reachable and a refusal fails instead of billing; **public work only** |
| `local` | `LITELLM_LOCAL_KEY` | none | `canonical`, `local/*` | every canonical model, and `local/<id>` for any id the GPU workstation's server (free) serves, such as `local/qwen3:32b`; the key has no cap, so use it for local models only |
| `tier-vibe` | `LITELLM_TIER_VIBE_KEY` | none | `tier-vibe`, `vibe-tier`, `opencode-go-tier`, `paid` | the `tier-vibe` chain: Vibe's Flash models, then Go's cheap ones; and `paid/<model>` |
| `tier-go-cheap` | `LITELLM_TIER_GO_CHEAP_KEY` | none | `tier-go-cheap`, `opencode-go-tier`, `paid` | the `tier-go-cheap` chain: Go's cheap models; and `paid/<model>` |
| `tier-review` | `LITELLM_TIER_REVIEW_KEY` | none | `free`, `tier-review`, `vibe-tier`, `opencode-go-tier` | the `tier-review` chain: free models, then Vibe Flash, then Go Flash; **public work only** |
| `paid` | `LITELLM_PAID_KEY` | none | `paid` | `paid/<model>` only (each a budgeted paid group, see the paid groups in AGENTS.md of litellm); a dedicated key for callers that name a paid model explicitly |

A key lists **access groups**, not a provider wildcard (the free `local` key alone adds its own
`local/*`: LiteLLM puts a wildcard in an access group only with an Enterprise licence, so the key
names it): a caller sends one canonical model
name (`glm-5.3`, `mistral-large`, `claude-sonnet-4-5`, ...) and the proxy routes it across the
provider accounts by priority (litellm#20), so the agent id no longer picks the provider. Every paid
deployment, and the local one, is in `canonical`. The free deployments are not: they sit only in
the `free/<model>` groups, in the public-only access group `free` (litellm#106), and only the
`data_scope: public` keys hold it. Which models an account can actually use is
the live catalogue's job (below); a listed model can still be refused.

`registry/estate-tooling.yaml` (`litellm:`) is the source. The render writes
`registry/generated/litellm/routes.json` (base URL, header, one route per agent id with its
key variable, cap (`null` is no cap), `budget_duration`, `data_scope` and access groups, never a
key value), the team each key goes into (in `registry/generated/workstation-secrets.json`) and
`registry/generated/litellm/make-admin-token.sh` (the owner's one-hour management key, below). LiteLLM counts `max_budget` in its own cost unit (USD): the number is entered as the
cap, and it is the owner's to rescale if the exchange rate matters.

**The budget resets.** Every capped route's team carries `budget_duration: 30d`, so a cap is a monthly
allowance that LiteLLM resets, not a lifetime cap that runs out for good. The render refuses a
capped route without it. The caps on the keys are a safety net; each provider account carries its
own cap, in litellm's `catalog/routing.json` `accounts`:

| Account | Cap | Window |
|---|---|---|
| `vibe` | 255 | calendar month, from the 1st |
| `vibe-study` | 127 | calendar month, from the 1st |
| `vibe-api` | 25.5 | calendar month, from the 1st |
| `vibe-study-api` | 12.7 | calendar month, from the 1st |
| `opencode-go` | 40 USD | 30-day cycle |

The gateway holds the same numbers as the defaults of `settings.quota.accounts`, and reads them
when it chooses a tier ([quota-tiers.md](model-gateway/quota-tiers.md)).

### The public-only keys: public, non-sensitive work only

A free provider may use the prompts to train or improve its models, so every free deployment is
**public work only** (agent-kit#737). The litellm repository serves a free model in one way only
(litellm#106): the group `free/<model>`, which holds every free copy of that model across the free
providers (OpenRouter, the Google free tier, Groq, Cloudflare, NVIDIA, Z.ai, Kilo, OpenCode Go's
free ids and the rest), in the access group `free` alone, never in `canonical`:

| Public-only group | What is in it | Model names | Held by |
|---|---|---|---|
| `free` | every free copy of each model, one group per model, no paid deployment | `free/<model>` | `free`, `tier-review` |

A name carries no free suffix (`free/beam-1.0-501b-a23b`, never `-free` or `:free`), and only chat
models are deployed. The proxy checks each provider's limits before the call, skips a provider that
is limited and lets it back in at its reset. A request every free copy refuses (quota, cooldown, a
4xx) is a failure, never a paid retry. A model the account blocks, such as an OpenRouter free model
behind the account's training-data setting, is not usable, so it is not deployed. The `free` and
`tier-review` routes are `data_scope: public`.

No other name reaches a free model. The old per-provider names and access groups
(`openrouter/<x>` in `openrouter-free-public`, `public/<x>` in `google-free-public` with its paid
fallbacks, `google-free/<x>`, `opencode-go-free/<x>` in `opencode-go-free-public`, and each
`<route>-free-public`) are gone, and so are the `openrouter` and `public` keys
(`LITELLM_OPENROUTER_KEY`, `LITELLM_PUBLIC_KEY`): setup removes both from the secret store. The Pi
provider of the `free` key lists only `free/<model>` names and `tier-free`, from the catalogue
snapshot's `free/` groups.

The render enforces the keys' side (`scripts/render_registry.py`, tested in
`tests/test_registry_render.py`): a route holding `free` must be `data_scope: public`, a
`data_scope: public` route must hold it, and a route that names a retired `<x>-free-public`
group is refused. A public group is listed on a Pi provider only under its `free/` name.

**Use a public-only key for public, non-sensitive work only.** Use it for open-source code,
public documentation and questions you would post publicly. Never send it a private repository,
employer code (`nedap`), a secret, a transcript, a customer's data or anything under an NDA, and
never route a critic on a private repository to one. When unsure, use a `canonical` key: a model a
paid provider serves is reachable there too.

The critique runner enforces the last rule rather than leaving it to care (#737). Every Pi critic
client in `skills/workstation/critique/routing.json` names its route's `data_scope` (`any` or
`public`; `tests/test_critique_setup.py` checks it against `routes.json`). A client with
`data_scope: public` is routed only to a repository GitHub says is public (`gh api
repos/<owner>/<repo> --jq .visibility`); a public repository is not held to the critique allowlist
at all. A private repository, one whose visibility cannot be read and a `nedap` one never are. For
any other repository
the roster drops it with the reason `data-scope`, it is never kept as a fallback, and a refute round
passes it over. The check lives in `scripts/crates/critique`, each
with tests.

OpenCode Zen's own free models (`big-pickle`, `glm-5.3-flash-free` and the rest of Zen's `-free`
ids) are not used: OpenCode serves them to the OpenCode client alone, outside the proxy, and the
model gateway refuses the `opencode/` provider on every client. The free ids OpenCode Go serves are
in the `free/<model>` groups (item 7 below).

### Provider priority (what the keys route to)

Set in the litellm repository (litellm#20); recorded here because the agent ids depend on it.

1. A model Vibe serves (including GLM 5.3, GLM 5.3 Flash, DeepSeek V4.1 Flash): `vibe` and
   `vibe-study` at 2:1, then stop. **No other provider serves a model Vibe serves**, OpenCode
   Go and the general Mistral keys included (litellm#49: once the plans are out, the request
   fails instead of spending the general keys).
2. Models only the general Mistral keys serve (audio, OCR, embeddings and the rest of the broad
   list): `vibe-api` and `vibe-study-api` at 2:1. They are the last resort: a model any other
   route serves is never deployed on them, and a caller reaches them otherwise only by naming
   `vibe-api/<id>`.
3. Claude only through the `anthropic` key.
4. Gemini: the Google free tier only inside the `free/<model>` groups, with no paid fallback.
5. OpenCode Go only for models no Mistral, Google or Anthropic key serves, through LiteLLM's
   own `opencode_go` provider (litellm 1.109 and later): it picks the API family (Chat
   Completions, Responses, Messages, Gemini) per model and prices each request, so the spend
   shows under OpenCode Go in the LiteLLM UI. The proxy sets no base for it.
6. OpenRouter free models only, `data_collection: allow` (public work only; most free endpoints
   exist only with training on), only inside the `free/<model>` groups (never in `canonical`).
   The OpenRouter account's privacy setting "Free model training" must be on.
7. OpenCode Go's free ids (`beam-free`, `hy4-flash-free`, ...), whose providers may train on the
   prompt, only inside the `free/<model>` groups under the model's name without the suffix
   (`free/beam-1.0-501b-a23b`), never in `canonical` or a paid group.

Mistral's four accounts: `vibe` (255) and `vibe-study` (127) are Vibe-scope; `vibe-api` (25.5) and
`vibe-study-api` (12.7) are general API keys. Each cap is a calendar month, from the 1st.

### Names the litellm repository must match

`canonical` (every paid and local deployment), the public-only group `free` and its
`free/<model>` names (`LITELLM_PUBLIC_GROUPS` and `LITELLM_FREE_PREFIX` in
`scripts/render_registry.py`),
the tier names and the access groups their `key_models` name (`registry/litellm-tiers.json`), the
agent ids / key aliases above, and the canonical model names the gateway defaults use
(`registry/litellm-catalogue.json` lists what the catalogue serves).

## The tiers

A tier is a callable model name on the proxy, with a cost ceiling and an ordered chain. The litellm
repository owns them (`catalog/routing.json`, key `tier_groups`); `registry/litellm-tiers.json` is a
snapshot of that key, and `ci-helpers litellm-tiers --write` refreshes it.

| Tier | Ceiling | Chain |
|---|---|---|
| `tier-free` | free | `free/hy4-flash`, `free/longcat-2.5`, `free/beam-1.0-501b-a23b`, `free/glm-4.7-flash`, `free/qwen3.8-27b` |
| `tier-vibe` | go | `vibe-tier/deepseek-v4-1-flash-1017`, `vibe-tier/zai-glm-5-3-flash`, `vibe-tier/mistral-small-2603`, then `tier-go-cheap` |
| `tier-go-cheap` | go | `opencode-go-tier/deepseek-v4.1-flash`, `opencode-go-tier/glm-5.3-flash`, `opencode-go-tier/mimo-v2.6-flash` |
| `tier-strong-open` | go | `opencode-go-tier/qwen4-max`, `vibe-tier/zai-glm-5-3`, `opencode-go-tier/kimi-k3.1-pro`, `opencode-go-tier/gpt-6-luna`, then `tier-vibe` |
| `tier-review` | go | `free/nemotron-3-ultra-550b-a55b`, `free/gemma-4-31b-it`, `free/glm-4.7-flash`, `free/gpt-oss-120b`, `vibe-tier/deepseek-v4-1-flash-1017`, `vibe-tier/zai-glm-5-3-flash`, `opencode-go-tier/qwen3.8-flash`, `opencode-go-tier/minimax-m3.1-flash`, `opencode-go-tier/mimo-v2.6-flash` |

A key that calls a tier holds the tier's name beside the access groups its chain needs, and the
render refuses a route whose `models` differ from the tier's `key_models`. `tier-free` is called
with the `free` key. `tier-vibe`, `tier-go-cheap` and `tier-review` have a key each. `tier-review`
starts on free models of several labs, whose providers may train on the prompt, so its key is
`data_scope: public`. `tier-strong-open` has no workstation key: it is an escalation tier, and a run
on it is refused with `tier-key-missing` until a key is provisioned. The paid steps are
per-deployment budgeted copies (`vibe-tier`, `opencode-go-tier`), so a tier key carries no cap of
its own; the proxy's per-deployment budgets bound the spend.

`model-gateway run --tier T` runs a delegation as a tier: it picks the tier's profile and its Pi
route, and Pi sends the tier's name as the model. The gateway drops to a cheaper tier when the plan
the tier draws on is at 80% of a window; [quota-tiers.md](model-gateway/quota-tiers.md) has the
ladder, the signals and the settings.

## The decisions of 2026-11-18

- **Claude and Codex subscriptions stay out of LiteLLM.** No route reaches a subscription. Claude is
  served only through the `anthropic` API key, and no workstation key reaches it: a group with a
  deployment on the `anthropic` route is reached only by a key holding that route's own access
  group (`WITHHELD_ROUTES` in `scripts/litellm_catalogue.py`). Codex has no route at all.
- **The ladder runs free, then Vibe, then Go.** `tier-free`, `tier-vibe`, `tier-go-cheap`,
  `tier-strong-open`, cheapest first.
- **Reviews use free models, then Vibe Flash, then Go Flash.** That is the `tier-review` chain.
- **GLM-5.3, Kimi K3.1 Pro and GPT-6 Luna are opt-in only.** They sit in `tier-strong-open` alone,
  behind their own access group `strong-open-tier`, which only a `tier-strong-open` key holds. No
  other tier reaches them.

The plan behind these decisions is
[free-inference.md](https://github.com/JorisJonkers-dev/litellm/blob/docs/free-providers-and-subscriptions/docs/free-inference.md).

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
any client can send. Pi, Claude Code, Codex, Hermes, OpenCode and Vibe send that one header
(see "Running a harness on a route"); this repository ships the table, the launchers and the gate.

The old sign-in edge's auth-api service token, `LITELLM_EDGE_TOKEN`, is retired: setup removes
it from the secret store, and neither `litellm-gate` nor `measure-litellm-budget` sends a second
header any more (`--edge-token-env` is gone from `litellm-gate`).

## Running a harness on a route

Each harness has one way to run a LiteLLM model with a route key read from the secret store.
`litellm-harness` (the Rust crate `scripts/crates/litellm-harness`) does it, installed as `agent-kit-litellm`; the data it uses is
`registry/generated/litellm/harnesses.json`, rendered from the `litellm` section of the registry
by `scripts/render_registry.py` (the proxy URL and, per route, `litellm-<agent>`, the key
variable, the data scope and the models its access groups reach). Pi has its own providers
(`registry/generated/pi/providers.json`). No file written here holds a key, only the name of the
variable that does.

```bash
agent-kit-litellm run claude   vibe --model glm-5-3 -- -p "hello"
agent-kit-litellm run codex    vibe --model glm-5-3
agent-kit-litellm run hermes   vibe --model glm-5-3
agent-kit-litellm run opencode vibe --model glm-5-3
agent-kit-litellm run vibe     vibe --model glm-5-3
```

`run <harness> <route>` reads `LITELLM_<ROUTE>_KEY` with `agent-kit-secret get`, puts it in the
child's environment and replaces itself with the harness. Arguments after `--` go to the harness.
The key is never in argv and never printed. A missing key, route or model stops the run with
exit 2 before anything starts.

| Harness | Config setup writes | What `run` does |
| --- | --- | --- |
| Claude Code | none | sets `ANTHROPIC_BASE_URL` (the proxy without `/v1`) and `ANTHROPIC_AUTH_TOKEN` for that one process, and drops `ANTHROPIC_API_KEY`; the default profile and its `settings.json` are never changed |
| Codex | `[model_providers.litellm-<route>]` in `~/.codex/config.toml` (`base_url`, `env_key`, `wire_api = "responses"`) | `codex -c model_provider="litellm-<route>"`, key in the route's variable |
| Hermes | `providers.litellm-<route>` in `~/.hermes/config.yaml` (`base_url`, `key_env`, `api_mode`, models) | `hermes chat --provider litellm-<route>` |
| OpenCode | `provider.litellm-<route>` in `opencode.json` (`options.baseURL`, `options.apiKey = "{env:KEY}"`, models) | `opencode -m litellm-<route>/<model>` |
| Vibe | `[[providers]]` and `[[models]]` in `~/.config/agent-kit/vibe-litellm/config.toml` | `VIBE_HOME` set to that home, `VIBE_ACTIVE_MODEL=litellm-<route>/<model>` |

Setup (`setup-workstation.sh`) installs the launcher, then merges the entries into the config of
each harness that is on `PATH` (not in a cloud session; OpenCode 1 is left alone). It owns every
entry named `litellm-*`: it replaces those the registry names, removes the ones whose route is
gone, keeps everything else and keeps a `.bak` of the file it changed.

Vibe is the exception to "the harness's own config": its provider entries go in a home of their
own, and the launcher and the merge both refuse `~/.vibe`, `~/.vibe-private` and any home whose
name contains `private`. Plain `vibe`, `vibe-study` and `vibe-private` do not see the LiteLLM
providers, and a LiteLLM run does not see Shared or Private Memory.

Data scope: the `free` and `tier-review` routes (`data_scope: public`) are for public work only.
Never start a harness on them in a private repository.

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

Setup mints them itself (`mint: litellm` in the registry, done by `agent-kit-keys`).
When a key is missing from the secret store, or LiteLLM's `GET /key/info` refuses it, setup asks
once for a **LiteLLM admin token**: a management key that expires in one hour, which the owner
makes with the proxy's master key:

```bash
registry/generated/litellm/make-admin-token.sh
```

On a terminal the script asks for the master key and does not show what is typed. Without a
terminal, or to skip the question, put the key in `LITELLM_MASTER_KEY` first.

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
no new key. A key minted before that holds its groups itself, so a team change never reaches
it (the old `openrouter` key, minted before its route became public-only, still listed `canonical`,
and every free model answered `403 key_model_access_denied`). Setup reads each stored key's own
`/key/info`, and a key that names anything but `all-team-models` is minted again on a run with a
token (`LITELLM_<ROUTE>_KEY: the key is scoped to ..., not to its team's models; minting it again`).

## The window and reset

Every capped route's team is made with `budget_duration: 30d`, so LiteLLM resets its spend
every 30 days. `measure-litellm-budget` (the Rust crate `scripts/crates/litellm-budget`) confirms it on the real proxy: one `GET /key/info` with
the agent id's key (no model call, no spend), printing only `max_budget`, `budget_duration`,
`budget_reset_at` and `spend` (a minted key's team's, read with a second `GET /team/info`), and
the request with the key redacted. It reads the key from the
environment or the secret store:

```bash
measure-litellm-budget --agent vibe
```

`model-gateway catalog` shows the same pools: each deployment names the key whose pool it draws on
(`pool LITELLM_VIBE_KEY`), and a `budget pool` line per key gives the monthly cap from
`routes.json` and the spend so far. The spend is read by running `measure-litellm-budget --agent
<route>` for a capped route, so it is a number only when that program is installed and the proxy
answers; otherwise it prints `unknown` (`"spend_eur": "unknown"` in `--json`, beside
`monthly_cap_eur`, which is `"none"` for an uncapped key).

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
when a paid key moves tokens and records no spend. The `free/<model>` groups and the
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
- **The model gateway's Pi clients** (`pi-vibe`, `pi-vibe-study`, `pi-opencode-go`, `pi-local`,
  `pi-free`: one per route in `registry/generated/litellm/routes.json`; `pi-free` lists free models only, as `free/<model>`, and is sent a repository only when GitHub says it is public; `pi-local` lists the models the `local` deployment serves, when it is probed usable) list the
  models the home's `~/.pi/agent/models.json` gives the route's `litellm-<route>` provider, which
  setup wrote from `canonical_groups`, and judge each canonical group by its deployments in the live
  catalogue: usable when one deployment is, skipped with the provider's reason when every probed
  deployment is blocked or failed, free when public or every deployment tried first is free. A Pi
  run goes through `agent-launcher confine`. `pi-free` and `pi-tier-review` (`data_scope: public`)
  are never sent a repository GitHub does not say is public.
- **Setup's own writers** (the Pi providers per route) use `scripts/litellm_catalogue.py`:
  `load_catalogue()` returns the live copy, else the snapshot `registry/litellm-catalogue.json`,
  with every problem as a warning; `provider_models(catalogue, provider)` lists one provider
  route's probes; `canonical_groups(catalogue, access_groups)` lists the canonical model names a
  key with those access groups reaches. A public-only group (a free tier) is reached only through
  an access group that only `data_scope: public` routes hold in `routes.json`, or the access
  groups the catalogue names on the group itself, so no group name is fixed in the reader.

- **The Pi provider table** (`registry/generated/pi/providers.json`, rendered by
  `scripts/render_registry.py`) takes each route's models from `canonical_groups()`; no model list is
  kept by hand any more (the old `registry/litellm-access-groups.json` is gone). Only chat groups with
  at least one deployment the catalogue probed `usable` are listed (`usable_group_names()`; a
  deployment's probe is `<route>/<upstream_id>`, and a blocked, error, unknown or not-chat probe does
  not count), so a group with no usable deployment, such as a Gemini transcribe or robotics model,
  is not offered. The `local` route lists only groups with a deployment on the `local` route (its
  `canonical` access group also reaches the cloud models, which the local server does not serve), and
  a `data_scope: public` route lists only public groups. A route left with no model has no
  Pi provider at all, and setup removes one it wrote before. Each Pi model entry carries `contextWindow` (the group's `context_length`: the smallest among
  its first-tried deployments, left out when unknown) and, for a free group, an all-zero `cost`. The
  catalogue holds no price, only whether a model is free, so a paid model has no `cost` entry.
  The render must give the same text on every machine for `--check`, so it reads the checked-in
  snapshot `registry/litellm-catalogue.json`, never the live cache, and it never touches the network.
  Refresh the snapshot with the command in its `comment` (it now keeps `context_length`), then run
  `render_registry.py --write`. A group that names its own access groups in the catalogue is
  reached by exactly those: the `free/<model>` groups name `free`, and the Claude ones name
  `anthropic`, which no workstation key holds. A public group is listed only under its `free/`
  name, so a provider's own name for a free model (`openrouter/<x>`, `public/<x>`) never reaches a
  Pi provider even while the catalogue still lists it.

A missing or stale catalogue is a warning, never a failed run: the gateway lists unchecked when
it has none and still uses a stale one (older than two days) with a warning.
