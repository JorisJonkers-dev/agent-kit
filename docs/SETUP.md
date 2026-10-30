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

Each Claude profile, Codex and Vibe ends a run with exactly the MCP servers
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
4. It stores each in the macOS keychain as `agent-kit.<VARIABLE>` (a 0600
   `~/.config/agent-kit/secrets.env` where there is no keychain). The
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
| `MISTRAL_STUDY_API_KEY` | `vibe-study`: Vibe signed in with the Study account | optional; never exported, read only by `vibe-study` |
| `MISTRAL_API_KEY` | not set on the workstation: Vibe uses your account login. Only cloud sessions pass one in | cloud only |
| `OVERLEAF_SESSION` | the `overleaf` MCP server (self-hosted Overleaf) | optional |

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

Vibe lists only the models under `[[models]]` in `${VIBE_HOME}/config.toml`;
its own default is Mistral Medium 3.5. With the key Vibe signed in with (its
keychain item, or `MISTRAL_API_KEY` in a cloud session), setup reads
Mistral's model list (`scripts/vibe-sync-models.py`) and offers every chat
model the key can use: one entry per underlying model, under its `-latest` id
where there is one, so upgrades arrive on their own. Models without tool
calling, transcription models and deprecated ones are left out. A model you
define yourself under the same alias wins, and a model gone upstream is
removed on the next run.

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
| Hermes (laptop and in-cluster) | **Model-elective** — recall and retain only happen when the model calls a `memory-api` tool itself. |
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
- **`vibe-study`** runs Vibe in the same home, with the Study account key
  (`MISTRAL_STUDY_API_KEY`) handed over as `MISTRAL_API_KEY`, which overrides
  Vibe's machine-wide personal login. Create an API key in the Study
  account's Mistral console and paste it when setup asks. The key is stored
  like the other keys but never exported, so plain `vibe` keeps the personal
  login. Without it, `vibe-study` refuses rather than falling back to the
  personal account. It drops an inherited `MEMORY_PRIVATE_TOKEN` and never
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

### Servers reached through a port-forward

A server with a `workstation_connect` block, today only `kubernetes`, is a
ClusterIP service with no ingress route. It is registered at a loopback URL
(`http://127.0.0.1:18080/mcp`) that a `kubectl port-forward` publishes.
[`installer/port-forward-agent.sh`](../installer/port-forward-agent.sh) keeps
that forward up:

- **macOS**: a launchd user agent, `dev.jorisjonkers.agent-kit.port-forward.<name>`,
  with `KeepAlive`. A bare `kubectl port-forward &` dies when the laptop
  sleeps, the network changes or the pod restarts. After that the server drops
  out of every session, and nothing reports it. launchd restarts the forward
  whenever kubectl exits.
- **Elsewhere**: a one-shot background forward, which is not kept alive.

The forward is pinned to the kube context that is current when setup runs, and
it listens on `127.0.0.1` only. If you switch clusters, re-run setup to re-pin
it.

```bash
launchctl print gui/$(id -u)/dev.jorisjonkers.agent-kit.port-forward.kubernetes   # state
tail -f ~/Library/Logs/agent-kit/port-forward-kubernetes.log                      # kubectl output
launchctl bootout gui/$(id -u)/dev.jorisjonkers.agent-kit.port-forward.kubernetes  # stop until next login
rm ~/Library/LaunchAgents/dev.jorisjonkers.agent-kit.port-forward.kubernetes.plist # remove for good
```

If a forward started by hand already holds the port, setup warns. The agent
retries every 10s and takes over once that forward exits.

The GitHub MCP server comes from the `github` **plugin**, which manages its own
credential. It reports `Authorization header is badly formatted` when that
credential is missing or malformed — which reads like an unconfigured server
rather than a missing token.

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

Run setup from a shell **without** `CLAUDE_CONFIG_DIR` set, not from inside a
`claude-personal` session. With it set, the personal root looks like the
primary; setup refuses to link a root onto itself and fails that profile.

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

## Language servers have two halves

A `*-lsp` plugin declares the LSP wiring; the language server **binary** does
the work. Install the plugin without the binary and the plugin registers no
tools and says nothing about why — from inside a session it is
indistinguishable from a working one.

So the script verifies `command -v <binary>`, never the plugin list, and warns
by name when the binary is absent:

| Plugin | Binary | Comes from |
|---|---|---|
| `typescript-lsp` | `typescript-language-server` | npm |
| `pyright-lsp` | `pyright-langserver` | npm |
| `kotlin-lsp` | `kotlin-lsp` | brew |
| `jdtls-lsp` | `jdtls` | brew |
| `ruby-lsp` | `ruby-lsp` | gem |
| `gopls-lsp` | `gopls` | `go install` |
| `rust-analyzer-lsp` | `rust-analyzer` | `rustup component add` |
| `lua-lsp` | `lua-language-server` | brew |
| `clangd-lsp` | `clangd` | brew (llvm) |
| `php-lsp` | `intelephense` | npm |
| `csharp-lsp` | `csharp-ls` | `dotnet tool install` |
| `swift-lsp` | `sourcekit-lsp` | the Swift/Xcode toolchain — nothing to install |
| `liquid-lsp` | `shopify` | npm — installed disabled |

## Claude Code is installed by its own installer

`claude` lives at `~/.local/bin/claude` and self-updates through
`claude update`. Do **not** also `npm i -g @anthropic-ai/claude-code`: two
installs on one `PATH` is how a stale binary wins an argument you did not know
you were having. The script updates in place when the binary exists and only
runs the installer when it does not.

## Hooks

The estate ships **no agent hooks**. The three knowledge-recall hooks are
retired along with the knowledge base they wrote into.

`setup-workstation.sh` does the purge itself, on every run: it strips hook
groups from `settings.json` whose command basename is one of the retired
scripts, deletes the leftover script files and `~/.codex/hooks.json`, then
reads the file back and fails if any retired name survives. Your own hooks
are matched by basename and left alone. (Before agent-kit#40, `install-agents.sh`
owned this; it is retired along with `install.sh`, and `--uninstall` covers
what a machine set up by either of them still has.)

Plugin-owned hooks are a different thing entirely. `security-guidance` ships
its own `PreToolUse` and `Stop` hooks; the purge matches on the retired
scripts' basenames, so those survive.

## Spec Kit is not installed

Neither the `speckit.*` commands nor their skills are in the registry, on any
surface. The kit still renders the Spec Kit scaffold for repositories that
already depend on it; nothing new should adopt it.

## Related

- [REGISTRY.md](REGISTRY.md) — adding a tool so every surface gets it
- [HERMES.md](HERMES.md) — the same registry, applied to the cluster gateway
- [MEMORY.md](MEMORY.md) — what replaced the knowledge base
- [CLOUD.md](CLOUD.md) — the same setup in a Claude Code cloud environment
