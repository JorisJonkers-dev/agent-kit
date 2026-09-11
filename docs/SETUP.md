# Workstation setup

One command brings a machine to the estate's agent baseline: Claude Code and
Codex at their latest releases, every plugin, the language servers those
plugins drive, and the MCP fleet.

```bash
uv run python scripts/render_registry.py --check   # artifacts current?
./installer/setup-workstation.sh --check           # what would change
./installer/setup-workstation.sh                   # do it
```

`setup-workstation.sh` is **generated** from
[`registry/estate-tooling.yaml`](../registry/estate-tooling.yaml). Do not edit
it. Edit the registry and re-render — see [REGISTRY.md](REGISTRY.md).

## Credentials

The script never writes a secret. Export what you need first; anything that is
a **required** credential is skipped (a warning, not a failed run) when absent.
Optional credentials (below) never gate registration — the server self-manages
auth.

| Variable | Used by | Required? |
|---|---|---|
| `MEMORY_API_KEY` | the `memory` MCP server | required |
| `OVERLEAF_SESSION` | the `overleaf` MCP server (self-hosted Overleaf) | optional |

`OVERLEAF_SESSION` is the `overleaf.sid` cookie from a logged-in browser
session on the **self-hosted** instance — not `overleaf_session2`, which is the
overleaf.com name and breaks auth against `overleaf.jorisjonkers.dev`
(verified: the wrong name makes `olcli whoami` report "Session invalid"). It is
optional, because `olcli auth` once stores the session itself; export it as an
override, or skip it. The self-hosted base URL is baked into the registry, so a
bare `olcli` with no `OVERLEAF_BASE_URL` talks to overleaf.com and every call
404s against a project that is not there.

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

`setup-workstation.sh` only *reports* a machine that still has them wired.
`install-agents.sh` does the purge: it strips hook groups from
`settings.json` whose command basename is one of the retired scripts, deletes
the leftover script files and `~/.codex/hooks.json`, then reads the file back
and fails if any retired name survives. Your own hooks are matched by basename
and left alone.

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
