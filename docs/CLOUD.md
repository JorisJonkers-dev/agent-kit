# Claude Code cloud environments

A cloud environment on [claude.ai/code](https://claude.ai/code) gets the same
agent baseline as a laptop: Codex, Hermes, the plugins, the language servers
the estate uses, the MCP fleet and the memory platform. Two files in
[`installer/cloud/`](../installer/cloud) are all you paste; everything else is fetched from the
latest GitHub release at setup time, so the kit keeps changing here without
editing the environment again.

| File | Goes into the environment's |
|---|---|
| [`installer/cloud/setup-script.sh`](../installer/cloud/setup-script.sh) | **Setup script** field |
| [`installer/cloud/environment.env`](../installer/cloud/environment.env) | **Environment variables** field, values filled in |

The setup script downloads the latest release's `setup-workstation.sh` from
`https://github.com/JorisJonkers-dev/agent-kit/releases/latest/download/setup-workstation.sh` and runs it with `--cloud`. That copy is not in a checkout, so it
fetches the kit bundle attached to the same release
(`releases/download/v<version>/agent-kit-skills.tar.gz`), checks its sha256,
and re-runs the copy inside it.

## Set it up

1. **Mint a token for the environment**, as for any other host
   ([SETUP.md](SETUP.md#per-host-service-tokens-memory-api--memory-private)):
   service `MEMORY_API`, labelled `claude-cloud`. Revoking it later touches
   no other device.
2. **Create the environment.** Network access **Custom**, with the Trusted
   defaults included, plus:

   ```text
   *.jorisjonkers.dev
   release-assets.githubusercontent.com
   download-cdn.jetbrains.com
   openrouter.ai
   api.openai.com
   cdn.playwright.dev
   playwright.download.prss.microsoft.com
   ```

   `*.jorisjonkers.dev` carries both memory hosts and Overleaf. GitHub
   serves the kit: `github.com` is a Trusted default, and release downloads
   redirect to `release-assets.githubusercontent.com`, listed in case the
   defaults do not cover it; without it the setup script cannot even fetch
   the kit. The
   JetBrains CDN serves the Kotlin language server, the next two are Hermes'
   and Codex' model APIs, and the last two serve Playwright's browser on first
   use. **Full** access works too.
3. **Paste the two files** and fill in the values.
4. **Start a session and check it** (below).

## What `--cloud` changes

| | Laptop | Cloud |
|---|---|---|
| Claude Code | installed and updated | the host's own; a config-only CLI is installed under `/opt/agent-kit/claude-cli` if none is on `PATH` during setup |
| Claude profiles | `work` and `personal` | one |
| MCP tokens in `~/.claude.json` | the value | a `${VAR}` reference, expanded from the session's environment |
| `kubernetes` MCP server | port-forwarded | skipped: no kube context |
| `github` plugin | installed | skipped: the host's GitHub proxy and tools cover it |
| Language servers | all | TypeScript, Python, Kotlin, Java |
| Codex | your login | logged in with `OPENAI_API_KEY` |
| Hermes | your config | model set to OpenRouter if none is configured |

An entry with `cloud: false` in the registry is what keeps it out
([REGISTRY.md](REGISTRY.md#keeping-an-entry-off-the-cloud)).

Tokens stay references because the environment caches a disk snapshot of the
setup run and reuses it for about a week. A reference means the snapshot holds
no memory token, and a rotated token only needs the variable changed. Codex is
the exception: its login lands in `~/.codex/auth.json`, inside the snapshot.

## Check the first session

The setup output is kept at `/var/log/agent-kit-setup.log`. Ask Claude to run:

```bash
tail -30 /var/log/agent-kit-setup.log   # the summary and every FAIL
claude mcp list                         # memory-api, playwright, drawio, overleaf
claude mcp list | grep hindsight        # Hindsight Coding Agents' memory tools
```

If `claude` is not on the session's `PATH`, use the copy setup installed:
`/opt/agent-kit/claude-cli/bin/claude`. The log's first cloud lines say which
one setup used and which environment variables it could see; the session sees
them all, whatever that line says.

A server in `claude mcp list` with a missing-variable warning means that
variable is not set in the environment. A server that is **absent** means the
session's Claude Code reads a different home than setup wrote to; the log's
`cloud: running as ... HOME=...` line says which one setup used.

Memory is the same bank as the laptop's: Hindsight Coding Agents is pinned to
the static bank `default`, not one per repository, so the cloud checkout's
different path does not split it.

## Getting a change onto the cloud

1. Merge the change here. A release attaches `setup-workstation.sh` and the
   kit bundle to its GitHub release, and `releases/latest` serves them as soon
   as the publish workflow finishes.
2. The environment re-runs its setup script only when the script text or the
   network settings change, or after about a week. Bump the `refreshed:` date
   in the pasted script to pick the release up now.
