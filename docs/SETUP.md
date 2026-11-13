# Workstation setup

One command brings a machine to the estate's agent baseline: Claude Code and
Codex at their latest releases, every plugin, the language servers those
plugins drive, and the MCP fleet.

```bash
uv run python scripts/render_registry.py --check   # artifacts current?
./installer/setup-workstation.sh --check           # what would change
./installer/setup-workstation.sh                   # do it
./installer/setup-workstation.sh --no-profiles     # primary Claude profile only
./installer/setup-workstation.sh --cloud           # a cloud environment, see CLOUD.md
```

Run from a terminal, setup **walks you through** what it can fix: it gets
the tokens and keys (below) and offers to load them in every new shell.
`--no-prompt`, `--check`, `--cloud` and runs without a terminal never ask.

Every run ends with **Still to do**: one numbered list of every failure and
warning it raised, failures first, each with what to do about it. An empty
list prints "Nothing left to do." A missing toolchain (no `rustup`, no
`dotnet`) is noted, not listed.

## Setup converges

Each Claude profile, Codex, Vibe and OpenCode ends a run with exactly the MCP servers
the registry declares for it, plus the ones the keep file names
([CONTEXT.md](../CONTEXT.md) defines the terms). Claude's marketplaces and
user-scope plugins converge the same way, once for every profile, because
they share `plugins/` and `settings.json`. A marketplace counts as declared by
the name Claude shows, so the GitHub `claude-plugins-official` is undeclared:
its plugins come from `agent-kit-official`. Setup removes a retired entry
everywhere, and removes any undeclared one unless
`~/.config/agent-kit/keep.yaml` names it (a kept marketplace keeps its
plugins):

```yaml
keep:
  - context7
  - idea
```

The keep file lives outside the repository, so work-only servers never enter
the public registry. A missing file keeps nothing; a malformed one stops the
run before anything is removed. `--check` prints
`would remove <name> (<surface>, <reason>)` for each removal. A normal run
reads every surface back and fails if a removed entry is still there.
Antigravity is the one surface this does not cover: setup merges its single MCP
file and keeps every server it does not name, so nothing is removed there (see
[Antigravity](#antigravity)).

`setup-workstation.sh` is **generated** from
[`registry/estate-tooling.yaml`](../registry/estate-tooling.yaml). Do not edit
it. Edit the registry and re-render — see [REGISTRY.md](REGISTRY.md).

## Credentials

Setup gets them for you first, before anything that needs them
(`scripts/agent-kit-keys.py`, driven by `workstation_secrets:` in the
registry):

1. It signs you in to auth.jorisjonkers.dev once (username or email,
   password, then TOTP) and, as an admin, grants you any missing `MEMORY_*`
   service.
2. It mints one service token per service for this machine, labelled
   `<host> agent-kit`, and checks each against its own host.
3. Mistral Vibe signs in with your own Mistral account (`vibe`, then sign
   in), which keeps its key in the keychain itself. Setup asks for no Mistral
   key: an exported `MISTRAL_API_KEY` would override that login, so a key an
   earlier setup stored is removed from the keychain and `env.sh`.
4. It stores each in the one secret store, `agent-kit-secret`: the macOS
   Keychain as `agent-kit.<VARIABLE>`, or on Linux the Secret Service
   (GNOME Keyring or another libsecret provider; service `agent-kit`, username
   `<VARIABLE>`). There is no plain-file store: with no store, or no key in
   it, the run or launch stops with a message. A `secrets.env` an earlier
   setup wrote is moved into the store and removed. The
   private-bank token is the exception: it is sealed behind a passphrase you
   choose (see Vibe launchers and Private Memory). Secrets travel over stdin, never argv, and
   are never printed.
5. It writes `~/.config/agent-kit/env.sh`, which reads them back, and offers
   to add one line to your `~/.zshrc` or `~/.bashrc` that loads it.

A value already in the environment wins over the stored one. A later run
asks only for what is missing or refused by its host. Optional credentials
(below) never gate registration — the server self-manages auth.

| Variable | Used by | Required? |
|---|---|---|
| `HINDSIGHT_API_TOKEN` | the `memory-api` MCP server, and Hindsight Coding Agents' hooks and `hindsight` tools | required |
| `HINDSIGHT_API_URL` | anything that reads it; Coding Agents takes its URL from its own config | set by env.sh |
| `MEMORY_PRIVATE_TOKEN` | the `memory-private` MCP server (Private Memory), in `vibe-private` only | never exported; sealed behind your passphrase, given to `vibe-private` per launch |
| `GRIMOIRE_API_TOKEN` | the `grimoire` MCP server (Grimoire at grimoire.jorisjonkers.dev) | required for `grimoire`; without it the server is not registered |
| `MISTRAL_STUDY_API_KEY` | `vibe-study`: Vibe signed in with the Study account | optional; you get it with `vibe-study --login` (setup never asks); never exported, read only by `vibe-study` |
| `MISTRAL_API_KEY` | not set on the workstation: Vibe uses your account login. Only cloud sessions pass one in | cloud only |
| `OUTLINE_API_KEY` | the `outline` MCP server (Outline at notes.jorisjonkers.dev) | required for `outline`; without it the server is not registered |
| `OVERLEAF_SESSION` | the `overleaf` MCP server (self-hosted Overleaf) | optional |
| `OPENROUTER_API_KEY` | this host's own OpenRouter key, never exported; read from the store with `agent-kit-secret get OPENROUTER_API_KEY` | optional; one key per host, never shared |

`HINDSIGHT_API_TOKEN` and `MEMORY_PRIVATE_TOKEN` are **per-host, individually
revocable** service tokens, not shared secrets — mint one per device with
`POST /api/v1/auth/service-tokens {"service":"MEMORY_API"|"MEMORY_PRIVATE",
"label":"<device>"}` while signed in to auth-api (agent-kit#41/#42 design),
and revoke a lost device with `DELETE /api/v1/auth/service-tokens/{id}`
without touching any other device or host. Never share one token between
hosts, and never fall back to the account password or a long-lived shared
key — see the [#41 design](https://github.com/JorisJonkers-dev/agent-kit/issues/41)
for why both are explicitly rejected.

`hindsight-api.jorisjonkers.dev` serves only the shared bank `default`: its
single-bank MCP endpoint `/mcp/default/` and the REST paths under
`/v1/default/banks/default/`. Everything else on that host is a 404, including
the multi-bank `/mcp` root.

### Vibe's models

What the model gateway offers of Vibe is decided by the probe's answer, not
by this file: the gateway runs
`vibe --model model-gateway-probe --max-turns 1 --max-tokens 1
--max-price 0 -p ping` — a model id reserved for the gateway, never a model —
which Vibe refuses before making any request, and
lists the aliases named in the `Available:` line of that refusal. Vibe gets
its aliases from its built-in catalogue too, so the answer can name an alias
`config.toml` does not declare, and nothing else can add to or subtract from
what the gateway lists. `${VIBE_HOME}/config.toml` never decides the offer:
it only supplies the name joins, because each accepted alias is joined to the
model name the configuration gives it, which the gateway's catalogue may know
the model by.

Separately from the gateway, `scripts/vibe-sync-models.py` keeps
`config.toml` itself in shape; Vibe's own default is Mistral Medium 3.5. With
the key Vibe signed in with (its keyring item, see below, or `MISTRAL_API_KEY`
in a cloud session), setup reads Mistral's model
list and writes one `[[models]]` entry for every chat model the key can use:
one entry per underlying model, under its `-latest` id where there is one, so
upgrades arrive on their own. Models without tool calling, transcription
models and deprecated ones are left out. A model you define yourself under
the same alias wins, and a model gone upstream is removed on the next run.

### Where Vibe keeps its sign-in

Signing in with `vibe` stores the Mistral key in the OS keyring, under the
service `ai.mistral.vibe` and the account `MISTRAL_API_KEY`. Only when no
keyring takes it does Vibe write a `MISTRAL_API_KEY=` line to `~/.vibe/.env`
(mode 0600) instead. Which keyring that is depends on the OS:

| OS | Where the key is | How agent-kit checks it, without reading the key |
| --- | --- | --- |
| macOS | the login Keychain, which Vibe asks through `/usr/bin/security` | `security` on `PATH`: `find-generic-password -s ai.mistral.vibe -a MISTRAL_API_KEY` (no `-w`, so nothing is printed) |
| Linux | the Secret Service (GNOME Keyring, KeePassXC, or KWallet's Secret Service mode), through Python's `keyring` with its `SecretService` backend. The item has the attributes `service=ai.mistral.vibe` and `username=MISTRAL_API_KEY` | `gdbus call --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets --method org.freedesktop.Secret.Service.SearchItems "{'service': 'ai.mistral.vibe', 'username': 'MISTRAL_API_KEY'}"` (the runner adds `--timeout 5`), which returns item paths only, never the secret |
| any | `~/.vibe/.env` | a non-empty `MISTRAL_API_KEY=` line |

This was found by reading the installed Vibe (`vibe/utils/keyring.py` and
`vibe/setup/auth/api_key_persistence.py`; `mistral-vibe` 3.x), and not from
its documentation. Vibe also reads the pre-rename service `vibe` and moves it to
`ai.mistral.vibe` on its next start, so after one run of `vibe` the item is
under the new name. The check looks only for the new name: if a critic is
dropped as not signed in right after an upgrade, run `vibe` once. On Windows
the check asks no keyring (Vibe's is the Credential Manager there), so only
the `.env` line counts. Setup's model sync reads the key through `keyring` on
every platform except macOS. `secret-tool` is not needed, and NixOS does not install it
by default.

The critique runner uses one check for this, `vibeSignedIn` in the model
gateway's `application/vibe-sign-in.ts`. It lives in the gateway so that
gateway code can reuse it; the gateway itself has no Vibe sign-in check yet,
and `model-gateway usage` reports Vibe's `signed_in` as `unknown`. A critic
counts as signed in when that check finds the keyring item or the `.env`
line. An exported `MISTRAL_API_KEY` does not count, because a critic runs
with it unset. When the check finds neither, prepare drops the Vibe critics
with `vibe: not signed in; ...`. Setup's model sync
(`scripts/vibe-sync-models.py`) needs the key itself to call the Mistral API.
Off macOS it reads the key through `keyring`, which is why setup runs that
script with `--with keyring`.

To see which store holds it without printing the key: on Linux run the
`gdbus` command above. An empty answer, `(@ao [], @ao [])`, means no item.

### Hindsight Coding Agents

Claude Code and Codex get Hindsight's own plugin, **Hindsight Coding Agents**
(`coding_agents` in the registry), which replaced the deprecated
`hindsight-memory` plugin. Setup installs it at a pinned version with
`npx @vectorize-io/hindsight-coding-agents@<version> install claude-code codex
--server self-hosted`, which wires three hooks into each agent and a
`hindsight` MCP server (setup registers it in every Claude profile). It then
locks the plugin down in `~/.hindsight/coding-agent.json`:

- the shared bank `default`, never one per repository (`bankId`,
  `dynamicBankId: false`); the plugin's default per-repo banks are not routed;
- recall **once**, at a session's first prompt (`autoInject: recall`, about
  12 facts, 10s timeout);
- nothing saved on its own: session write-back, git seeding, the codebase
  survey, knowledge-page refreshes, bank reconfiguration and npm self-update
  are all off;
- a memory policy (`toolGuideExtra`) that tells the agent what is worth
  saving with the tools: long-term decisions and their reasons, conventions,
  facts about people and the user, durable context the repository does not
  record, and what the user asks to remember. Never small talk, task progress
  or secrets.

The token is read from `HINDSIGHT_API_TOKEN`; setup removes one copied into
the config. A key you add to that file yourself is kept, but the kit's keys
are reset on every run.

### Automatic vs. model-elective capture

| Client | Capture |
|---|---|
| Claude Code, Codex | **Recall automatic, saving deliberate** — Hindsight Coding Agents recalls at a session's first prompt; the agent saves with its `hindsight` or `memory-api` tools, by the memory policy. |
| Hermes (laptop) | **Model-elective** — recall and retain only happen when the model calls a `memory-api` tool itself. |
| Mistral Vibe | **Model-elective** for now, through the same MCP servers. Vibe's hooks fire only around tools and after a turn (`pre_tool`, `post_tool`, `post_agent`), so automatic retain is possible but automatic recall at prompt start is not (fleet-infra#399). |

All of them write into the same shared bank, `default` (the setup script pins
the plugin's `bankId`, and `hindsight-api.*` serves no other bank). The
`memory-mcp` (Basic Memory) server is retired: Basic Memory is parked at zero
replicas, and setup unregisters it from Claude, Codex, Vibe and local Hermes.
Its notes stay in the `knowledge-vault` repository.

`OVERLEAF_SESSION` is the `overleaf.sid` cookie from a logged-in browser
session on the **self-hosted** instance — not `overleaf_session2`, which is the
overleaf.com name and breaks auth against `overleaf.jorisjonkers.dev`
(verified: the wrong name makes `olcli whoami` report "Session invalid"). It is
optional, because `olcli auth` once stores the session itself; export it as an
override, or skip it. The self-hosted base URL is baked into the registry, so a
bare `olcli` with no `OVERLEAF_BASE_URL` talks to overleaf.com and every call
404s against a project that is not there.

`OUTLINE_API_KEY` is an API key of a **separate Outline user**, not your own
account. Outline gives the key exactly that user's access, so the `outline`
server reads and writes only what you share with that user: whole
collections, or single documents. A collection that every member can view is
visible to that user too, so keep collections the agent must not see private.
To set it up:

1. Create the user: sign in to `notes.jorisjonkers.dev` once with a separate
   auth-api account that has the notes grant.
2. Signed in as an admin, check that the MCP server is on under Settings →
   Workspace → AI.
3. Share what the agent may use with that user.
4. Signed in as that user, create a key under Settings → API & Apps, and paste
   it when setup asks.

Setup checks the key against `https://notes.jorisjonkers.dev/mcp` before it
stores it. Documents the agent writes belong to that user; share them back
with your own account to edit them there.

`GRIMOIRE_API_TOKEN` is a Grimoire **Access Token** (`gmt_…`). It acts as
the Account that minted it, held to the scopes chosen then: `read` for every
read, `build` for changing Campaigns and their prep, `play` for acting in
Sessions. No token can manage the Account or hold Admin powers, and a tool
that takes a `campaignId` works only for that Campaign's DM. Give the agent
only the scopes it needs; `read` alone keeps it read-only. A token lives at
most 365 days, so mint a new one and run setup again when it expires.

Signed in to `grimoire.jorisjonkers.dev`, mint the token on the Account page
under Access Tokens, and paste it when setup asks. Setup checks it against
`https://grimoire.jorisjonkers.dev/mcp` before it stores it, and the check
fails while the platform's forward-auth answers a bearer request to `/mcp`
with the login redirect: Grimoire checks the token itself, so the edge has to
let `/mcp` through to it first.

Setup registers `grimoire` at user scope. A `grimoire` entry added by hand at
local scope, for one directory, overrides it there. Remove such an entry in
that directory with `claude mcp remove grimoire --scope local`, once in each
Claude profile.

### Keeping a directory out of the shared bank

The shared bank `default` is the one Hermes and every agent recall from.
Nothing is saved to it automatically any more, but Hindsight Coding Agents
still recalls from it and offers its tools. For a directory whose sessions
are private:

```bash
memory-private on [DIR]      # default: the current directory
memory-private status [DIR]
memory-private off [DIR]
```

`on` sets `HINDSIGHT_DISABLED=true` in that directory's
`.claude/settings.local.json` (Claude Code's per-user settings), keeping every
other key. Claude Code git-ignores that file only when it creates it, so in a
repository where it is not ignored yet, `on` also adds it to `.git/info/exclude`. Claude Code hands that `env` block to the plugin's
hooks once the folder is trusted, and the plugin then goes inert there: no
shared-bank recall at all. For one session only: `HINDSIGHT_DISABLED=true claude`.

### The Nedap bank and the bank resolver

Employer facts live in their own Hindsight bank, `nedap` (ADR-0012), never in
Shared Memory (`default`) and never in Private Memory. A session's bank is
chosen by its repository's remote owner, by the `memory-bank` command
(`scripts/crates/memory-bank`), which reads the owner the way the model
gateway and the critique runner do: a repository with any `nedap`-owned remote,
on any host or alias, is the Nedap bank; every other repository, and a
directory with no remote, is Shared Memory.

```sh
cargo run -q --manifest-path scripts/Cargo.toml -p memory-bank -- resolve [PATH]   # {"bank":"nedap","bank_id":"nedap"} or {"bank":"shared","bank_id":"default"}
```

The resolver knows two banks and cannot name Private Memory, which only
`vibe-private` reaches, behind the passphrase. If git cannot read the remotes,
it exits 1 with no bank (as it does for a remote URL that names no owner, such as a local mirror path, and a stripped GIT_DIR keeps git on the path given): an unreadable remote never falls back to Shared
Memory. A session holds the one bank its repository resolves to, so a `nedap`
session recalls and retains in the Nedap bank only and a personal session never
reaches it (the crate's tests prove both against an in-process server). No
Gold Band agent id reaches Private Memory: `memory-private` is
`enabled_for: [vibe-private]` alone, and the registry check refuses a Gold Band
instance that names the `private` bank or the server.

The employer-specific endpoint and token for the Nedap bank are not in this
public registry. They go in the work machine's overlay
(`~/.config/agent-kit/overlay.yaml`, `instances.gold-band.mcp_servers`), like
every other employer entry. The deployed memory server must serve the `nedap`
bank on its own host and token service before that entry works; that is a
live step listed on agent-kit#440.

### Vibe launchers and Private Memory

Personal notes go to Private Memory, a separate Hindsight bank (`private`),
served only on `hindsight-private.jorisjonkers.dev` and only to a
`MEMORY_PRIVATE` service token. Every other agent and launcher gets Shared
Memory (`memory-api`) only. Vibe has three launchers:

| Launcher | Mistral account | Memory | Passphrase | Vibe home | Who may run it |
|---|---|---|---|---|---|
| `vibe` | personal (Vibe's own sign-in) | Shared Memory | no | `~/.vibe` | you and agents |
| `vibe-study` | Study account | Shared Memory | no | `~/.vibe` (shared with `vibe`) | you and agents |
| `vibe-private` | personal | Shared Memory + Private Memory | yes, at a terminal | `~/.vibe-private` | you only |

The names follow one rule: an account suffix (`-study`, like `claude-personal`)
says which account a launcher signs in with, and `-private` marks the one
launcher behind the passphrase gate. See [ADR-0023](https://github.com/JorisJonkers-dev/workspace/blob/main/docs/decisions/ADR-0023-only-vibe-private-reaches-private-memory.md).

- **`vibe`** is Vibe itself, with no wrapper. Its home lists every
  workstation MCP server except `memory-private`, and its `AGENTS.md` names
  only `memory-api`.
- **`vibe-study`** (`vibe-study --acp` starts Vibe's ACP server instead, for Gold Band, with the same refusals) runs Vibe in the same home, with the Study account key
  (`MISTRAL_STUDY_API_KEY`) handed over as `MISTRAL_API_KEY`, which overrides
  Vibe's machine-wide personal login. Sign in once with `vibe-study --login`:
  it runs Vibe's own browser sign-in (`vibe --setup`) in a throwaway Vibe
  home with Vibe's keychain switched off (`VIBE_TEST_DISABLE_KEYRING=1`), so
  the key that sign-in mints lands in that home's `.env` instead of replacing
  the personal login. It checks the key against Mistral's model list (on
  stdin, never argv), keeps it in the agent-kit store, never exported, reads
  it back the way `vibe-study` will, and deletes the throwaway home. It needs
  setup's `env.sh`, so run setup first. Run it again to sign in afresh.
  The keychain switch is Vibe's own but undocumented: on macOS, if a Vibe
  upgrade ever ignores it, `--login` sees the personal login change, puts it
  back and fails; if it cannot read the personal login first, it does not
  sign in at all. Elsewhere there is no keychain item to watch, so check that
  plain `vibe` still signs you in after a Vibe upgrade. Without the key,
  `vibe-study` refuses rather than falling back to the personal account. It drops an inherited `MEMORY_PRIVATE_TOKEN` and never
  uses vibe-private's home, even when started from inside a vibe-private
  session.
- **`vibe-private`** runs Vibe in its own home, `~/.vibe-private`, the only
  one whose config holds `memory-private` and whose `AGENTS.md` asks it to
  recall from both banks. Its sessions never share a directory with `vibe`
  or `vibe-study`, so a password-free run can neither resume nor read them.
  It always signs in with the personal login: an inherited Mistral key is
  dropped.

**The token is behind a passphrase only you know.** Setup mints it and seals
it with `agent-kit-private` (openssl AES-256-CBC, PBKDF2 600,000 rounds) in
`~/.config/agent-kit/private/`, a 0700 folder. The first time, you choose the
passphrase; it is not your login password, and nothing can recover it (setup
then mints a new token). After that:

- `vibe-private` runs `agent-kit-private exec vibe`, which asks for the
  passphrase at the terminal and gives the token to that one launch.
- Nothing else can open it. The file is ciphertext, the passphrase and token
  never reach argv, and without a terminal the gate asks nothing and gives
  nothing, so an agent's shell gets no prompt to answer. The gate never reads
  stdin.
- Claude Code and Codex cannot reach Private Memory at all: no server entry,
  no session launcher. Setup removes what earlier versions installed for them
  (the `--mcp-config` file and Codex's switched-off entry), and converge
  removes `memory-private` from any surface but vibe-private's home, even one
  the keep file names.
- Claude Code also gets deny rules (`claude_private_deny` in the registry):
  its file tools and Bash may not touch `~/.vibe/**` or `~/.vibe-private/**`,
  the sealed token, or `agent-kit-private`. These stop accidental reads, not
  a determined agent: a Bash pattern misses a rewritten path (`~/.v""ibe`) or
  a script. The passphrase is the boundary; the logs have none beyond these
  rules.

**The split.** Before these launchers, every Vibe run shared `~/.vibe`, and
its history holds sessions that recalled Private Memory. On the first run
after the upgrade, setup moves that whole home to `~/.vibe-private`, history
and all, and seeds a fresh `~/.vibe` with only its settings and login
(`config.toml`, `hooks.toml`, `AGENTS.md`, `.env`, `skills/`, `prompts/`,
`agents/`). It runs once, only for a home that listed `memory-private`, never
overwrites an existing `~/.vibe-private`, and deletes nothing
(`scripts/vibe-split-homes.py`). The fresh home is built beside the old one
first: if that fails, nothing has moved, setup leaves both Vibe homes alone
for that run, and the next run tries again. `--check` says what it would do.
The `vibe` function an earlier `env.sh` defined is dropped (a `vibe` function
of your own is kept); open a new shell.

Known limits:

- Codex has no read deny-list, so it could read the Vibe homes' logs, though
  not the token.
- Vibe started by an editor or the Vibe app uses the shared home and has no
  Private Memory.
- Anything run inside an unlocked vibe-private session (Vibe's own tools, a
  plain `vibe` it starts) inherits the token and that session's home: it is
  already past the passphrase. `vibe-study` drops both.
- Vibe keeps MCP OAuth tokens machine-wide, not per home; Private Memory
  uses a static token, so it is not affected.
- Mistral models are synced into both homes with the personal login.
  `vibe-study` lists the same models; one the Study account cannot use fails
  when picked.
- A token left in the login keychain by an earlier setup is removed on the
  next run and replaced; it cannot be revoked from here, since no id was kept
  for it. Setup records each token's id (`~/.config/agent-kit/token-ids.json`,
  ids only) and revokes a token as soon as it replaces it.

Without the token, `vibe-private`'s Vibe connects to Private Memory with no
credential and is refused.

Vibe saves to Private Memory only what is worth keeping, by tool:
`~/.vibe-private/AGENTS.md` asks it to keep facts about people and you,
plans, decisions and their reasons, and anything you ask it to remember, and
never to copy personal notes into `memory-api`, which Hermes and every other
agent read. Nothing is saved every turn.

### Revoking a host token, and the per-host OpenRouter key (#508)

**Revoke demonstration.** Setup records each minted token's id in
`~/.config/agent-kit/token-ids.json` (ids only, never tokens). Revoke one host's
token and watch its host refuse it; nothing prints a token:

```bash
cd ~/Projects/JorisJonkers-dev/agent-kit
python3 scripts/agent-kit-keys.py --table registry/generated/workstation-secrets.json status          # HINDSIGHT_API_TOKEN ok
python3 scripts/agent-kit-keys.py --table registry/generated/workstation-secrets.json revoke HINDSIGHT_API_TOKEN   # asks password and TOTP
python3 scripts/agent-kit-keys.py --table registry/generated/workstation-secrets.json status          # HINDSIGHT_API_TOKEN invalid
```

`status` asks the token's own host (`hindsight-api.jorisjonkers.dev`) with the stored
token, so `invalid` is that host refusing it. Other hosts' tokens keep working. Running
setup afterwards sees the refused value and mints a fresh one (`ensure`).

**OpenRouter key per host.** Each host keeps its own `OPENROUTER_API_KEY` in its own
secret store (Keychain or Secret Service), so no key is shared and a lost machine's key
is revoked alone on openrouter.ai. Make a key on openrouter.ai/settings/keys named for the
host, then run setup in a terminal and paste it when asked (setup checks it against
OpenRouter's key endpoint before it stores it). To store one without a full setup run
(this stores it unchecked):

```bash
printf "OpenRouter key for %s: " "$(hostname)"; stty -echo; read -r k; stty echo; echo
printf %s "$k" | agent-kit-secret put OPENROUTER_API_KEY; unset k
agent-kit-secret has OPENROUTER_API_KEY && echo stored
```

**Overleaf CLI login decision.** The options for how `olcli` logs in to
`overleaf.jorisjonkers.dev`: (1) `olcli --base-url https://overleaf.jorisjonkers.dev --cookie-name overleaf.sid auth`
with the `overleaf.sid` session cookie,
which olcli keeps in its own config and later commands reuse until it expires; (2)
`OVERLEAF_SESSION` in the secret store, exported by `env.sh` as an override; (3) email
and password with `--save-password`, which keeps the password in plaintext and re-logs in
on expiry. Decision (recommended, the owner confirms in #508): option 1, per host, with
no saved password. The cookie is short-lived and revocable by signing out, an exported
`OVERLEAF_SESSION` would put it in every child process's environment, and a plaintext
password is the worst of the three. When the cookie expires, run `olcli auth` again.
On this machine olcli holds no login yet.

### Per-host service tokens (memory-api / memory-private)

auth-api (agent-kit#41) now issues per-user, per-host bearer credentials —
"service tokens" — for CLI/agent access to `hindsight-api.jorisjonkers.dev`
(the shared Hindsight bank) and `hindsight-private.jorisjonkers.dev` (the
private bank), both behind forward-auth. The `MEMORY_MCP` service for the
retired Basic Memory host is no longer used. This is the contract `setup-workstation.sh` will consume once a
follow-up lands the corresponding `mcp_servers:` entries — no ticket is filed
for that follow-up yet, see the note below.

1. While logged in to the estate (an existing `auth.jorisjonkers.dev` session
   cookie), mint one token per host:

   ```bash
   curl -sS https://auth.jorisjonkers.dev/api/v1/auth/service-tokens \
     -H 'Content-Type: application/json' \
     -H "X-XSRF-TOKEN: $(<csrf-token-from-browser>)" \
     -b "<session-cookie-from-browser>" \
     -d '{"service":"MEMORY_API","label":"<hostname>"}'
   # repeat with "service":"MEMORY_PRIVATE" for the private host
   ```

   Minting requires the caller already hold the `MEMORY_API` / `MEMORY_PRIVATE`
   grant (`user_service_permissions`) — an admin assigns that first, the same
   way any other service grant is assigned. The raw token is returned exactly
   once, as `smt_...`; there is no way to retrieve it again.
2. Send it as `Authorization: Bearer <token>` directly to the host. A token
   minted for one host is rejected by the other, and by every other
   forward-auth host in the estate: `/verify` checks the token's single
   `SERVICE_MEMORY_API` or `SERVICE_MEMORY_PRIVATE` claim against that host's own
   permission mapping, and denies outright on a host with no mapping at all
   (unlike a full session, which is unenforced there) — a service token never
   gets broader reach than the two hosts it was minted for.
3. Losing a laptop: `DELETE /api/v1/auth/service-tokens/{id}` (session-
   authenticated) revokes that host's token immediately, with no effect on
   any other device or host.

This is a manual, one-time step per host today: minting needs a browser
session, so it cannot be scripted headlessly without either a device-code
grant (rejected in the auth-api design as unnecessary complexity here) or a
human copying a session cookie. `setup-workstation.sh` storing/detecting the
result (so re-running setup is not a second manual step) needs a follow-up
ticket; none exists yet.

Both hosts are registered from `registry/estate-tooling.yaml`'s `memory-api`
and `memory-private` entries. The `memory-mcp` (Basic Memory) entry that was
registered beside them is retired (`surfaces: []`). The mem0 `memory` entry
is gone from the registry, and setup removes a leftover one as undeclared.

Every `workstation` MCP server is registered into **Claude Code, Codex and
local Hermes**. Codex gets each server via `codex mcp add`; local Hermes reads
`~/.hermes/config.yaml` and is given the rendered workstation block (see
`registry/generated/hermes/mcp-servers.local.yaml`) by
`scripts/hermes-merge-mcp.py`.

### The kubernetes server runs locally

`kubernetes` is `kubernetes-mcp-server` (an npm package, pinned, installed
through the package proxy's 7-day cooldown) run on the workstation over stdio.
The in-cluster server and the port-forward to it are retired (estate#316).
Each agent starts [`installer/kubernetes-mcp-readonly`](../installer/kubernetes-mcp-readonly)
(setup puts it on `PATH`), which:

- takes the current kube context, or `AK_KUBE_CONTEXT`, which must be the
  read-only context `workstation-kube.py` installs (`<cluster>`, for example
  `personal`; ServiceAccount `<device>-ro`, view without Secrets). A context
  ending in `-admin` or `-pf` is refused;
- gives the server a throwaway kubeconfig holding that one context, so no tool
  can switch to another, and deletes it when the client closes;
- starts it with `read_only` and `disable_destructive` (the 0.1.x server takes
  these as TOML options, not `--read-only` flags), Secrets denied, and only
  the `core` toolset, because the `config` toolset prints the kubeconfig.

Switch clusters with `AK_KUBE_CONTEXT` or `kubectl config use-context`; no
setup re-run is needed. Setup also removes the old port-forward launchd agent
(`dev.jorisjonkers.agent-kit.port-forward.*`) and systemd user unit from a
machine that has them.

The GitHub MCP server comes from the `github` **plugin**
(`external_plugins/github` in the official plugin marketplace; setup
installs it from `agent-kit-official`), which setup
cannot rewrite. Its `.mcp.json` is:

```json
{"github": {"type": "http", "url": "https://api.githubcopilot.com/mcp/",
            "headers": {"Authorization": "Bearer ${GITHUB_PERSONAL_ACCESS_TOKEN}"}}}
```

Claude Code expands `${GITHUB_PERSONAL_ACCESS_TOKEN}` from the environment it
was started in. When the variable is unset or empty the header goes out as
`Authorization: Bearer ` with no token, and `https://api.githubcopilot.com/mcp/`
answers `HTTP 400: Error POSTing to endpoint: bad request: Authorization header
is badly formatted`. The empty token is inferred from the header template, not captured on the wire. That reads like an unconfigured server; the cause is the
missing variable. The plugin's `.mcp.json` names no other credential: nothing in it reads
`gh auth` or starts an OAuth flow.

To get the tools, start Claude Code with the variable set to a GitHub token
that can reach the repositories you work on, for example
`GITHUB_PERSONAL_ACCESS_TOKEN="$(gh auth token)" claude`. Setup never writes
that token anywhere, and neither should a profile file you commit. The same 400 is
reported upstream with other triggers:
[anthropics/claude-code#84367](https://github.com/anthropics/claude-code/issues/84367)
(closed as not planned) describes a stray `}` leaking into the header after
the template is expanded, and
[anthropics/claude-code#107604](https://github.com/anthropics/claude-code/issues/107604)
(open) the desktop Code tab. A session without the variable has no github MCP tools; use `gh`
instead.

Checked live on the Linux workstation (2026-11-12): `GITHUB_PERSONAL_ACCESS_TOKEN`,
`GITHUB_PAT`, `GH_TOKEN` and `GITHUB_TOKEN` were all unset in the environment
Claude Code was started from, and the session's MCP status listed
`plugin:github:github` as failed with exactly the 400 above, so the cause here
is most likely the missing variable (inferred from the header template; the
server was not shown connecting with the variable set), not an agent-kit bug (the
plugin ships the `Bearer ${GITHUB_PERSONAL_ACCESS_TOKEN}` header itself and
setup cannot rewrite it). The owner step: export the variable from a private,
uncommitted shell profile (`export GITHUB_PERSONAL_ACCESS_TOKEN="$(gh auth
token)"`, or a fine-grained token with repository read and write), then start a
new Claude Code session. A token with a trailing newline or a `Bearer ` prefix
of its own is expected, from the header template, to produce the same 400 (not
tested), so keep the value to the bare token.

## Playwright MCP on a cold cache

The registry pins `@playwright/mcp` (container pin, `post_install` fetches the
matching Chromium) and Claude and Codex start it as the one `playwright-mcp`
binary on `PATH`, headless (Vibe's config was not read, see below):
`playwright-mcp --headless --browser chromium`. It is never run through `npx`,
so an empty `~/.npm/_npx` cannot change which binary starts.

Checked live on the Linux workstation (2026-11-12) with `~/.npm/_npx` moved
aside (restored afterwards), `playwright-mcp` resolving to
`~/.npm-global/bin/playwright-mcp`, which links to
`~/.npm-global/lib/node_modules/@playwright/mcp/cli.js`:

| Agent | Tools listed | Binary the server ran from |
| --- | --- | --- |
| Claude | 25 (`mcp__playwright__browser_*`), server `connected` | `playwright-mcp` from `~/.npm-global/bin` per its config; the process snapshot cannot tell its server from other sessions' |
| Codex | 25 (`browser_*`, through the app-server MCP status call) | the same command; a new `playwright-mcp` process appeared during the Codex run, but its parent was not recorded as Codex |
| Vibe | 25 (`browser_*`, through its tool search) | not observed: Vibe's process was not seen starting it and its config was not readable from the check |

Codex was not logged in to a model account, so its tools were listed from its
MCP status call rather than from a model turn. Vibe's resolved path therefore
remains unverified; #505 stays open for it.

## Package proxy

One step of the run then points every package manager on the machine at the
estate's packages proxy, `https://packages.jorisjonkers.dev`, so third-party
dependencies all arrive through one host with one key. First-party packages
stay direct: the `@jorisjonkers-dev` scope on npm.pkg.github.com, Maven
artifacts from `maven.pkg.github.com/JorisJonkers-dev/*`, and Go modules
from `github.com/JorisJonkers-dev/*`.

The key is fetched in a fixed order: a `PACKAGES_PROXY_TOKEN` already in
the environment; else
`vault kv get -mount=secret -field=workstation packages-system/keys`, only
when `vault token lookup` succeeds; else the stored token file; else a
no-echo prompt, which only a run in a terminal asks. Whatever answers, the
key is checked against `https://packages.jorisjonkers.dev/npm/typescript`
before anything is written, and a refused key writes nothing at all — not
the token file, not one package manager. A working key is stored at
`${XDG_CONFIG_HOME:-~/.config}/jorisjonkers/packages-token`, mode 0600, so
a later run finds it there and never asks again while it still works.

What the step writes — each writer owns one marked block or one file:

| Target | What setup puts there |
|---|---|
| `~/.netrc` | the proxy machine line (`machine packages.jorisjonkers.dev login packages`) |
| `go env` | `GOPROXY` and `GOPRIVATE` |
| `~/.npmrc` | the proxy registry and `_authToken` |
| `~/.config/uv/uv.toml` | the default index, and `exclude-newer = "7 days"` so a build never depends on a package newer than its lock |
| `~/.config/pip/pip.conf` | the proxy `index-url` under `[global]` |
| `~/.cargo/config.toml` and `~/.cargo/credentials.toml` | the sparse registry, and the token |
| `~/.gradle/init.d/packages-proxy.init.gradle.kts` | the init script, with no key inside it |
| `~/.gradle/gradle.properties` | the key (`packagesProxyToken`), 0600 |
| `~/.m2/settings.xml` | the mirror and its server — written only when the file does not exist, 0600; a `settings.xml` you already have is never edited, and the snippet to add by hand is printed with `PACKAGES_PROXY_KEY` standing in for the key |

`~/.config` is `$XDG_CONFIG_HOME` when that is set. The Gradle init script
filters third-party repositories at the settings level and adds the proxy
there; at project level it only filters, and it adds the proxy to a project
only when that project declares its own third-party repository — so a build
with `FAIL_ON_PROJECT_REPOS` still works.

A setting outside the block that would clash with it (your own `registry=`
line in `.npmrc`, your own `exclude-newer`, your own `index-url`) makes that
one writer refuse with a warning and leave the file exactly as it was; the
other writers still get their blocks. The first write of each file is backed
up once to `<file>.bak-agent-kit`. Re-running setup just re-applies the same
blocks, so it is the way back after a refused or partial write.

The step is skipped under `--check` (report only), in a `--cloud` session
(the cloud environment brings its own), and when `HOME` is unset or `/`.
With no key at all it warns and skips — no other step depends on it, so a
machine without the proxy still gets its full setup.

Known limits:

- The Gradle init script is tested by static assertions only, never under a
  real Gradle build.
- The Vault path needs a Vault login first: without one the order falls
  through to the stored token file or the prompt.
- Personal Tokens (identity#174/#175) will replace the key source later.

## What the exit code means

The script is `set -uo pipefail`, not `-e`, on purpose: one unavailable
upstream should not abandon the other twenty steps. It counts instead.

- `FAIL` — something the estate depends on is not working. Non-zero exit.
- `warn` — an optional piece is unavailable (no credential, a language server
  whose toolchain is not installed on this machine). Zero exit.

The last line is the tally. A zero exit with warnings is a normal outcome on a
machine that does not do Ruby or C#.

## Two Claude logins, one setup

`CLAUDE_CONFIG_DIR` moves a Claude Code config root whole: credentials,
`projects/`, `history.jsonl`, `sessions/` and `.claude.json` all travel with
it. That is what makes a second *login* a second *directory*, and it is also
what keeps a personal conversation out of the work account's history.

The registry declares the profiles under `claude_profiles:`. Today:

| Profile | Config root | Holds |
|---|---|---|
| `work` (primary) | `~/.claude` | the default a bare `claude` uses, and every shared asset |
| `personal` | `~/.claude-personal` | its own login and its own history; everything else is a symlink |

Setup creates the secondary root and symlinks each surface named in its
`shared_paths:` back into the primary — today `skills`, `agents`, `commands`,
`hooks`, `plugins` and `settings.json`. So:

- **Plugins install once.** `plugins/` is shared and `enabledPlugins` lives in
  the shared `settings.json`, so a plugin installed for work is already
  installed, and already enabled, for personal.
- **MCP servers are registered twice.** A server lives in the profile's own
  `.claude.json`, which is exactly the file the two profiles must not share.
  Every `claude mcp add` in the generated script runs once per profile.
- **Nothing stateful is shared.** The renderer refuses a `shared_paths:` entry
  naming `projects`, `history.jsonl`, `sessions`, `.claude.json` or the
  credentials file, and a test proves the refusal fires.

Setup also writes a launcher per secondary profile into `~/.local/bin` (or
`$CLAUDE_LAUNCHER_DIR`), the directory `claude` itself installs into:

```bash
claude-personal   # first run: log in, personal account
```

The launcher only sets `CLAUDE_CONFIG_DIR` and execs `claude`. A
`claude-<profile>` file that setup did not write is left alone and reported.

Setup always installs the work profile into `~/.claude`. An ambient
`CLAUDE_CONFIG_DIR` (for example from inside a `claude-personal` session) is
ignored, and setup says so on stderr; the personal root stays a separate profile.

A real file or directory already sitting where a symlink would go is **left
alone** and reported in the summary; the script never replaces one. Fix it by
moving your own copy aside and re-running.

### The one thing to verify on macOS

Claude Code stores the OAuth tokens in the login keychain under the service
`Claude Code-credentials`. Whether that entry is namespaced per config root
decides whether two profiles can hold two accounts at once. Check after
logging both in:

```bash
security dump-keychain | grep -c '"svce"<blob>="Claude Code-credentials"'
```

Two entries: the profiles are independent. One: the logins overwrite each
other and a switch means logging in again — use a separate macOS user account
for the personal profile instead.

## Antigravity

Google Antigravity is another agent surface beside Claude Code, Codex and
Vibe: a desktop agent manager and a CLI, `agy`, that share one configuration.
Setup installs the CLI on both operating systems and writes that configuration
the way it does for the others.

**Does Antigravity have an official Linux build?** Yes, for the CLI. Google's
getting-started page (<https://antigravity.google/docs/getting-started?tab=cli>,
checked 2026-11-12) lists one automated installer for macOS and Linux,
`curl -fsSL https://antigravity.google/cli/install.sh | bash`, which puts `agy`
at `~/.local/bin/agy`; the same page gives glibc requirements for the Linux
desktop app. So there is no "macOS only" skip on Linux.

**Who installs it.** Setup, not nix-config: `agy` is not a Nix package this
estate pins, so on NixOS too setup runs the vendor installer when `agy` is not
on `PATH`, and runs it again as the update, the way it does for the Claude Code
installer. On macOS it is the `antigravity-cli` Homebrew cask. The desktop app
(`antigravity` cask) is installed on macOS only; on Linux setup logs that it is
not installed and `agy` carries the whole configuration. In `--cloud` mode it
logs `not set up in cloud mode`, because a cloud host is Linux and unattended.

The critique runner treats the Google critic as ready on Linux exactly as on
macOS: `agy` on `PATH` and signed in (`agy models` lists the routed model).

### Signing in is yours to do

Antigravity signs in with a Google account through the browser on first launch
of `agy` or the app, and keeps the credential itself. agent-kit never sees,
stores or passes a Google credential, and signing in on one surface signs in
the others.

That leaves one failure setup can only report: a headless run uses the cached
credential, so an unsigned machine fails later with an authentication-required
error instead of at setup. So when `agy` is on `PATH`, setup runs
`agy models`, with stdin closed and a 20-second alarm. Listing the models
needs a signed-in account but calls no model, so it spends no quota and
answers even while the quota is spent; signed in, it takes about a second.
When agy answers that you must sign in, setup warns
`antigravity: needs sign-in: run agy and sign in with your Google account`;
any other failure warns `antigravity: agy models failed: <agy's first line>`.
Both show under `--check` too, so they are in **Still to do** before you hit
them. Only the CLI can be asked: a machine with the desktop app and no `agy`
is not checked.

### Shared Memory, through a proxy

Antigravity gets the workstation MCP fleet minus `memory-private`: Shared
Memory only, never Private Memory. `memory-private` is a session-only server
and the renderer refuses `enabled_for: [antigravity]` on one, so there is no
path to it.

The token cannot live in Antigravity's configuration. Its MCP file expands no
environment variables — a header value is literal — so a token in `headers`
would be a token on disk. `memory-api` is registered as a local stdio command
instead, `~/.local/bin/antigravity-mcp-proxy`, which reads
`HINDSIGHT_API_TOKEN` from the agent-kit secret store at start-up (through
`agent-kit-secret`: the Keychain entry `agent-kit.HINDSIGHT_API_TOKEN`, or the
Secret Service item on Linux; no file fallback)
and proxies JSON-RPC to `hindsight-api.jorisjonkers.dev`. The token stays in
memory: never on disk, never in argv, never printed. The fragment runs it
through `/bin/sh -c` because a GUI-launched app has no `~/.local/bin` on
`PATH`, and Antigravity does not expand variables in `command`. When the
server drops its session (a redeploy, say), the proxy opens a new one and
retries, so Antigravity needs no restart.

A server that needs no token is registered by URL: `n8n`, whose OAuth tokens
Antigravity keeps in `~/.gemini/antigravity/mcp_oauth_tokens.json`, and
`kubernetes` is a local stdio command (see above).

### Where its configuration lives

| Path | What setup puts there |
|---|---|
| `~/.gemini/config/mcp_config.json` | the MCP fleet, merged from `registry/generated/antigravity/mcp_config.json` |
| `~/.gemini/config/AGENTS.md` | the instructions pointer, Shared Memory only |
| `~/.gemini/config/skills/<name>/` | the kit's skills, for the app |
| `~/.gemini/antigravity-cli/skills/<name>/` | the same skills, for `agy` |

The two skills folders are not a duplicate: the app reads
`~/.gemini/config/skills/` and `agy` reads
`~/.gemini/antigravity-cli/skills/`, so setup copies each skill into both.

One file serves the app, the CLI and the IDE. `agy mcp add` writes one server
at a time, so setup merges the whole fragment with
`scripts/antigravity-merge-mcp.py` instead: a server the fragment names
replaces any entry of that name, and every other server and every other
top-level key in the file is kept. A server you turned off with
`agy mcp disable` stays off. The first merge copies the file
to `mcp_config.json.bak` and never overwrites that copy, so the config you had
before agent-kit survives. Antigravity is the one surface setup does not
converge — a server you add to that file yourself stays. The merge runs only
when `agy` is on `PATH`; without the CLI there is nothing to configure.

Setup writes **only** under `~/.gemini/config/` and
`~/.gemini/antigravity-cli/`. It never touches `~/.gemini/AGENTS.md` or
`~/.gemini/GEMINI.md`, which Gemini CLI reads too, and it leaves a
`~/.gemini/config/AGENTS.md` of your own alone: the file it writes carries the
`managed by agent-kit` marker, and one without it is reported and kept. Note
that `brew uninstall --cask antigravity` removes part of `~/.gemini` as well,
so uninstalling the app can take your own files with it.

## OpenCode

[OpenCode](https://opencode.ai) is another terminal agent beside Claude Code,
Codex and Vibe. Setup installs **OpenCode 2** from Homebrew core
(`brew install opencode`) and configures it like the others. It is not a
council or critique target: no runner routes work to it.

The opencode.ai install script, the npm `opencode-ai` package and the
`anomalyco/tap` formula still install OpenCode 1, which reads MCP servers from
another shape. When the `opencode` on `PATH` reports a 1.x version, setup warns
`opencode: <version> is OpenCode 1; its MCP config is left alone`, and neither
merges nor converges `opencode.json`: the V2 shape would read there as one
server named `servers`. It leaves the binary alone too: only an OpenCode that
Homebrew installed is Homebrew's to upgrade. Skills and `AGENTS.md` are
written either way.

Without Homebrew on `PATH` (a Linux machine, say) the install step says so:
`opencode: Homebrew is not on PATH; install it, or OpenCode 2, then re-run`.

OpenCode is a workstation surface only. A `--cloud` run logs
`opencode: not set up in cloud mode` and writes nothing under
`~/.config/opencode`.

### What it gets

| Path | What setup puts there |
|---|---|
| `~/.config/opencode/opencode.json` | the MCP fleet under `mcp.servers`, merged from `registry/generated/opencode/opencode.json` |
| `~/.config/opencode/AGENTS.md` | the instructions pointer, Shared Memory only |
| `~/.config/opencode/skills/<name>/` | the kit's skills, the vendored skills (superpowers), and the skills of every enabled Claude Code plugin |

`~/.config` is `$XDG_CONFIG_HOME` when that is set.

**MCP servers.** The workstation fleet minus `memory-private`, as for
Antigravity; the renderer refuses `enabled_for: [opencode]`. OpenCode fills in
`{env:NAME}` when it loads the file, so a token-bearing server carries
`Authorization: Bearer {env:HINDSIGHT_API_TOKEN}` (and so on) with OAuth
discovery off, and no secret reaches the file. The value comes from the
environment OpenCode starts in, which `~/.config/agent-kit/env.sh` sets up in
every shell. `n8n` is left to OpenCode's own sign-in: run
`opencode mcp auth n8n` once. OpenCode gets no `hindsight` server, because
Hindsight Coding Agents installs for Claude Code and Codex only; `memory-api`
reaches the same Shared Memory bank.

`scripts/opencode-merge-mcp.py` replaces each server the fragment names, keeps
an `enabled` flag you set on one, and keeps every other key in the file. The
first merge copies the file to `opencode.json.bak` and never overwrites that
copy. The backup is taken only from a file no merge has touched, so a machine
with no OpenCode config before agent-kit never gets one. Each write goes to a temporary file that is then renamed over it, so a
starting OpenCode never reads half a file. A file with comments in it is
refused rather than rewritten without them, and convergence skips it with a
note while the other agents still converge. Otherwise the file converges like
Codex's: an undeclared server goes unless the keep file names it. Both steps run only when `opencode` is on `PATH`.

**Skills.** OpenCode reads `~/.claude/skills` and `~/.agents/skills` as well
as its own folder, and its own folder wins a name clash. Setup copies the
kit's and the vendored skills into its own folder all the same, so OpenCode
holds the registry's copy even when Claude's home is somewhere else.

**Plugins.** OpenCode cannot load a Claude Code plugin, and its own plugins
are JavaScript modules, so setup carries over what a plugin ships that
OpenCode can use: its skills. `scripts/opencode-plugin-skills.py` asks
`claude plugin list --json` where each enabled plugin of the registry is
installed and copies its skills the way Claude finds them: each path its
`plugin.json` lists under `skills`, the default `skills/` folder, or a
`SKILL.md` at the plugin's root. A plugin you disabled in Claude ships no
skills, and a `skills` path that leads out of the plugin is ignored. Each
copy is built beside the others, marked with a `.agent-kit-plugin-skill` file
naming its plugin, and only then renamed into place, so an interrupted run
cannot leave an unmarked copy. The next run replaces the marked copies,
removes any that no enabled plugin ships any more, and never touches a skill
folder without the marker. A plugin's hooks, agents and commands stay Claude's — the
caveman hooks and security-guidance among them — and so do plugin MCP servers
(the `github` plugin's needs a token setup does not manage).

**The service.** OpenCode 2 runs a background service that reads its config
and skills once, at start. When `opencode service status` shows it running,
setup restarts it at the end of a run so the new configuration takes effect.

## Same-name MCP servers in two layers

Result of the same-name overlap test (#439): a trivial stdio server named
`overlap-echo`, with one tool `echo`, declared with the same name in the user
layer (the agent's own config) and the project layer (a file in the working
directory) at once, in the personal instance on the Linux workstation
(2026-11-12). Each declaration returned `layer=<user|project>` and logs a line when
it starts, so the winner shows in the tool output and the start log.

| Agent | User layer | Project layer | Result |
| --- | --- | --- | --- |
| Claude (`~/.claude.json`, `.mcp.json`) | declared | declared | One tool, `mcp__overlap-echo__echo`; the project server answered and the user server was never started. The name overrides; nothing appears twice. |
| OpenCode (`~/.config/opencode/opencode.json`, `opencode.json`) | declared | declared | One tool, `overlap-echo.echo`; the project server answered. The name overrides; nothing appears twice. |
| Vibe (`~/.vibe/config.toml`, `.vibe/config.toml`) | not run | not run | Not tested: the check may not edit or back up Vibe's shared home, which also holds the credentials. |

The work instance is not on this machine (it lives on the work Mac only), so it
was not tested; the agents it binds are Claude and Codex, and Codex was not
tested in either instance.

In Claude and OpenCode the project layer overrides the user layer for the same
name, so a server declared in both is not duplicated, but the user-layer
declaration is then dead weight and might hide a stale command (a risk, not
observed); the registry's layering rule is to declare
each server in exactly one layer per agent.

## Language servers have two halves

A `*-lsp` plugin declares the LSP wiring; the language server **binary** does
the work. Install the plugin without the binary and the plugin registers no
tools and says nothing about why — from inside a session it is
indistinguishable from a working one.

So the script verifies `command -v <binary>`, never the plugin list, and warns
by name when the binary is absent:

| Plugin | Binary | Comes from | On NixOS |
|---|---|---|---|
| `typescript-lsp` | `typescript-language-server` | npm | npm |
| `pyright-lsp` | `pyright-langserver` | npm | npm |
| `kotlin-lsp` | `kotlin-lsp` | brew; JetBrains tarball on Linux | tarball |
| `jdtls-lsp` | `jdtls`, runs `java` | brew; Eclipse tarball on Linux | `jdt-language-server` + a JDK |
| `ruby-lsp` | `ruby-lsp` | gem | `ruby-lsp` |
| `gopls-lsp` | `gopls` | `go install` | `gopls` |
| `rust-analyzer-lsp` | `rust-analyzer` | `rustup component add` | `rust-analyzer` |
| `lua-lsp` | `lua-language-server` | brew | `lua-language-server` |
| `clangd-lsp` | `clangd` | brew (llvm) | `clang-tools` |
| `php-lsp` | `intelephense` | npm | npm |
| `csharp-lsp` | `csharp-ls` | `dotnet tool install` | `csharp-ls` |
| `swift-lsp` | `sourcekit-lsp` | the Swift/Xcode toolchain — nothing to install | `sourcekit-lsp` |
| `liquid-lsp` | `shopify` | off by default: never installed; install it by hand to turn it on | same |

A server with a runtime (`jdtls` runs `java`) counts as present only when
both are on `PATH`; the launcher alone answers no request.

On NixOS (`/etc/NIXOS` exists) brew, gem, `go install` and rustup cannot
install into the system, so setup does not try. The packages in the last
column come from nix-config's agent-workstation role, and setup warns with
the package name when one is missing. Rerun setup after a rebuild to enable
the plugins.

## Claude Code is installed by its own installer

`claude` lives at `~/.local/bin/claude` and self-updates through
`claude update`. Do **not** also `npm i -g @anthropic-ai/claude-code`: two
installs on one `PATH` is how a stale binary wins an argument you did not know
you were having. The script updates in place when the binary exists and only
runs the installer when it does not.

## Hooks

The estate ships **four agent hooks**: the critique pipeline's
`critique-commit-gate`, `critique-pr-gate` and `critique-session-start`
([ADR-0002](adr/0002-exactly-three-critique-hooks.md)), and
`projects-layout-guard`, which denies a clone or worktree outside
`~/Projects/<owner>/` ([ADR-0009](adr/0009-projects-layout-guard.md)). The
guard's script is in the `projects-layout` skill, and the same rule is written
as a marked block into each client's global instruction file. The three
knowledge-recall hooks are retired along with the knowledge base they wrote
into.

`setup-workstation.sh` merges the critique hooks by name into Claude Code's
`settings.json` and Codex's `hooks.json`, one matcher group each, running the
script of that name in the agent home's `critique` skill. An existing group is
rewritten where it stands, because Codex keys its trust by position, and the
file is rewritten only when it changes. Codex runs them once you trust them in
`/hooks`; `/flow setup` checks that. `--uninstall` removes them. What they do
is in [the pipeline README](../skills/critique/README.md).

Just before that merge, on every run, it purges the retired hooks itself: it strips hook
groups from `settings.json` and `~/.codex/hooks.json` whose command basename is
one of the retired scripts, deletes the leftover script files, then
reads the file back and fails if any retired name survives. Your own hooks
are matched by basename and left alone. (Before agent-kit#40, `install-agents.sh`
owned this; it is retired along with `install.sh`, and `--uninstall` covers
what a machine set up by either of them still has.)

Plugin-owned hooks are a different thing entirely. `security-guidance` ships
its own `PreToolUse` and `Stop` hooks; the purge matches on the retired
scripts' basenames, so those survive.

## Spec Kit is not installed

The kit no longer carries Spec Kit: no `speckit.*` commands or skills, and no
`.specify` scaffold. Setup still deletes the `speckit.*` commands and skills
that older installers left in the Claude Code and Codex homes. A project that
keeps its own `.specify/memory/constitution.md` still has it read into council
prompts; only the seeding is gone.

## Related

- [REGISTRY.md](REGISTRY.md) — adding a tool so every surface gets it
- [MEMORY.md](MEMORY.md) — what replaced the knowledge base
- [CLOUD.md](CLOUD.md) — the same setup in a Claude Code cloud environment

### The secret store on Linux (B1-01)

`agent-kit-secret get|put|delete|has NAME` is the one interface; the key
script, `vibe-study` and the MCP stdio proxy all use it. On macOS it is
`security` and the login Keychain. On Linux it is the Secret Service, the
store Vibe itself uses.

NixOS ships no `secret-tool`, so the Linux backend is split by what each
operation needs. An existence check (`has`) asks the Secret Service for the
item path over D-Bus with `gdbus SearchItems`, the way the critique runner
checks Vibe's sign-in: it returns paths only, never a value. Reading and
writing go through Python's `keyring` (Vibe's own library), supplied by
`uv run --with keyring` when the system python has none. Adding libsecret to
nix-config was not needed and would not help a non-Nix Linux. Exit status:
0 done, 1 key absent, 2 no usable backend; a locked or absent keyring stops
the launch with a message and nothing is written elsewhere.
