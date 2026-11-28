## The `models` command

`models` lists the client models on this machine that the data policy lets a
delegation from a repository reach. It prints exactly one JSON object on stdout
and exits 0; on an error it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
model-gateway models --json '{"repo": "/path/to/repo"}'
model-gateway models --input <file.json>
```

`repo` is the path the caller works in; the gateway asks git for that path's
repository root and its remote. The result carries one entry per permitted
client model with its client, model id, family, cost class, difficulty tier,
capability notes, source, `may_send` (`code` and `history`) and `inline_only`,
plus a `missing` list naming each client the gateway cannot use and why, a
`skipped` list naming each model it cannot offer and why, and a `policy` line
saying why the data policy listed what it did. A client counts as installed
when its binary is on `PATH`; one that is not is named in `missing` whatever
its model source. Models are sorted by client, then model id.

`source` says where the list came from:

- `live` — the client's own listing: Antigravity's `agy models` (a
  `Fetching available models...` line, then one `<id>\t<display name>` per
  model), OpenCode's `opencode models` (one `<provider>/<model>` per line), and
  Vibe's own answer, which the gateway reads by probing it: it runs
  `vibe --model model-gateway-probe --max-turns 1 --max-tokens 1
  --max-price 0 -p ping` — a model id reserved for the gateway, never a model
  — which Vibe refuses before making any request, and it reads the aliases
  from the `Available:` line of that refusal. The caps leave the probe unable
  to spend anything even were its id ever accepted as a model: one turn, one
  token, no price. The connection warnings Vibe prints first are tolerated;
  the answer's terminal colours are stripped, and every id it names must be
  one id — letters, digits, dots, underscores and hyphens, starting with a
  letter or digit — so text Vibe never named as a model is never offered; an
  answer that names nothing available lists nothing. The probe costs one Vibe
  start, so its answer is kept in the cache —
  `$XDG_CACHE_HOME/model-gateway/vibe-available.json`, else
  `~/.cache/model-gateway/vibe-available.json` — for ten minutes, keyed by
  the resolved vibe binary's path and mtime and the VIBE_HOME `config.toml`
  mtime, so an upgrade of either re-probes; a probe that failed is never
  kept. Vibe's built-in aliases come from Vibe's own
  catalogue, so `$VIBE_HOME/config.toml` never decides what is offered: it is
  read only to join each accepted alias to the model name the configuration
  gives it, which the catalogue may know the model by. Only the families a
  client may reach are offered, so Antigravity's Claude and GPT-OSS models are
  left out. An OpenCode model takes the cost class of its provider: the shipped
  defaults count OpenCode Go, the owner's subscription, as subsidised, and
  every other provider as metered-personal.
- `config` — the models the client's own configuration declares. Hermes has no
  non-interactive listing — `hermes model` is an interactive picker — so the
  gateway reads `hermes config get model --json` and offers the one model that
  names as its default, with the provider it runs on. The other models Hermes
  could reach are not listed.
- `declared` — the shipped catalogue, for Codex and Claude Code.

A listed model takes its family, tier and notes from the catalogue entry that
matches its id. A model the catalogue does not know is still listed, with its
family worked out from its id, `"tier": null` and no notes: the `delegate`
skill must not pick a null-tier model, because nobody has judged what it
handles well. A model whose family the gateway cannot work out from its id is
placed from the live LiteLLM catalogue instead (see "The live LiteLLM
catalogue"): any usable chat model the catalogue knows is listed, so
`defaults.json` entries are overrides, not a precondition. Only a model the
catalogue does not know either is left out: it is named in `skipped` with the
reason, so it never vanishes silently, and adding it to the catalogue in
`defaults.json` or the machine override brings it back.

Every listed model also carries `cost` and `providers`. `cost` is
`{"class": <cost_class>, "input_per_mtok": n, "output_per_mtok": n}`: a free
model says `0` explicitly, a model with a price in the defaults (or, for a Pi
model, a price on the model one of its deployments serves) says it in dollars
per million tokens, and a model nobody prices says `"unknown"` for each price,
never a guess. `providers` lists each provider that serves the model in
fallback order, as `{"provider", "upstream_id", "route"}`, where `route` is the
LiteLLM route that reaches it: for a Pi model, the canonical group's
deployments in the catalogue (empty when there is no catalogue or it does not
list the group); for any other client, the one provider the client runs it on.

A listing that fails, or a configuration that cannot be read, leaves that
client out and names it in `missing` with the reason; the other clients are
still listed. A Hermes configuration that names no model is such a failure. A
failed Vibe probe — the command cannot run, it times out, or it answers
nothing the gateway can read — offers no Vibe model at all, not even the
catalogue's entries, because the probe is the only measure of what Vibe
accepts: a run on a model it never named would be refused before it starts,
so the gateway never falls back to the catalogue. A Vibe home whose
`config.toml` declares the probe id itself as a model is such a failure too:
the probe could run that model instead of being refused, so nothing is
offered and nothing is launched until the alias is renamed or removed. The
other way round, a probe that does read names every alias Vibe accepts, and
that is what Vibe offers: a catalogue entry whose alias Vibe does not accept is
never offered, and is named in `skipped` with the reason, whether Vibe dropped
the model or the catalogue outlived it. The entry keeps recording the model's
prices until it is removed from `defaults.json`.

### Pi: one client per LiteLLM route

Pi is the preferred client: `pi-vibe`, `pi-vibe-study`, `pi-opencode-go`,
`pi-local`, `pi-free`, `pi-tier-vibe`,
`pi-tier-go-cheap` and `pi-tier-review`, one per LiteLLM route (`registry/generated/litellm/routes.json`),
all started by the one `pi` program, each on its route's key, which sits in the
route's own team and budget. `pi-free` is the free-only client (below). A tier client sends the tier's name as the model. All are
`subsidised`, except a model that is `free` (below).

**One definition of free.** A model is `free` when the live LiteLLM catalogue
marks it free: for a Pi model, its canonical group, free only when every
deployment of the group, fallbacks included, is a model its provider marks free
(a free tier that still bills once its quota is gone is not). Setup writes the
same flag into Pi's `models.json` as a `cost` of 0 in every field, for every
catalogue-free model; where the catalogue cannot be read or has no word on a
model, that zero price (every field a number 0; a missing, empty, partial or
non-numeric `cost` is not free) stands in. A catalogue that calls the model
billed always wins over a price in the file. No configuration or ledger file
can declare `free`. `models` prints a free model with `"free": true` and the
cost class `free` (ahead of `subsidised`), and `models --free` (or
`{"free": true}` in the input) lists only the free ones. A free model is held to
the caps and the plan limit exactly like a `subsidised` one (neither applies).
Each lists the models the home's
`~/.pi/agent/models.json` (or `PI_CODING_AGENT_DIR`) gives its
`litellm-<route>` provider, which setup writes from the live catalogue's
canonical groups the route's key reaches, so `models` shows what a Pi client
can run now. A model's family comes from its id (a few more prefixes than the
other clients know: `ministral`, `magistral`, `voxtral`, `gemma`, `mimo`), and
its tier and notes from the defaults entries with `"client": "pi"`, which
describe a model once for every `pi-<route>` client; a canonical model with no
entry is listed with `"tier": null` and is never picked. A client whose
`models.json` is missing, is not JSON or has no such provider is named in
`missing` with the reason. Setup writes a provider only for a route that has a
usable model (the `local` route has none until a local model is probed), so where
it wrote the other routes, `missing` says the route has none instead of asking
to re-run setup.

The live catalogue judges a Pi model by its canonical group, not by a route:
the group is offered when one of its deployments is usable, is named in
`skipped` with the provider's own reason when every deployment the catalogue
probed is blocked or failed, is marked `"free": true` when it is public or every
deployment tried first is a free model, and is offered unchecked when the
catalogue probed none of them.

**The free client.** A free model is reached in one way only: as
`free/<model>` on `pi-free`, the `free` route, whose key holds the access group
`free` and nothing else (litellm#106). Each `free/<model>` group is every free
copy of one model across the free providers; the proxy checks each provider's
limits before the call, skips a provider that is limited and returns it at its
reset, and never falls back to a paid deployment, so a request every free copy
refuses fails instead of billing. No other client, provider or name reaches a
free model: the old per-provider names (`openrouter/<x>`, `public/<x>`,
`google-free/<x>`, `opencode-go-free/<x>`) and the
`pi-public`, `pi-openrouter` and `opencode-free` clients are gone. `pi-free` holds
its models to the one definition of free: `models` lists a free model and names
a billed or unpriced one in `skipped` with the reason, and `run` refuses a billed
one with `free-only-refused` before anything starts. Every client refuses
OpenRouter: a model named `openrouter/...`, or a Hermes or OpenCode run on the
`openrouter` provider, is `provider-not-allowed`, and no machine file opens it.

`pi-free` and `pi-tier-review` have `"data_scope": "public"` (a client field,
`any` by default): a free provider may train on what it is sent, so those routes
are for public work only, so a delegation that reads a repository is
sent to it only when GitHub says the repository is public. A public repository
is already public, so the route may read it: a read-only run and a writing run
alike, carrying its code and history. A private repository, one whose
visibility cannot be read and a `nedap` one are refused, whatever an allowlist
entry lists; `models` with a `repo` follows the same rule, listing the
client's models for a public repository and leaving them out for any other. With no `repo` it lists as before, since such a
run carries only what the caller declares.

A Pi run starts in a Docker Sandbox, never bare: Pi has no
sandbox of its own and its tools run inside its own process. The launch's own
agent directory (`<run dir>/<client>/pi-agent`, `PI_CODING_AGENT_DIR` and
`TMPDIR` for the launch) holds only the route's provider with its key, read from
the secret store just now and written 0600, and is removed when the launch ends.
A launch that cannot be sandboxed (no provider for the route in `models.json`,
a key the store cannot give, no working `sbx`) never starts: the run is
`launch-failed`, its `stderr.txt` names the reason, and no key is ever in a
message. See "The `run` command".

### The live LiteLLM catalogue

A client with a `litellm_route` (`vibe` on `vibe`, `opencode` on
`opencode-go`) has its listed models judged by the live LiteLLM catalogue: the
probes the litellm repository's sync publishes, which setup fetches to
`$XDG_CACHE_HOME/agent-kit/litellm-catalogue.json` (else
`~/.cache/agent-kit/litellm-catalogue.json`). A model is looked up by its id
and every name its defaults entry gives it, without the route prefix. Only a
model the catalogue calls `usable` is offered; one it calls `blocked` is named
in `skipped` with the provider's own reason, and any other status (`error`,
`budget-exhausted`, `unprobed`, ...) is skipped with that status. A model the
catalogue marks free carries `"free": true`. A model the catalogue does not
list is offered as before, on the client's own answer.

A model with no `defaults.json` entry whose id names no known family is placed
from the catalogue, for a Pi client by its canonical group and for another
client by its route's listing: its family is the one the gateway already names
by the id's prefix, else the vendor of its canonical id
(`reflection/beam-1.0-501b-a23b` is family `reflection`), else the id's first
word; its tier is a guess from the name (`routine`; `trivial` for a name with
`lite`, `mini`, `nano`, `tiny`, `micro` or `small`; `hard` for `pro`, `large`,
`ultra`, `super`, `max` or `plus`, never above `hard`), and its notes say so; a
`defaults.json` entry overrides all three. Only chat models are placed: one the
catalogue lists under another modality is named in `skipped` with the
catalogue's own reason, and a model it calls blocked or failed is refused as
before. `run` places a model the same way, so `run` accepts a model that
`models` lists. A Pi run names the canonical group (`free/beam-1.0-501b-a23b`),
not the upstream id it is served under (`beam-free`); the refusal for the upstream
id names the group.

A missing, unreadable or malformed catalogue is never an error: the models
are listed unchecked and the result carries `warnings` saying so. A catalogue
older than two days is still used, with a warning that it is stale. A run on a
model the catalogue refuses is refused before anything is launched, code
`model-not-usable`, with the same reason.

Any other subcommand answers `{"status":"error","code":"unknown-command",...}`.

