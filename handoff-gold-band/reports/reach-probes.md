# Reach-level probes run by the orchestrator, 2026-11-06 (verified on this machine unless marked)

## Pi (installed today)

- Installed globally with `npm install -g --ignore-scripts --before=2026-10-30T15:00:00Z @earendil-works/pi-coding-agent@1.4.0 pi-acp@0.0.34`. `--before` applies to all 130 packages (transitive too), so nothing younger than 7 days was resolved. Pi 1.5.0 (latest) was published 2026-11-02, 4 days before install, so 1.4.0 (2026-10-21) was used instead. `pi-acp` 0.0.34 is 42 days old. Binaries: `~/.nvm/versions/node/v22.21.0/bin/pi` and `pi-acp` (nvm path; the launcher needs an absolute path that survives nvm switches).
- `pi --help` shows: `--tools <list>` (allowlist, patterns allowed; example `--tools read,grep,find,ls`), `--exclude-tools <list>` (denylist, includes MCP tools), `--no-tools`, `--no-builtin-tools`, `--no-mcp`, `--no-extensions`, `--no-skills`, `--no-context-files`, `--skill <path>`, `--offline`. Default tools: read, bash, edit, write.
- `pi-acp` starts Pi as `<cmd> --mode rpc --no-themes [--session ...]` with `env: process.env`. The command comes from env `PI_ACP_PI_COMMAND` (pi-acp dist/index.js:129-140, 1949). So a wrapper script named in `PI_ACP_PI_COMMAND` can exec `pi --tools ... --no-mcp --no-extensions "$@"`. This makes `read` (and `write`) reachable by tool configuration, which contradicts "Pi has no permission prompts, so nothing can be restricted".
- Not verified: the exact built-in tool names beyond the help example; whether Pi has any web search or fetch tool (default tools show none, so `read-web` and `research` would need an extension or MCP server, which `--no-extensions` and `--no-mcp` remove); path-scoped writes (OS layer needed). No model call was made.
- `PI_CODING_AGENT_DIR` redirects Pi's config home (seen in pi-acp source).

## OpenCode

- `opencode debug agents` prints each agent's resolved permissions as a list of `{action, resource, effect}` rules with effects allow / ask / deny. Actions seen: `*`, `read`, `shell`, `grep`, `glob`, `webfetch`, `websearch`, `subagent`, `external_directory`, `question`, `browser`, `skill`. The built-in `explore` agent is `* deny` followed by allows for shell, grep, glob, webfetch, websearch, read. So deny-by-default with per-action allows is native.
- Three shapes of `OPENCODE_CONFIG_CONTENT` (`permission` object, `permissions` array, `agent.build.permission`) did not change what `debug agents` printed, so the config key that sets these rules is still unknown. `debug config` only lists config sources. Status stays UNVERIFIED.

## Not run

Hermes and Antigravity probes (need running the agent; Hermes shows no launch-time toolset switch, Antigravity needs `agy --print`).

## Hermes (probed, no model call)

- `hermes tools list` shows toolsets: web (search and scraping), browser, terminal, file (read and write together), code_execution, vision, delegation, memory, skills, computer_use and more. `hermes tools enable/disable --platform X` accepts only the listed messaging and cli platforms; `acp` is rejected ("Unknown platform 'acp'").
- In ACP mode the session is built with `enabled_toolsets = _expand_acp_enabled_toolsets(["hermes-acp"], mcp servers)`: the list is hard-coded in `acp_adapter/session.py` (~line 625) and is not read from `platform_toolsets` in config. The `hermes-acp` toolset holds web_search, web_extract, terminal, process, read_file, write_file, patch, search_files, browser_*, skills, todo, memory and more (`toolsets.py:380`).
- Result: tool configuration cannot restrict Hermes in ACP mode. `read`, `read-web` and `research` are NOT POSSIBLE as defined (shell and web cannot be removed). `write-scoped` is possible only as an operating-system write confinement, with the shell still present. `write` is the default. Config home redirect: `HERMES_HOME` (verified earlier). Only MCP servers can be omitted (disabled entries in `mcp_servers`).

## Antigravity (probed, no model call)

- `agy acp` flags: `--sandbox` (terminal restrictions), `--include-default-customizations` (default true), `--agent`, `--add-dir`, `--app-data-dir`, `--cli_dir_name`, `--gemini-dir`, `--dangerously-skip-permissions`, `--disable-telemetry`, `--process-per-session`, `--login`. No tool allowlist flag. `agy agents` lists no custom agents. `~/.gemini/settings.json` holds only an IDE nudge flag and the auth type.
- The settings key names for tool gating are not shown anywhere locally; `--settings` exists only on the root command, not on `acp`. So `read`, `read-web` and `research` stay UNVERIFIED; `write-scoped` can use `--sandbox` plus the OS layer; `write` is the default. An earlier worker's claim of permission-rule grammar and hooks came from docs, not from these probes.

## agents.json changes (2026-11-06)

`pi-local` now uses the absolute `pi-acp` path. New `pi-read` runs the same adapter with `PI_ACP_PI_COMMAND=/Users/j.w.jonkers/.local/bin/pi-read`; the wrapper `config/bin/pi-read` execs Pi with `--tools read,grep,find,ls --no-mcp --no-extensions --no-skills --no-context-files --offline` (`bash`/sh syntax checked; it prints 1.4.0 for `--version`; tool names unverified; not installed to `~/.local/bin`).
