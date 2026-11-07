I'll start by locating the relevant files with grep.No grep tool exists in this environment and bash is denied; `read_file` can list directories, so I'll use it to navigate. Let me locate the repo root.Now let me list the generated directories and read the MCP artifacts.Now manifest.yaml and docs/SETUP.md. These may be large; I'll read fully.Now docs/SETUP.md and the merge scripts.Now the scripts.I have everything I need; let me pin exact line numbers for key claims in the registry and docs.## 1) SUMMARY

The fleet is 12 servers declared in `registry/estate-tooling.yaml:1213-1473`, plus two plugin-owned MCP servers (Claude's `github` plugin; Hindsight Coding Agents' `hindsight`). Every live workstation server is already converged into each agent's own config — Claude (both profiles), Vibe, OpenCode, Antigravity, local Hermes, Codex — and Codex is excluded from Gold Band anyway. Since Gold Band injects `contextServers` into every capable ACP session and upstream has no dedupe, adding any current-fleet server would double-register its tools. Recommendation: keep `contextServers` empty; the fleet stays per-agent.

## 2) RECOMMENDATION

**Artefact 1 — `~/.gold-band/settings.json`, `contextServers` array (primary recommendation):**

```json
"contextServers": []
```

Rationale: every live server is already in each configured agent's own config (`docs/SETUP.md:26`, "each … ends a run with exactly the MCP servers the registry declares"; `:389`, "Every `workstation` MCP server is registered into Claude Code, Codex and local Hermes"); Vibe/OpenCode/Antigravity/Hermes fragments confirm it (`registry/generated/{vibe,opencode,antigravity,hermes}/...`). Declaring them in Gold Band too would register each server twice per session (duplicate-tool risk, ref §MCP servers: injection is per session by agent capability; no dedupe).

**Artefact 2 — only if you decide pi needs tools (it has no agent-kit-configured home):** add the no-secret stdio servers and a token-free routes only, e.g.:

```json
"contextServers": [
  { "n8n-mcp":    { "command": "n8n-mcp",    "args": [] } },
  { "playwright": { "command": "playwright-mcp", "args": ["--headless", "--browser", "chromium"] } },
  { "drawio":     { "command": "drawio-mcp", "args": [] } }
]
```

Accept that this duplicates those tools for every other Gold Band agent (pi is the sole beneficiary). Exact record shape (array vs map, per-server keys): verify in diodeme/Gold-Band (src/config/mod.rs).

**Artefact 3 — if pi needs `memory-api`, use the existing stdio proxy so no secret lands in the file** (pattern already in the estate, `docs/SETUP.md:545`; proxy reads the keychain at startup, token never on disk/argv):

```json
{ "memory-api": { "command": "/bin/sh", "args": ["-c",
  "exec \"$HOME/.local/bin/antigravity-mcp-proxy\" --url 'https://hindsight-api.jorisjonkers.dev/mcp/default/' --secret HINDSIGHT_API_TOKEN"] } }
```

**Duplicate-tool rule:** a server may be declared in exactly one layer — either Gold Band's `contextServers` or the agent's own config, never both. Since `setup-workstation.sh` converges the agents' own configs from the registry (`docs/SETUP.md:641-643`), a server moves to Gold Band only by removing it from the registry's workstation surfaces in the same change; absent that, `contextServers` stays empty.

**Secrets at runtime, never in the file:** all fleet tokens live in the macOS keychain as `agent-kit.<VAR>` and are read into every shell by `~/.config/agent-kit/env.sh` (`docs/SETUP.md:56-71`); Vibe reads them by `api_key_env` (`registry/generated/vibe/mcp-servers.toml:5-15`), OpenCode via `Bearer {env:VAR}` (`registry/generated/opencode/opencode.json`), local Hermes via `${VAR}` (`registry/generated/hermes/mcp-servers.local.yaml`). Gold Band records quote env-var *names* or a proxy command, never values. Whether Gold Band expands vars at launch is unverified — verify in diodeme/Gold-Band (src/config/mod.rs).

**Per-server inventory and verdicts** (credential: kw=keychain+env.sh, —=none):

| Server | Transport | Endpoint | Credential | Needs it | Verdict |
|---|---|---|---|---|---|
| memory-api | http | hindsight-api…/mcp/default/ | `HINDSIGHT_API_TOKEN` bearer, kw (`registry/estate-tooling.yaml:1214`) | all | per agent only |
| memory-private | http | hindsight-private…/mcp/private/ | passphrase-sealed, `export: false`, `enabled_for: [vibe-private]` (`:1231`, `:92-95`) | vibe-private only | **DO NOT add**: vibe-private is not a Gold Band agent; any record would expose the token path and plaintext copy |
| memory-mcp | http | — | retired | nobody (Basic Memory at zero replicas, `:1250`) | **DO NOT add**: retired, in-cluster only |
| n8n | http | n8n…/mcp-server/http | per-client OAuth, no stored secret (`:1267`) | Claude×2, Codex, Vibe, OpenCode, Antigravity, Hermes local | per agent only; Gold Band `oauth{clientId, clientSecret}` would be plaintext |
| n8n-mcp | stdio | `n8n-mcp`, docs-only (`:1279`) | — | same | per agent only |
| kubernetes | http | `http://127.0.0.1:18080/mcp` via launchd port-forward (`:1295`–1306) | — | same | per agent only; svc.cluster.local URL is in-cluster only |
| github | http | api.githubcopilot.com/mcp/ | `GITHUB_PAT`; surfaces hermes/runner only (`:1316`, `:1323`) | in-cluster; Claude uses its `github` plugin, plugin-managed credential (`docs/SETUP.md:424`) | **DO NOT add**: secret plaintext; workstation deliberately uses the plugin |
| grimoire | http | grimoire…/mcp | `GRIMOIRE_API_TOKEN` bearer, kw (`:1334`) | personal hobby agents | per agent only |
| outline | http | notes…/mcp | `OUTLINE_API_KEY`, kw (`:1354`) | all | per agent only |
| vuetify | http | mcp.vuetifyjs.com/mcp | — (`:1371`) | disabled | **DO NOT add**: `enabled: false` |
| playwright | stdio | `playwright-mcp --headless --browser chromium` (`:1381`) | — | all | per agent only |
| drawio | stdio | `drawio-mcp` (`:1410`) | — | all | per agent only |
| overleaf | stdio | `olcli-mcp` + env `OVERLEAF_BASE_URL`, `OVERLEAF_COOKIE_NAME`, `OLCLI_MCP_UNCONFIRMED_WRITES`; optional `OVERLEAF_SESSION` (`:1436-1473`) | olcli's own session | all | per agent only |
| hindsight (plugin) | per agent | pinned npx install wires hooks + server into Claude×2, Codex (`registry/estate-tooling.yaml:624-627`; `docs/SETUP.md:128`) | `HINDSIGHT_API_TOKEN` | Claude×2, Codex | per agent only; auto-recall hooks are agent-internal (ref §Overview) |

Gold Band's own `--gold-band-memory-mcp` built-in is not part of this fleet and needs no record.

## 3) FINDINGS

1. 12 servers + 2 plugin-owned MCP servers form the fleet | `registry/estate-tooling.yaml:1213-1473`; `docs/SETUP.md:128,424` | inventory complete | H
2. Every live server is already converged per agent (Claude×2, Codex, Vibe, OpenCode, Antigravity, Hermes local) | `docs/SETUP.md:26,389`; `registry/generated/*` | any Gold Band duplicate double-registers tools; no upstream dedupe | M (per reference; test 1)
3. Injection is per session by agent capability, global across agents | ref §MCP servers | `contextServers` cannot be scoped to pi only | H
4. http records carry `headers`/`oauth{clientSecret}` in plaintext settings.json | ref §MCP servers, §Storage | tokens may never go in records; use env names or the stdio proxy | H
5. memory-private is passphrase-gated and unexported, vibe-private only | `registry/estate-tooling.yaml:92,1231-1242` | excluded from Gold Band entirely | H
6. `MISTRAL_STUDY_API_KEY` is unexported, read only by `vibe-study` | `registry/estate-tooling.yaml:60-66` | vibe-study custom agent must rely on its launcher env, not Gold Band records for it — but launch command itself supplies it | M
7. Codex excluded from Gold Band per owner rules; Codex's MCP config keeps its own servers anyway | `registry/generated/...`; `scripts/codex-add-url-mcp.py` | no conflict | H
8. Exact `contextServers` JSON shape (array vs map, key names) not visible in the reference | labelled unverified | paste-ready JSON may need key renames; verify in diodeme/Gold-Band (src/config/mod.rs) | L

## 4) VERIFY LIST

1. No dedupe between agent-own and injected servers: register `playwright` in `contextServers`, run DIRECT chat with claude-acp, call `tools/list` — count playwright tools; >1 set confirms duplication.
2. `settings.json` stores `headers`/`oauth.clientSecret`/stdio `env` in plaintext: add a test http server with a dummy header, open the file.
3. Whether a GUI-launched Gold Band inherits `env.sh` (`HINDSIGHT_API_TOKEN` etc.): stdio server that logs its environment; check on launch.
4. Whether stdio records and agent-adapter `env` expand variables or store literals; whether `contextServers` is an array or object — verify in diodeme/Gold-Band (src/config/mod.rs).

## 5) DECISIONS FOR THE OWNER

1. Empty `contextServers` (recommended) or per-server for pi? Recommended: empty now; add Artefact 2 only if pi does real work.
2. pi's purpose in Gold Band? Recommended: wipe/run tool agent; give it docs-only servers (Artefact 2), not memory.
3. Keep `kubernetes` loopback server reachable — Gold Band sessions reach it only while the launchd port-forward is up. Recommended: leave per agent only; Gold Band inherits the same URL if ever added.
4. If you later want Gold Band as the MCP injection layer: remove the server from `workstation` surfaces in the registry and add it to `contextServers` in the same change (one layer, one server). Recommended: not now.
5. LiteLLM gateway later: do not put proxy API keys in settings.json either — same plaintext rule. Recommended: keys via environment, keys never in records.
6. Skills: Gold Band symlinks global `~/.gold-band/skills/` into agents' own dirs (ref §Skills); its built-in profiles are read-only. Recommended decision: skip duplicating skills; see skills question in the roles output. — (If you want a skills decision here: recommended answer: govern skills via Gold Band symlink only per-agent where it wins, none globally.)

SUMMARY: Gold Band config for the MCP fleet: recommend `~/.gold-band/settings.json` `"contextServers": []`. The estate's 12-server fleet (memory-api, memory-private, memory-mcp retired, n8n, n8n-mcp, kubernetes, github, grimoire, outline, vuetify disabled, playwright, drawio, overleaf) plus two plugin-owned servers (Claude's github plugin, Hindsight Coding Agents' hindsight) is already converged into every configured agent's own config by `setup-workstation.sh`: Claude registers each workstation server once per profile in `.claude.json` (profiles must not share it, `registry/estate-tooling.yaml` claude_profiles note), Vibe via `scripts/vibe-merge-mcp.py` into both homes (vibe-private keeps memory-private, the only holder), OpenCode via `scripts/opencode-merge-mcp.py` with `{env:VAR}` headers, Antigravity via `scripts/antigravity-merge-mcp.py` with a keychain-reading stdio proxy for token-bearing servers, local Hermes via `scripts/hermes-merge-mcp.py`, Codex via `scripts/codex-add-url-mcp.py` (excluded from Gold Band anyway). Gold Band injects `contextServers` into every capable ACP session and has no dedupe, so any duplicate declaration double-registers tools per session. Rule: one server, one layer — a server lives in Gold Band's contextServers XOR an agent's own config, never both; if it moves to Gold Band, remove it from the registry's workstation surfaces in the same change. DO NOT add: memory-private (passphrase-sealed, unexported, vibe-private only), memory-mcp (retired, zero replicas), github (GITHUB_PAT plaintext + plugin-managed on Claude), vuetify (disabled), any token-bearing http server as a headers/oauth record (settings.json is plaintext; the http oauth record carries clientId/clientSecret). Per agent only: memory-api, n8n (per-client OAuth sign-in), n8n-mcp, kubernetes (loopback 127.0.0.1:18080 via launchd port-forward; the svc.cluster.local URL is in-cluster only), grimoire, outline, playwright, drawio, overleaf (olcli self-managed session), hindsight plugin. Runtime secrets never enter the file: tokens sit in the macOS keychain as `agent-kit.<VAR>` and are exported by `~/.config/agent-kit/env.sh` in every shell; agents read them by env name (Vibe `api_key_env`, OpenCode `{env:VAR}`, Hermes `${VAR}`); Gold Band should name env vars or use the existing antigravity-mcp-proxy stdio command pattern so tokens stay off disk. Remaining gaps to verify in diodeme/Gold-Band (src/config/mod.rs) or by test: exact contextServers JSON shape, that headers/env are stored literally (confirming plaintext), that a GUI launch inherits env.sh, and a tools/list duplicate test with playwright registered in both layers.