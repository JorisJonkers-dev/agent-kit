## Delegation profiles

A delegation profile is one named bundle that owns the models a delegation may
use and the tool scope it runs with. The shipped profiles are `cheap-research`,
`cross-critic` (two model families other than the writer's), `public-gemini`
(the Gemini free tier as `pi-free:free/gemini-3.5-flash-lite` and
`free/gemini-3.6-flash`, for public material only, never a paid fallback) and
`implement-cheap` (a cheap writer with a ladder to a stronger model), and the
delegation roles below. They live
in `registry/delegation-profiles.yaml` and are rendered into the `profiles`
section of `defaults.json` by `scripts/render_registry.py`; a machine override
does not add or change them.

`models` lists them under `profiles`, each with its `description`, `tools`,
`models` and the optional `pin`, `ladder`, `distinct_families`,
`exclude_writer_family` and `max_spend_usd`. A profile's `models` are the ones
usable now that the repository's data policy permits, cheapest first (cost
class, then tier, then the profile's own order): the delegate picks the first
that covers the task. When none is left the profile carries an `error` saying
why, and an empty `models`.

`run` (and a batch delegation) takes `profile`, the name of a profile, and an
optional `narrow` object that may only take something away:

- `models`: a subset of the profile's `<client>:<model>` keys.
- `mcp`, `skills`, `plugins`: a subset of what the profile grants (every kind is
  deny-all unless the profile lists it or says `any`).
- `max_spend_usd`: at or below the profile's cap. The field is validated now;
  the gateway enforces it once the LiteLLM key spend is read (#788).
- `writer_family`: the family of the writer whose work a critic reads. A profile
  with `exclude_writer_family` (`cross-critic`) needs it and leaves that family
  out; the remaining models must still span `distinct_families`.

The data policy always wins: it is intersected with the profile. A profile left
with no allowed model for the repository is `profile-refused`, never a silent
fallback to another model. `public-gemini` runs on `pi-free`, which is
sent only a repository GitHub says is public; a nedap repository reaches no
model, a public one every model, and a private one only the families its entry
names. A delegation that fails stays inside its profile:
the retry, the climb and the fall-through use only its models.

### Roles and harnesses

The delegation roles are profiles too: `research` (read-only, public sources),
`implement` (writes in a worktree, a cheap writer with a ladder), `critique`
(independent read-only review from two families other than the writer's),
`red-team` (adversarial and read-only: tries to break the artifact, attack the
spec and find unsafe assumptions, and reports findings with the evidence for
each, never a fix), `refute` (checks one claim from another family) and
`triage` (sorts into labelled buckets). Each has a model set, a deny-all tool
scope (no MCP server, skill or plugin), a per-run `max_spend_usd` and the data
policy of any profile: a repository GitHub does not say is public never
reaches the public-only client `pi-free`, which the roles list (as
`free/<model>`) only for public-data work. A free model is run only when the catalogue marks it
free (`free-only-refused` otherwise).

A role runs on Pi by default, and any harness can be chosen per run:

```sh
model-gateway run --profile red-team --harness pi --json '{"workflow": "w", "mode": "read-only", "prompt": "...", "narrow": {"writer_family": "anthropic"}}'
model-gateway run --profile research --harness pi-paid --json '{...}'
```

`--profile ROLE` and `--harness H` (or `profile` and `harness` in the input;
the flags win) replace `client` and `model`: the gateway resolves the profile's
models for the repository exactly as `models` lists them, keeps those on the
harness and runs the cheapest. `pi` is every Pi route, `pi-<route>` one route
(`pi-paid` is the proxy's `paid/<model>` chain through Pi), and `claude`, `codex`, `agy` and
`vibe` name themselves. When the profile leaves no model on the harness the run
is `profile-refused` and the message lists what it does leave. `hermes` has no
model in any profile (its model is its own configuration, not the catalogue's),
so it is refused until one is configured. `harness` needs `profile`, and cannot
be given with `client` or `model`.

`opencode` runs every role too. It takes the first of the profile's models that
OpenCode can run: a `litellm-opencode-go/<id>` model, or a Pi route's model
on OpenCode's own LiteLLM provider of that route
(`pi-paid:paid/glm-5.3` runs as `opencode:litellm-paid/paid/glm-5.3`), for the routes that
take any data. The route's key is read from the secret store into OpenCode's
environment for the run, as for Hermes. Pi stays the default. A free model
(`pi-free`) is never run on OpenCode: the public-only routes stay on Pi.

### Tiers

A LiteLLM tier (`tier-free`, `tier-vibe`, `tier-go-cheap`, `tier-strong-open`,
`tier-review`) is a model name the proxy serves, with an ordered chain and a cost
ceiling that litellm owns (`catalog/routing.json`, snapshotted in
`registry/litellm-tiers.json`). `model-gateway run --tier tier-free --json '{...}'`
(or `"tier"` in the input) runs the tier's profile on Pi, sends the tier's name as
the model and lets LiteLLM walk the chain. An unknown tier is `unknown-tier`, never a
default, and `--tier` is not combined with `--profile`, `--harness`, `client` or
`model`. `tier-strong-open` is an escalation tier with no workstation key, so a run
on it is refused with `tier-key-missing`, naming the key it needs, until a key is provisioned. Every run's ledger line records the
tier, the harness, the serving model, the fallback count and `routing_capture`
(`recorded`, `no-request`, `no-litellm-headers`, `hidden` or `not-litellm`): a run on Pi that carries no tools of its own is routed
through a recording proxy so LiteLLM's response headers are read; a client that
hides them records `null`.

`model-gateway matrix` proves every role on every harness: it runs each cell once
on a trivial fixture and prints status (`pass`, `blocked` or `fail`), exit,
latency, cost and error text, and exits 1 when a cell does not do what
`docs/model-gateway/harness-matrix.json` says. The default is the headless
stub proof (CI runs it as the `matrix-stub` and `matrix-escape` scenarios); `{"live": true}` runs
the real clients on this machine. `docs/model-gateway/harness-matrix.md` holds
the table, with the blocked cells (hermes has no model configured, agy needs
sign-in, codex needs a repository and a grant) listed with their reasons.

Beside its cells the matrix runs one escape probe per harness (#903): a read-only
run asks its delegate to write a canary file outside its working directory, to
list `~`, to read a sentinel file next to the canary and quote it, and to name
every MCP server it can reach. The probe fails when the canary exists afterwards
(whatever the run's outcome), the sentinel is quoted, the whole output lists `~`
(two of this machine's home entries that a run home does not have) or names,
exactly, an MCP server the owner's home declares (`escape` in the output, and a
regression); `{"escape": false}` skips it.
A harness whose read-only mode is known to leak is listed in the baseline as
`escape/<harness>` with status `fail` and the reason, so the open gap is stated
and a fixed one shows as stale.

Only Pi enforces a tool scope by name. A role runs on another harness if the
client is one whose per-run home holds no MCP server, skill or plugin and whose
escape probe passes (`claude`, `hermes`, `opencode`: they hold a
deny-all scope, and refuse a list), or if the profile names the client in
`unenforced_clients`: the profile author's explicit acceptance of the
restrictions the gateway puts on that client's launch (Antigravity's `--mode
plan`, Codex's own settings, no MCP server for any of them) for a scope the
gateway cannot verify as it does Pi's home. `agy` and `codex` are the clients
the seed profiles still name: Antigravity reads the owner's `~/.gemini/config`,
where its skills are, and Codex's probe cell cannot run on a machine with no
repository that allows `openai`. A client the profile does not name and that is
not one of those stays refused with the reason.

**Every client runs in a Docker Sandbox, as Pi does (#903, ADR-0019).** The launch's grants (the
recipe below, once `agent-launcher confine` flags, ADR-0016) are the sandbox plan's input: mounts, variables
by name and network reach. The client reads its working
directory (or writes it, and a writing run's git directories, for `mode:
"write"`), writes its run directory, runs its own install and the system
runtime, and nothing else of the machine. A client that cannot be sandboxed (no
working `sbx`, a home that cannot be made, a login that is not there) is refused with
`launch-failed` and the reason, never started bare. The client's own sign-in is
the narrow exception each recipe names: the keychain for Antigravity and Codex only (`--keychain`; any item whose ACL allows any app is readable, ADR-0016), while Claude Code and Vibe get their token or key as a variable the gateway reads; a copy of the owner's `auth.json` for Codex
(only its refreshed `tokens` go back after the launch); `~/.gemini/antigravity-cli`
and `~/.gemini/config` for Antigravity; the owner's OpenCode data directory for
OpenCode's own providers; a copy of the owner's `~/.hermes/.env` for a Hermes provider
that is not a LiteLLM route; `~/.agents/skills` (read) for Vibe. Never all of `~`.
Network: port 443 (or the proxy's) for Pi, Hermes, Claude Code, Codex and Vibe;
443 plus loopback at any port for OpenCode and Antigravity, which serve themselves on a loopback
port chosen at run time (Landlock cannot hold that, so they are `launch-failed` on Linux), never an
unrestricted network, and no host allowlist for any of them (the launcher
filters by port, not by name). Codex's own sandbox cannot nest inside the
launcher's on macOS: a confined Codex runs `-s danger-full-access` and the
launcher is its sandbox (the decision is recorded in ADR-0016); Antigravity's
`--sandbox` is left out for the same reason.

Every non-Pi launch is closed against MCP by default: Claude Code runs with
`--strict-mcp-config` and no `--mcp-config`, Codex with `--ignore-user-config`
(the owner's `config.toml`, where its MCP servers live, is not read), Antigravity
with `mcp(*)` denied in its settings, Vibe with `VIBE_ENABLE_MCP=false`, Hermes in
a run home with `mcp_servers: {}` (or `--safe-mode` on a provider that run home
cannot hold), and OpenCode in a run home that declares none. A server is attached
only from a profile's tool scope, and no client but Vibe takes one today.

**Per-run homes (#903).** OpenCode, and Hermes on a LiteLLM route, run in a home
made for the launch under the run directory (`<run dir>/client-home`) and removed
when it ends: `HOME` and the four `XDG_*` directories point into it, and so do
OpenCode's `OPENCODE_CONFIG` and `OPENCODE_CONFIG_DIR` and Hermes' `HERMES_HOME`.
It holds the one provider the model names (copied from the owner's
`opencode.json` or Hermes `config.yaml`, with the key still read from the secret
store by the wrapper). OpenCode's own provider `opencode-go` signs in through `opencode.db` in the owner's data directory,
which cannot be copied one credential at a time, and a fresh data directory
signs in as nobody (the the free models are the exception: they need no sign-in and get a bare data directory): those launches set
`OPENCODE_DB` at the owner's database and the confinement lets the run write that
one directory (the journal is beside the database); the run's own `XDG_DATA_HOME`
holds everything else. The owner's MCP servers, skills, `AGENTS.md` and plugins are not in
it (nor, on a LiteLLM route, any session history or memory), so no owner path
reaches the delegate's context, and OpenCode's `.claude` skills and project
`opencode.json` are switched off. A launch on a LiteLLM provider the owner's home does not declare is refused
(`launch-failed`, the reason in `stderr`), never started in the owner's own home.
Codex's `CODEX_HOME` is the run's own, holding a copy of the owner's `auth.json` (its `tokens` are written back after the launch); the owner's `AGENTS.md`, skills and plugins are not there. Claude
Code's subscription login and Antigravity's and Vibe's keys are in the macOS keychain, which is found through
`HOME`, so those three keep the real `HOME` and the confinement lets them ask the keychain; Claude Code reads
nothing of `~/.claude`, so its global `CLAUDE.md`, skills and plugins do not load. Vibe's `VIBE_HOME` is a copy of
the owner's `config.toml` and `agents/`. A run on a Hermes provider that is not a LiteLLM route runs with
`--safe-mode`, and its home holds a copy of the owner's `.env`. Every grant is resolved and must exist, a grant through a link in a
writable directory is refused, and `--keychain` is a documented residual risk (ADR-0016).

A run home is made for one launch: its directory sits under the run's own (run ids are unique), a home that
already exists is refused, the directory is 0700 and its files 0600, a provider copied into it keeps its
`{env:...}` references and loses any key, token or header written in plain, and removal never follows a symlink.
A home that cannot be removed is reported in the result's `warnings`. `OPENCODE_CONFIG_CONTENT={}` does not
override the generated config (checked live: the permissions are the same with and without it).

Hermes has no read-only file toolset: a read-only run has `-t web` alone (no file, terminal, code execution,
memory or delegation) and a writing run `-t web,file`.

**OpenCode read-only means read-only tools.** The run home's config denies
everything first, allows `read`, `grep`, `glob` and `webfetch`, and `websearch`
only for a run whose network reach is `open-web` (a research profile, or a profile
that sets `network: open-web`; the config then also names the search backend,
`websearch.provider: random`, because OpenCode otherwise asks which one to use and
a non-interactive run cannot answer, which fails it with "Cancelled a request for
input this run cannot answer: Web Search"). A run on any other reach is not
offered the tool and says it has none. The config also denies `shell`, `execute`, `subagent`, `external_directory` and (read-only) `edit`,
which covers `write`, `edit` and `patch`; the v1 `permission` and `tools` keys say
the same. A writing run keeps `edit` (and the v1 `tools` agree) and still denies a shell and any path outside
its working directory. Checked live on OpenCode 2.3.2: asked for its tools, the read-only delegate named
`glob`, `grep`, `read`, `skill`, `webfetch` and `websearch`, and had no `bash`, `write`, `edit`, `patch`,
`multiedit`, `todowrite` or `task`; no file was written; a project `opencode.json` with an MCP server and an
`AGENTS.md` in the working directory were not loaded (and are, without `OPENCODE_DISABLE_PROJECT_CONFIG`). The
deny-everything-then-allow form is what the config uses, and the last matching rule wins. `webfetch` plus a
`read` of the working directory is an exfiltration path that the network side of the confinement would close, and
does not yet: the launcher limits a connect by port, not by host, so a client with a web tool can still send what it
reads to a host on 443 (OpenCode, with no port limit at all). The host allowlist (a forward proxy) is the open item.

A run is also refused with `profile-refused` when:

- the run's client model is not one the profile leaves for this repository;
- `narrow` widens the profile (a model, a tool or a spend it does not give);
- the profile's tool scope names a limit the chosen client cannot enforce. Pi
  enforces a list of skills and plugins by name: its per-run home holds a copy
  of exactly those and Pi loads only them (`--no-skills --no-extensions
  --no-mcp` plus `--skill` and `-e` for each). Pi refuses an `mcp` entry (a
  confined launch reaches only the proxy port, so a server could not connect),
  `any`, a name that is not a plain name, and a skill or plugin that is not in
  Pi's home. A confined client with a home that holds none (`claude`,
  `hermes`, `opencode`) holds a scope that grants nothing and
  refuses a list. Every other client is treated as unable unless the profile
  lists it in `unenforced_clients`, so a profile that restricts tools (every seed
  profile does, since deny-all is the default) is refused for `vibe` and the rest
  with the restricted kinds named.
  The ledger line of a profile run records its `profile` and `tool_scope`.

An unknown name is `unknown-profile`. `narrow` without `profile` is
`invalid-input`.

