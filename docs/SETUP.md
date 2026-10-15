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
`dotnet`) or a hand-added MCP server is noted, not listed.

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
3. It asks you to paste `MISTRAL_API_KEY` (Enter skips) and checks it.
4. It stores each in the macOS keychain as `agent-kit.<VARIABLE>` (a 0600
   `~/.config/agent-kit/secrets.env` where there is no keychain). The
   private-bank token is the exception: it is sealed behind a passphrase you
   choose (see The private bank). Secrets travel over stdin, never argv, and
   are never printed.
5. It writes `~/.config/agent-kit/env.sh`, which reads them back, and offers
   to add one line to your `~/.zshrc` or `~/.bashrc` that loads it.

A value already in the environment wins over the stored one. A later run
asks only for what is missing or refused by its host. Optional credentials
(below) never gate registration — the server self-manages auth.

| Variable | Used by | Required? |
|---|---|---|
| `HINDSIGHT_API_TOKEN` | the `memory-api` MCP server, and the `hindsight-memory` plugin's own hooks | required |
| `HINDSIGHT_API_URL` | the `hindsight-memory` plugin (`https://hindsight-api.jorisjonkers.dev`) | required |
| `MEMORY_MCP_TOKEN` | the `memory-mcp` MCP server (Basic Memory) | required |
| `MEMORY_PRIVATE_TOKEN` | the `memory-private` MCP server: always in Vibe, and in a `memory-private claude\|codex` session | never exported; sealed behind your passphrase, given to `vibe` and `memory-private` per launch |
| `MISTRAL_API_KEY` | Mistral Vibe itself; a dedicated per-host key, never the cluster's Hindsight or Karakeep keys | required for Vibe |
| `OVERLEAF_SESSION` | the `overleaf` MCP server (self-hosted Overleaf) | optional |

`HINDSIGHT_API_TOKEN` and `MEMORY_MCP_TOKEN` are **per-host, individually
revocable** service tokens, not shared secrets — mint one per device with
`POST /api/v1/auth/service-tokens {"service":"MEMORY_API"|"MEMORY_MCP",
"label":"<device>"}` while signed in to auth-api (agent-kit#41/#42 design),
and revoke a lost device with `DELETE /api/v1/auth/service-tokens/{id}`
without touching any other device or host. Never share one token between
hosts, and never fall back to the account password or a long-lived shared
key — see the [#41 design](https://github.com/JorisJonkers-dev/agent-kit/issues/41)
for why both are explicitly rejected.

`HINDSIGHT_API_URL` is not a secret, but an unset value is not loud: the
`hindsight-memory` plugin quietly falls back to a personal **local** daemon
instead of the shared estate bank, which looks identical to a working setup
from inside a session. The setup script warns by name when it is unset;
verify by checking which bank a note actually landed in, not by the plugin
starting without an error.

`hindsight-api.jorisjonkers.dev` serves only the shared bank `default`: its
single-bank MCP endpoint `/mcp/default/` and the REST paths under
`/v1/default/banks/default/`. Everything else on that host is a 404, including
the multi-bank `/mcp` root. The plugin's own default bank is `claude-code`, so
the setup script sets `bankId: default` in `~/.hindsight/claude-code.json`;
without it every recall and retain is a 404.

### Automatic vs. model-elective capture

| Client | Capture |
|---|---|
| Claude Code | **Automatic** — the `hindsight-memory` plugin's `UserPromptSubmit`/`Stop` hooks recall and retain on every turn, no tool call needed. |
| Codex, Hermes (laptop and in-cluster) | **Model-elective** — no hook mechanism exists, so recall/retain only happens when the model calls a `memory-api` or `memory-mcp` tool itself. |
| Mistral Vibe | **Model-elective** for now, through the same MCP servers. Vibe's hooks fire only around tools and after a turn (`pre_tool`, `post_tool`, `post_agent`), so automatic retain is possible but automatic recall at prompt start is not (fleet-infra#399). |

Both paths write into the same shared bank, `default` (the setup script pins
the plugin's `bankId`, and `hindsight-api.*` serves no other bank), and the
same shared Basic Memory vault. Basic
Memory carries no per-call identity of its own, so attribute a change in the
note body/frontmatter, and always **edit** a note — `--overwrite` is
last-writer-wins and silently drops a concurrent edit from another agent or
session (verified live).

`OVERLEAF_SESSION` is the `overleaf.sid` cookie from a logged-in browser
session on the **self-hosted** instance — not `overleaf_session2`, which is the
overleaf.com name and breaks auth against `overleaf.jorisjonkers.dev`
(verified: the wrong name makes `olcli whoami` report "Session invalid"). It is
optional, because `olcli auth` once stores the session itself; export it as an
override, or skip it. The self-hosted base URL is baked into the registry, so a
bare `olcli` with no `OVERLEAF_BASE_URL` talks to overleaf.com and every call
404s against a project that is not there.

### Keeping a directory out of the shared bank

The `hindsight-memory` plugin retains every Claude Code session into the
shared bank `default`, which Hermes and every agent can recall from. For a
directory whose sessions are private:

```bash
memory-private on [DIR]      # default: the current directory
memory-private status [DIR]
memory-private off [DIR]
```

`on` sets `HINDSIGHT_AUTO_RETAIN=false` in that directory's
`.claude/settings.local.json` (Claude Code's per-user settings), keeping every
other key. Claude Code git-ignores that file only when it creates it, so in a
repository where it is not ignored yet, `on` also adds it to `.git/info/exclude`. Claude Code hands that `env` block to the plugin's
hooks once the folder is trusted, and the plugin then skips both of its retain
paths. Recall is unaffected. For one session only:
`HINDSIGHT_AUTO_RETAIN=false claude`.

### The private bank

Personal notes go to a separate Hindsight bank, `private`, served only on
`hindsight-private.jorisjonkers.dev` and only to a `MEMORY_PRIVATE` service
token. Hermes, runners and cloud sessions never get it.

Vibe, the agent for personal notes, has the `memory-private` MCP server on,
next to the shared bank.

**The token is behind a passphrase only you know.** Setup mints it and seals
it with `agent-kit-private` (openssl AES-256-CBC, PBKDF2 600,000 rounds) in
`~/.config/agent-kit/private/`, a 0700 folder. The first time, you choose the
passphrase; it is not your login password, and nothing can recover it (setup
then mints a new token). After that:

- `vibe` (a function in env.sh) runs `agent-kit-private exec vibe`, which asks
  for the passphrase at the terminal and gives the token to that one launch.
- `memory-private claude|codex` goes through the same gate.
- Nothing else can open it. The file is ciphertext, the passphrase and token
  never reach argv, and without a terminal the gate asks nothing and gives
  nothing, so an agent's shell gets no prompt to answer.
- Claude Code also gets deny rules (`claude_private_deny` in the registry):
  its file tools and Bash may not touch `~/.vibe/**`, where Vibe keeps its
  session logs (the private conversations themselves), the sealed token, or
  `agent-kit-private`.

Known limits: Codex has no read deny-list, so it could read `~/.vibe` logs,
though not the token. Vibe started by an editor or the Vibe app, not through
the `vibe` function, has no private bank. A token left in the login keychain
by an earlier setup is removed on the next run and replaced.

Without the token, Vibe connects to the private bank with no credential and
is refused.

Vibe saves to the private bank only what is worth keeping, by tool:
`${VIBE_HOME}/AGENTS.md` asks it to keep facts about people and you, plans,
decisions and their reasons, and anything you ask it to remember, and to
recall from both banks at the start of a task. Nothing is saved every turn.

Claude Code and Codex have it installed switched off: Claude Code has it only
as a `--mcp-config` file, and Codex lists it as disabled. Open one session
with it on:

```bash
memory-private claude [ARGS]          # or: memory-private codex [ARGS]
```

That session also runs with `HINDSIGHT_AUTO_RETAIN=false` and with the shared
stores Hermes reads switched off (`memory-api` and `memory-mcp`), so nothing
from it can land where Hermes can see it. Every other session, including another one
open at the same time, still has no private bank.

### Per-host service tokens (memory-api / memory-mcp)

auth-api (agent-kit#41) now issues per-user, per-host bearer credentials —
"service tokens" — for CLI/agent access to `hindsight-api.jorisjonkers.dev`
(Hindsight) and `memory-mcp.jorisjonkers.dev` (Basic Memory MCP), both behind
forward-auth. This is the contract `setup-workstation.sh` will consume once a
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
   # repeat with "service":"MEMORY_MCP" for the second host
   ```

   Minting requires the caller already hold the `MEMORY_API` / `MEMORY_MCP`
   grant (`user_service_permissions`) — an admin assigns that first, the same
   way any other service grant is assigned. The raw token is returned exactly
   once, as `smt_...`; there is no way to retrieve it again.
2. Send it as `Authorization: Bearer <token>` directly to the host. A token
   minted for one host is rejected by the other, and by every other
   forward-auth host in the estate: `/verify` checks the token's single
   `SERVICE_MEMORY_API` or `SERVICE_MEMORY_MCP` claim against that host's own
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

**Not done in this change:** registering `memory-api` / `memory-mcp` as
`mcp_servers:` entries in `registry/estate-tooling.yaml`. The registry
already carries a `memory` entry (`enabled: false`) pointed at mem0 per
[MEMORY.md](MEMORY.md)'s bring-up plan, which predates the Hindsight / Basic
Memory platform this ticket's tokens are for. Reconciling those two — is
mem0 still the plan, or has it been superseded — is a decision for whoever
owns that bring-up, not something to guess into the generated registry
here.

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
